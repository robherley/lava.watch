//! `lava-ssh` — SSH server that streams a lava lamp to every connection.
//!
//! Public-facing toy. Hardened against the obvious abuse vectors:
//! all client input is dropped (only known control sequences are routed to
//! the [`lava_engine::Session`]), PTY size is clamped, only one session
//! channel per connection, non-shell SSH requests are refused, and per-IP
//! connection count is bounded.

use anyhow::{Context, Result};
use lava_engine::{parse_input, term, Palette, Session};
use russh::keys::ssh_key::PublicKey;
use russh::keys::PrivateKey;
use russh::server::{Auth, Config, Handle, Handler, Msg, Server, Session as RusshSession};
use russh::{Channel, ChannelId, Pty};
use std::collections::HashMap;
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

const MAX_COLS: u16 = 512;
const MAX_ROWS: u16 = 256;
const FRAME_PERIOD: Duration = Duration::from_millis(33); // ~30 fps
const PRE_SHELL_TIMEOUT: Duration = Duration::from_secs(30);
// Per-channel SSH flow-control window: how many bytes the client may send us
// before stalling. Russh's default is much larger; we keep it tight because
// we discard most client input and never grant more credit.
const SSH_WINDOW_SIZE: u32 = 32 * 1024;

/// User-facing config, sourced from env vars.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub port: u16,
    pub host_key: PathBuf,
    pub max_conn_time: Duration,
    pub max_per_ip: usize,
}

pub fn config_from_env() -> ServerConfig {
    fn parse<T: std::str::FromStr>(key: &str) -> Option<T> {
        env::var(key).ok().and_then(|s| s.parse().ok())
    }
    ServerConfig {
        port: parse::<u16>("LAVA_PORT").unwrap_or(2222),
        host_key: env::var("LAVA_HOST_KEY")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./host_key")),
        max_conn_time: Duration::from_secs(parse::<u64>("LAVA_MAX_CONN_TIME").unwrap_or(300)),
        max_per_ip: parse::<usize>("LAVA_MAX_PER_IP").unwrap_or(3),
    }
}

/// Run the SSH server until it stops accepting connections (typically never).
/// Logging is the caller's responsibility — set up a `tracing` subscriber
/// before calling.
pub async fn run(cfg: ServerConfig) -> Result<()> {
    let key: PrivateKey = russh::keys::load_secret_key(&cfg.host_key, None).with_context(|| {
        format!(
            "loading host key from {} (generate with: ssh-keygen -t ed25519 -f {} -N '')",
            cfg.host_key.display(),
            cfg.host_key.display(),
        )
    })?;

    let russh_cfg = Arc::new(Config {
        keys: vec![key],
        inactivity_timeout: Some(PRE_SHELL_TIMEOUT),
        window_size: SSH_WINDOW_SIZE,
        ..Default::default()
    });

    let mut server = LavaServer {
        config: Arc::new(cfg.clone()),
        tracker: Arc::new(ConnTracker::default()),
    };

    let bind = format!("0.0.0.0:{}", cfg.port);
    info!(
        %bind,
        max_session_secs = cfg.max_conn_time.as_secs(),
        max_per_ip = cfg.max_per_ip,
        "lava-ssh listening"
    );

    server.run_on_address(russh_cfg, bind).await?;
    Ok(())
}

#[derive(Default)]
struct ConnTracker {
    counts: Mutex<HashMap<IpAddr, usize>>,
}

impl ConnTracker {
    /// Acquire a slot for `ip`. Returns `Some(guard)` on success — the count
    /// is automatically decremented when the guard is dropped. Returns `None`
    /// if the per-IP cap would be exceeded.
    fn acquire(self: &Arc<Self>, ip: IpAddr, per_ip: usize) -> Option<ConnSlot> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = counts.entry(ip).or_insert(0);
        if *entry >= per_ip {
            return None;
        }
        *entry += 1;
        Some(ConnSlot {
            tracker: Arc::clone(self),
            ip,
        })
    }

    fn release(&self, ip: IpAddr) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(c) = counts.get_mut(&ip) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                counts.remove(&ip);
            }
        }
    }
}

/// RAII guard for a connection slot. Releases the slot on drop.
struct ConnSlot {
    tracker: Arc<ConnTracker>,
    ip: IpAddr,
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.tracker.release(self.ip);
    }
}

struct LavaServer {
    config: Arc<ServerConfig>,
    tracker: Arc<ConnTracker>,
}

impl Server for LavaServer {
    type Handler = LavaHandler;

    fn new_client(&mut self, addr: Option<SocketAddr>) -> Self::Handler {
        debug!(peer = ?addr, "client connected");
        LavaHandler {
            config: self.config.clone(),
            tracker: self.tracker.clone(),
            peer: addr,
            slot: None,
            pty_size: None,
            pty_term: None,
            banner: None,
            username: None,
            session_open: false,
            msg_tx: None,
        }
    }
}

struct LavaHandler {
    config: Arc<ServerConfig>,
    tracker: Arc<ConnTracker>,
    peer: Option<SocketAddr>,
    /// Held while the session is live; releases the connection slot on drop.
    slot: Option<ConnSlot>,
    pty_size: Option<(u16, u16)>,
    pty_term: Option<String>,
    banner: Option<String>,
    /// SSH username — used to pick a palette (e.g. `ssh uv@lava.watch`).
    username: Option<String>,
    session_open: bool,
    msg_tx: Option<mpsc::Sender<HandlerMsg>>,
}

/// Out-of-band events the russh handler ships to the per-session task.
enum HandlerMsg {
    Input(Vec<u8>),
    Resize(u16, u16),
}

struct SessionParams {
    peer: Option<SocketAddr>,
    palette: Palette,
    channel: ChannelId,
    cols: u16,
    rows: u16,
    max_time: Duration,
}

impl Handler for LavaHandler {
    type Error = russh::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        self.username = Some(user.to_string());
        Ok(Auth::Accept)
    }

    async fn auth_publickey(&mut self, user: &str, _key: &PublicKey) -> Result<Auth, Self::Error> {
        self.username = Some(user.to_string());
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        session: &mut RusshSession,
    ) -> Result<bool, Self::Error> {
        if self.session_open {
            // One session channel per connection — refuse the rest.
            return Ok(false);
        }
        self.session_open = true;
        // First chance to grab the client's SSH banner — exchanged before auth.
        if self.banner.is_none() {
            let id = session.remote_sshid();
            if !id.is_empty() {
                self.banner = Some(String::from_utf8_lossy(id).into_owned());
            }
        }
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        _channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        _session: &mut RusshSession,
    ) -> Result<(), Self::Error> {
        let cols = (col_width.min(u16::MAX as u32) as u16).clamp(1, MAX_COLS);
        let rows = (row_height.min(u16::MAX as u32) as u16).clamp(1, MAX_ROWS);
        self.pty_size = Some((cols, rows));
        self.pty_term = Some(term.to_string());
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut RusshSession,
    ) -> Result<(), Self::Error> {
        // `ssh help@lava.watch` prints a help doc and disconnects — no PTY
        // required, no connection slot consumed.
        if self.username.as_deref() == Some("help") {
            info!(peer = ?self.peer, "help requested");
            let handle = session.handle();
            tokio::spawn(async move {
                let _ = handle.data(channel, help_text()).await;
                let _ = handle.close(channel).await;
            });
            return Ok(());
        }

        let Some((cols, rows)) = self.pty_size else {
            // Shell without a PTY makes no sense for us.
            return Ok(());
        };

        // Try to acquire a connection slot. If we can't identify the peer
        // (no SocketAddr from russh), refuse — we can't enforce limits on it.
        let slot = self
            .peer
            .map(|p| p.ip())
            .and_then(|ip| self.tracker.acquire(ip, self.config.max_per_ip));

        let handle = session.handle();

        let Some(slot) = slot else {
            warn!(peer = ?self.peer, "session refused: per-IP limit reached");
            let msg = b"\r\ntoo many connections, try again shortly.\r\n";
            tokio::spawn(async move {
                let _ = handle.data(channel, msg.to_vec()).await;
                let _ = handle.close(channel).await;
            });
            return Ok(());
        };

        self.slot = Some(slot);

        // Username doubles as a palette selector (`ssh uv@lava.watch`).
        let palette = self
            .username
            .as_deref()
            .map(Session::palette_from_str)
            .unwrap_or_default();

        let (tx, rx) = mpsc::channel(8);
        self.msg_tx = Some(tx);
        let max_time = self.config.max_conn_time;
        let peer = self.peer;
        info!(
            peer = ?peer,
            cols,
            rows,
            term = ?self.pty_term,
            banner = ?self.banner,
            user = ?self.username,
            palette = palette.name(),
            "session start"
        );
        let params = SessionParams {
            peer,
            palette,
            channel,
            cols,
            rows,
            max_time,
        };
        tokio::spawn(async move {
            run_session(params, handle, rx).await;
        });

        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut RusshSession,
    ) -> Result<(), Self::Error> {
        let cols = (col_width.min(u16::MAX as u32) as u16).clamp(1, MAX_COLS);
        let rows = (row_height.min(u16::MAX as u32) as u16).clamp(1, MAX_ROWS);
        if let Some(tx) = &self.msg_tx {
            // Drop overflow rather than back up memory if a client floods resizes.
            let _ = tx.try_send(HandlerMsg::Resize(cols, rows));
        }
        Ok(())
    }

    /// Forward client bytes to the session task. We pre-filter to the
    /// recognized control bytes so we never buffer arbitrary keystrokes.
    async fn data(
        &mut self,
        _channel: ChannelId,
        data: &[u8],
        _session: &mut RusshSession,
    ) -> Result<(), Self::Error> {
        // Only forward if it parses to a known input — keeps random keystrokes
        // off the channel entirely.
        if parse_input(data).is_some() {
            if let Some(tx) = &self.msg_tx {
                let _ = tx.try_send(HandlerMsg::Input(data.to_vec()));
            }
        }
        Ok(())
    }

    async fn extended_data(
        &mut self,
        _channel: ChannelId,
        _ext: u32,
        _data: &[u8],
        _session: &mut RusshSession,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut RusshSession,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn tcpip_forward(
        &mut self,
        _address: &str,
        _port: &mut u32,
        _session: &mut RusshSession,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut RusshSession,
    ) -> Result<(), Self::Error> {
        // Drop the sender — the frame loop will notice on its next recv
        // (or on the next handle.data() failing once russh tears down).
        self.msg_tx = None;
        Ok(())
    }
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

/// Per-session frame loop. Owns a [`Session`] and pushes ANSI frames at
/// `FRAME_PERIOD`. Returns when the channel closes, the deadline fires, or
/// any write fails (client disconnect).
async fn run_session(
    params: SessionParams,
    handle: Handle,
    mut msg_rx: mpsc::Receiver<HandlerMsg>,
) {
    let SessionParams {
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
                HandlerMsg::Input(bytes) => {
                    if session.feed_input(&bytes) {
                        reason = "client_exit";
                        break;
                    }
                }
                HandlerMsg::Resize(c, r) => {
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
        let bye = b"\r\n\x1b[0m  *** session timed out ***\r\n";
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
