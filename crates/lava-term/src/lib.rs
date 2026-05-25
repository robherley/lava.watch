//! Shared terminal-transport plumbing for the SSH and telnet servers.
//!
//! Both transports do the same thing once a client is connected: own a
//! [`lava_engine::Session`], push ANSI frames on a fixed cadence, and drain
//! inbound key/resize events until the client leaves or a deadline fires. Only
//! the byte sink differs — an SSH channel vs. a raw TCP socket — so that's
//! abstracted behind [`FrameSink`] and everything else lives here: the
//! frame-loop driver ([`run_session`]), the timing/size constants, the per-IP
//! [`ConnTracker`], and the canned client messages.

mod tracker;

pub use tracker::{ConnSlot, ConnTracker};

use lava_engine::{term, Session};
use std::future::Future;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Frame cadence — ~30fps.
pub const FRAME_PERIOD: Duration = Duration::from_millis(33);
/// Upper bounds on a client-reported terminal size. Generous, but bounded so a
/// hostile or buggy client can't make us allocate enormous frame buffers.
pub const MAX_COLS: u16 = 512;
pub const MAX_ROWS: u16 = 256;
/// Cap on a single frame's wall-clock delta, so a long pause (suspend,
/// debugger, a slow network tick) can't lurch the simulation forward by
/// seconds on catch-up.
const MAX_DT: f32 = 0.25;

/// Sent to a client turned away because its IP is at the connection cap.
pub const BUSY_MESSAGE: &[u8] = b"\r\ntoo many connections, try again shortly.\r\n";
/// Sent just before disconnecting a session that hit its time limit.
const TIMEOUT_MESSAGE: &[u8] = b"\r\n\x1b[0m*** session timed out ***\r\n";

/// Clamp a client-reported terminal size into the supported range.
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(1, MAX_COLS), rows.clamp(1, MAX_ROWS))
}

/// Out-of-band events a transport feeds into a running session.
pub enum SessionMsg {
    /// Terminal input bytes. Transports pre-filter to known control sequences
    /// (see [`lava_engine::parse_input`]) before sending these.
    Input(Vec<u8>),
    Resize(u16, u16),
    /// Bytes to write straight back to the client, bypassing the renderer —
    /// e.g. telnet option replies. Transports that never need it don't send it.
    Raw(Vec<u8>),
}

/// Why [`run_session`] returned. Transports log the [`EndReason::as_str`] form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    ClientExit,
    Timeout,
    Disconnect,
    WriteFailed,
}

impl EndReason {
    pub fn as_str(self) -> &'static str {
        match self {
            EndReason::ClientExit => "client_exit",
            EndReason::Timeout => "timeout",
            EndReason::Disconnect => "disconnect",
            EndReason::WriteFailed => "write_failed",
        }
    }
}

/// A bytes-out channel to a connected client. Implemented by each transport
/// over its native write path (an SSH channel handle, a TCP socket, …).
pub trait FrameSink {
    /// Write all of `bytes` to the client. Returns `false` if the client is
    /// gone — the session then ends as a disconnect.
    fn write(&mut self, bytes: &[u8]) -> impl Future<Output = bool> + Send;

    /// Tear down the underlying transport. Called exactly once, after the
    /// frame loop ends and the screen has been restored.
    fn shutdown(&mut self) -> impl Future<Output = ()> + Send;
}

/// Drive a session to completion: write the terminal-setup prelude, then loop
/// — render a frame every [`FRAME_PERIOD`], apply inbound [`SessionMsg`]s, and
/// stop on client exit, disconnect, or `max_time`. Restores the client's
/// screen and shuts the sink down on the way out.
///
/// The caller owns connection setup outside the loop (slot acquisition, input
/// plumbing, logging) and the [`Session`]'s initial size/palette/speed.
/// `prelude` is transport-specific bytes to emit before the shared alt-screen
/// + mouse-enable setup (telnet negotiation; empty for SSH).
pub async fn run_session<S: FrameSink>(
    mut sink: S,
    mut session: Session,
    mut msgs: mpsc::Receiver<SessionMsg>,
    max_time: Duration,
    prelude: &[u8],
) -> EndReason {
    // Enter the alt-screen and enable mouse reporting, prefixed by whatever the
    // transport needs to send first.
    let mut intro = Vec::with_capacity(prelude.len() + 32);
    intro.extend_from_slice(prelude);
    intro.extend_from_slice(term::ENTER_ALT_SCREEN);
    intro.extend_from_slice(term::MOUSE_ENABLE);
    if !sink.write(&intro).await {
        return EndReason::WriteFailed;
    }

    let (cols, rows) = session.dimensions();
    let mut buf: Vec<u8> = Vec::with_capacity(cols as usize * rows as usize * 24);
    let mut interval = tokio::time::interval(FRAME_PERIOD);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let deadline = tokio::time::sleep(max_time);
    tokio::pin!(deadline);

    // Wall-clock dt so a slow tick doesn't make the sim fall behind real time
    // and stutter on catch-up.
    let mut last_tick = Instant::now();

    let reason = loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = Instant::now();
                let dt = (now - last_tick).as_secs_f32().min(MAX_DT);
                last_tick = now;
                session.tick(dt);
                buf.clear();
                session.render(&mut buf);
                if !sink.write(&buf).await {
                    break EndReason::Disconnect;
                }
            }
            msg = msgs.recv() => match msg {
                Some(SessionMsg::Input(bytes)) => {
                    if session.feed_input(&bytes) {
                        break EndReason::ClientExit;
                    }
                }
                Some(SessionMsg::Resize(c, r)) => {
                    let (c, r) = clamp_size(c, r);
                    session.resize(c, r);
                    buf.reserve(c as usize * r as usize * 24);
                }
                Some(SessionMsg::Raw(bytes)) => {
                    if !sink.write(&bytes).await {
                        break EndReason::Disconnect;
                    }
                }
                // All senders dropped — the transport's reader is gone.
                None => break EndReason::Disconnect,
            },
            _ = &mut deadline => break EndReason::Timeout,
        }
    };

    if reason == EndReason::Timeout {
        let _ = sink.write(TIMEOUT_MESSAGE).await;
    }
    let mut outro = Vec::with_capacity(32);
    outro.extend_from_slice(term::MOUSE_DISABLE);
    outro.extend_from_slice(term::LEAVE_ALT_SCREEN);
    let _ = sink.write(&outro).await;
    sink.shutdown().await;

    reason
}
