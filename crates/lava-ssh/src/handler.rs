//! `russh` glue. [`LavaServer`] is the per-process accept hook;
//! [`LavaHandler`] is the per-connection state and protocol callbacks. The
//! handler captures username/banner/PTY size as the SSH protocol advances,
//! then on `shell_request` consults [`Route`] and spawns the appropriate
//! task (help doc or lava session).

use crate::route::{serve_help, serve_lava, LavaParams, Route, SessionMsg};
use crate::tracker::{ConnSlot, ConnTracker};
use crate::Config;
use lava_engine::parse_input;
use russh::keys::ssh_key::PublicKey;
use russh::server::{Auth, Handler, Msg, Server, Session as RusshSession};
use russh::{Channel, ChannelId, Pty};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

const MAX_COLS: u16 = 512;
const MAX_ROWS: u16 = 256;

pub(crate) struct LavaServer {
    pub config: Arc<Config>,
    pub tracker: Arc<ConnTracker>,
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

pub(crate) struct LavaHandler {
    config: Arc<Config>,
    tracker: Arc<ConnTracker>,
    peer: Option<SocketAddr>,
    /// Held while the session is live; releases the connection slot on drop.
    slot: Option<ConnSlot>,
    pty_size: Option<(u16, u16)>,
    pty_term: Option<String>,
    banner: Option<String>,
    /// SSH username — used by [`Route::from_username`] to pick a palette
    /// (or the special `help` route).
    username: Option<String>,
    session_open: bool,
    msg_tx: Option<mpsc::Sender<SessionMsg>>,
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
        let handle = session.handle();
        let route = Route::from_username(self.username.as_deref());

        match route {
            Route::Help => {
                info!(peer = ?self.peer, "help requested");
                tokio::spawn(serve_help(handle, channel));
                Ok(())
            }
            Route::Lava(palette) => self.start_lava(channel, handle, palette).await,
        }
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
            let _ = tx.try_send(SessionMsg::Resize(cols, rows));
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
        if parse_input(data).is_some() {
            if let Some(tx) = &self.msg_tx {
                let _ = tx.try_send(SessionMsg::Input(data.to_vec()));
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

impl LavaHandler {
    /// Lava route plumbing — gated on PTY presence + slot acquisition,
    /// finally spawns [`serve_lava`].
    async fn start_lava(
        &mut self,
        channel: ChannelId,
        handle: russh::server::Handle,
        palette: lava_engine::Palette,
    ) -> Result<(), russh::Error> {
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
        let params = LavaParams {
            peer,
            palette,
            channel,
            cols,
            rows,
            max_time,
            speed: self.config.speed,
        };
        tokio::spawn(serve_lava(params, handle, rx));
        Ok(())
    }
}
