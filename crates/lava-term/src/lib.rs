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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// Upper bounds on a client-reported terminal size. Bounded fairly tightly:
/// every frame is (at most) a full repaint of `cols × rows` cells, so an
/// oversized terminal is the dominant driver of per-connection bandwidth.
/// Larger terminals render at this cap (the lamp letterboxes) rather than
/// ballooning the byte stream.
pub const MAX_COLS: u16 = 200;
pub const MAX_ROWS: u16 = 60;
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

/// A fresh RNG seed for a new session, so each connection starts from a
/// different blob layout. Gathers native entropy — wall-clock nanos plus a
/// process-wide counter, so two connections in the same instant still differ —
/// and runs it through the engine's shared [`lava_engine::seed_from`] mixer.
pub fn random_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    lava_engine::seed_from(nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15))
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
/// — render a frame every `frame_period`, apply inbound [`SessionMsg`]s, and
/// stop on client exit, disconnect, or `max_time`. Restores the client's
/// screen and shuts the sink down on the way out.
///
/// Delta rendering is enabled here (only changed cells are sent each frame),
/// and color quantization is set from `quantize` — this is the
/// bandwidth-sensitive streaming path. The caller owns connection setup
/// outside the loop (slot acquisition, input plumbing, logging) and the
/// [`Session`]'s initial size/palette/speed. `prelude` is transport-specific
/// bytes to emit before the shared alt-screen + mouse-enable setup (telnet
/// negotiation; empty for SSH).
pub async fn run_session<S: FrameSink>(
    mut sink: S,
    mut session: Session,
    mut msgs: mpsc::Receiver<SessionMsg>,
    frame_period: Duration,
    max_time: Duration,
    quantize: bool,
    prelude: &[u8],
) -> EndReason {
    session.enable_delta();
    session.set_quantize(quantize);

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
    let mut interval = tokio::time::interval(frame_period);
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
