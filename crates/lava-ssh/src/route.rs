//! Username-based routing — every shell request lands here, gets matched
//! to a [`Route`], and dispatches to a handler that runs in a spawned task.
//!
//! Two routes today: a static help doc, and the live lava lamp session. The
//! lamp's frame loop lives in [`lava_term::run_session`]; this module just
//! adapts a russh channel into a [`FrameSink`] and handles SSH-specific setup
//! and logging around it.

use lava_engine::{help_text, Palette, Session};
use lava_term::{FrameSink, SessionMsg};
use russh::server::Handle;
use russh::ChannelId;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::info;

/// Resolved per-username target. Constructed by [`Route::from_username`] and
/// dispatched by the SSH handler.
pub(crate) enum Route {
    /// `ssh help@…` — print the help text and disconnect.
    Help,
    /// Anything else — run the lava lamp with the parsed palette (or the
    /// default if the username doesn't match a known palette name).
    Lava(Palette),
}

impl Route {
    pub(crate) fn from_username(username: Option<&str>) -> Self {
        match username {
            Some("help") => Self::Help,
            user => Self::Lava(user.map(Session::palette_from_str).unwrap_or_default()),
        }
    }
}

/// Everything `serve_lava` needs to run the frame loop. Built once in the
/// handler's `shell_request` after slot acquisition.
pub(crate) struct LavaParams {
    pub peer: Option<SocketAddr>,
    pub palette: Palette,
    pub channel: ChannelId,
    pub cols: u16,
    pub rows: u16,
    pub max_time: Duration,
    pub speed: f32,
}

/// Help route — write the palette doc, close the channel, done.
pub(crate) async fn serve_help(handle: Handle, channel: ChannelId) {
    let bytes = help_text(
        "lava — pick a palette by SSH username:",
        "ssh uv@lava.watch",
    );
    let _ = handle.data(channel, bytes).await;
    let _ = handle.close(channel).await;
}

/// SSH byte sink — writes frames to a russh channel and closes it on shutdown.
struct SshSink {
    handle: Handle,
    channel: ChannelId,
}

impl FrameSink for SshSink {
    async fn write(&mut self, bytes: &[u8]) -> bool {
        self.handle.data(self.channel, bytes.to_vec()).await.is_ok()
    }

    async fn shutdown(&mut self) {
        let _ = self.handle.close(self.channel).await;
    }
}

/// Lava route — build the session, hand the frame loop to [`lava_term`], and
/// log the outcome. The matching `session start` log is emitted by the handler
/// before this task is spawned.
pub(crate) async fn serve_lava(
    params: LavaParams,
    handle: Handle,
    msg_rx: mpsc::Receiver<SessionMsg>,
) {
    let LavaParams {
        peer,
        palette,
        channel,
        cols,
        rows,
        max_time,
        speed,
    } = params;
    let peer = peer.map(|p| p.to_string()).unwrap_or_default();
    let started = Instant::now();

    let mut session = Session::with_seed(cols, rows, palette, lava_term::random_seed());
    session.set_speed(speed);

    let sink = SshSink { handle, channel };
    let reason = lava_term::run_session(sink, session, msg_rx, max_time, &[]).await;

    info!(
        peer,
        duration_secs = started.elapsed().as_secs(),
        reason = reason.as_str(),
        "session end"
    );
}
