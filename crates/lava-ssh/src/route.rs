//! Username-based routing — every shell request lands here, gets matched
//! to a [`Route`], and dispatches to a handler that runs in a spawned task.
//!
//! Two routes today: a static help doc, and the live lava lamp session.

use lava_engine::{term, Palette, Session};
use russh::server::Handle;
use russh::ChannelId;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::info;

const FRAME_PERIOD: Duration = Duration::from_millis(33); // ~30 fps

/// Resolved per-username target. Constructed by [`Route::from_username`] and
/// dispatched by the SSH handler.
pub(crate) enum Route {
    /// `ssh help@…` — print the palette cheat-sheet and disconnect.
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

/// Out-of-band events the russh handler ships to a running [`serve_lava`] task.
pub(crate) enum SessionMsg {
    Input(Vec<u8>),
    Resize(u16, u16),
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
}

/// Help route — write the palette doc, close the channel, done.
pub(crate) async fn serve_help(handle: Handle, channel: ChannelId) {
    let _ = handle.data(channel, help_text()).await;
    let _ = handle.close(channel).await;
}

/// Lava route — own a [`Session`], push ANSI frames at `FRAME_PERIOD`, drain
/// inbound key/resize events, exit on disconnect / deadline / Ctrl-C.
pub(crate) async fn serve_lava(
    params: LavaParams,
    handle: Handle,
    mut msg_rx: mpsc::Receiver<SessionMsg>,
) {
    let LavaParams {
        peer,
        palette,
        channel,
        cols,
        rows,
        max_time,
    } = params;
    let started = Instant::now();
    let mut session = Session::new(cols, rows, palette);

    if handle
        .data(channel, term::ENTER_ALT_SCREEN.to_vec())
        .await
        .is_err()
    {
        info!(
            peer = ?peer,
            duration_secs = started.elapsed().as_secs(),
            reason = "write_failed",
            "session end"
        );
        return;
    }
    let _ = handle.data(channel, term::MOUSE_ENABLE.to_vec()).await;

    let mut buf: Vec<u8> = Vec::with_capacity(cols as usize * rows as usize * 24);
    let mut interval = tokio::time::interval(FRAME_PERIOD);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let deadline = tokio::time::sleep(max_time);
    tokio::pin!(deadline);

    let reason: &'static str;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                session.tick(FRAME_PERIOD.as_secs_f32());
                buf.clear();
                session.render(&mut buf);
                if handle.data(channel, buf.clone()).await.is_err() {
                    reason = "disconnect";
                    break;
                }
            }
            Some(msg) = msg_rx.recv() => match msg {
                SessionMsg::Input(bytes) => {
                    if session.feed_input(&bytes) {
                        reason = "client_exit";
                        break;
                    }
                }
                SessionMsg::Resize(c, r) => {
                    session.resize(c, r);
                    buf.reserve(c as usize * r as usize * 24);
                }
            },
            _ = &mut deadline => {
                reason = "timeout";
                break;
            }
        }
    }

    if reason == "timeout" {
        let bye = b"\r\n\x1b[0m*** session timed out ***\r\n";
        let _ = handle.data(channel, bye.to_vec()).await;
    }
    let _ = handle.data(channel, term::MOUSE_DISABLE.to_vec()).await;
    let _ = handle.data(channel, term::LEAVE_ALT_SCREEN.to_vec()).await;
    let _ = handle.close(channel).await;

    info!(
        peer = ?peer,
        duration_secs = started.elapsed().as_secs(),
        reason,
        "session end"
    );
}

/// Build the doc shown to clients connecting as `help@lava.watch` —
/// lists the palette usernames in their own color (bold) and one example.
fn help_text() -> Vec<u8> {
    use std::fmt::Write as _;
    let mut s = String::new();
    let width = Palette::ALL
        .iter()
        .map(|p| p.name().len())
        .max()
        .unwrap_or(0);
    write!(s, "\r\n  lava — pick a palette by SSH username:\r\n\r\n").unwrap();
    for p in Palette::ALL {
        let names = p.input_names();
        let canonical = names[0];
        let aliases = &names[1..];
        let (r, g, b) = p.accent();
        let pad = width.saturating_sub(canonical.len());
        write!(
            s,
            "    \x1b[1;38;2;{r};{g};{b}m{canonical}\x1b[0m{:pad$}",
            "",
        )
        .unwrap();
        if !aliases.is_empty() {
            write!(s, "  (").unwrap();
            for (i, a) in aliases.iter().enumerate() {
                if i > 0 {
                    write!(s, ", ").unwrap();
                }
                write!(s, "\x1b[1;38;2;{r};{g};{b}m{a}\x1b[0m").unwrap();
            }
            write!(s, ")").unwrap();
        }
        write!(s, "\r\n").unwrap();
    }
    write!(s, "\r\n  example: ssh uv@lava.watch\r\n\r\n").unwrap();
    s.into_bytes()
}
