//! `lava-ssh` — SSH server that streams a lava lamp to every connection.
//!
//! Public-facing toy. Hardened against the obvious abuse vectors:
//! all client input is dropped, PTY size is clamped, only one session
//! channel per connection, non-shell SSH requests are refused, and per-IP
//! connection count is bounded.

use anyhow::{Context, Result};
use lava_engine::{term, Lava};
use russh::keys::ssh_key::PublicKey;
use russh::keys::PrivateKey;
use russh::server::{Auth, Config, Handle, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, Pty};
use std::collections::HashMap;
use std::env;
use std::io::IsTerminal;
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
const SSH_WINDOW_SIZE: u32 = 32 * 1024;

// User-facing config, sourced from env vars.
#[derive(Clone, Debug)]
struct ServerConfig {
    port: u16,
    host_key: PathBuf,
    max_conn_time: Duration,
    max_per_ip: usize,
}

fn config_from_env() -> ServerConfig {
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
            session_open: false,
            event_tx: None,
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
    session_open: bool,
    event_tx: Option<mpsc::Sender<SessionEvent>>,
}

enum SessionEvent {
    Resize(u16, u16),
    Exit,
}

impl Handler for LavaHandler {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn auth_publickey(&mut self, _user: &str, _key: &PublicKey) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        session: &mut Session,
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
        _session: &mut Session,
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
        session: &mut Session,
    ) -> Result<(), Self::Error> {
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
            let msg = b"\r\ntoo many connections.\r\n";
            tokio::spawn(async move {
                let _ = handle.data(channel, msg.to_vec()).await;
                let _ = handle.close(channel).await;
            });
            return Ok(());
        };

        self.slot = Some(slot);

        let (tx, rx) = mpsc::channel(4);
        self.event_tx = Some(tx);
        let max_time = self.config.max_conn_time;
        let peer = self.peer;
        info!(
            peer = ?peer,
            cols,
            rows,
            term = ?self.pty_term,
            banner = ?self.banner,
            "session start"
        );
        tokio::spawn(async move {
            run_session(peer, handle, channel, cols, rows, max_time, rx).await;
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
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let cols = (col_width.min(u16::MAX as u32) as u16).clamp(1, MAX_COLS);
        let rows = (row_height.min(u16::MAX as u32) as u16).clamp(1, MAX_ROWS);
        if let Some(tx) = &self.event_tx {
            // Drop overflow rather than back up memory if a client floods resizes.
            let _ = tx.try_send(SessionEvent::Resize(cols, rows));
        }
        Ok(())
    }

    // We don't buffer keystrokes, but watch for Ctrl-C / Ctrl-D so the client
    // can disconnect — without this they'd have to use the SSH escape (`~.`).
    async fn data(
        &mut self,
        _channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        // 0x03 = Ctrl-C (ETX), 0x04 = Ctrl-D (EOT).
        if data.iter().any(|&b| b == 0x03 || b == 0x04) {
            if let Some(tx) = &self.event_tx {
                let _ = tx.try_send(SessionEvent::Exit);
            }
        }
        Ok(())
    }

    async fn extended_data(
        &mut self,
        _channel: ChannelId,
        _ext: u32,
        _data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    // Refuse anything beyond pty + shell. For session-internal requests
    // (exec/subsystem/env/x11/signal) we return Ok and just don't act.
    // For network-bearing channel opens we explicitly refuse with Ok(false).

    async fn channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn tcpip_forward(
        &mut self,
        _address: &str,
        _port: &mut u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Drop the event sender — the frame loop will notice on its next recv
        // (or on the next handle.data() failing once russh tears down).
        self.event_tx = None;
        Ok(())
    }
}

/// Per-session frame loop. Owns a [`Lava`] and pushes ANSI frames at
/// `FRAME_PERIOD`. Returns when the channel closes, the deadline fires, or
/// any write fails (client disconnect).
async fn run_session(
    peer: Option<SocketAddr>,
    handle: Handle,
    channel: ChannelId,
    cols: u16,
    rows: u16,
    max_time: Duration,
    mut event_rx: mpsc::Receiver<SessionEvent>,
) {
    let started = Instant::now();
    let mut lava = Lava::new(cols, rows);

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

    let mut buf: Vec<u8> = Vec::with_capacity(cols as usize * rows as usize * 24);
    let mut interval = tokio::time::interval(FRAME_PERIOD);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let deadline = tokio::time::sleep(max_time);
    tokio::pin!(deadline);

    let reason: &'static str;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                lava.step(FRAME_PERIOD.as_secs_f32());
                buf.clear();
                term::render(&lava, &mut buf);
                if handle.data(channel, buf.clone()).await.is_err() {
                    reason = "disconnect";
                    break;
                }
            }
            Some(ev) = event_rx.recv() => match ev {
                SessionEvent::Resize(c, r) => {
                    lava.resize(c, r);
                    buf.reserve(c as usize * r as usize * 24);
                }
                SessionEvent::Exit => {
                    reason = "client_exit";
                    break;
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
    let _ = handle.data(channel, term::LEAVE_ALT_SCREEN.to_vec()).await;
    let _ = handle.close(channel).await;

    info!(
        peer = ?peer,
        duration_secs = started.elapsed().as_secs(),
        reason,
        "session end"
    );
}

/// Pretty + colored when stdout is a TTY, JSON otherwise — so dev sessions
/// are readable and `journalctl`/`docker logs`/log shippers get something
/// machine-parseable. Override the level via `RUST_LOG` (e.g. `lava_ssh=debug`).
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "lava_ssh=info".into());

    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if std::io::stdout().is_terminal() {
        builder.init();
    } else {
        builder.json().init();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();

    let cfg = config_from_env();

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
