//! `lava-telnet` — raw-TCP telnet server that streams a lava lamp to every
//! connection. The unauthenticated cousin of `lava-ssh`: no crypto, and no
//! username (so no palette routing — every session gets the default palette),
//! just the same engine frames over a socket that speaks the minimum telnet
//! needed to set up the terminal and learn the window size.
//!
//! Same hardening posture as the SSH transport: client input is parsed down
//! to known control sequences before it reaches the engine, window size is
//! clamped, sessions are time-bounded, and per-IP connection count is capped.

mod conn;
mod telnet;

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

/// User-facing config — the relevant subset of `lava_ssh::Config` (telnet has
/// no host key). Constructed by the binary entrypoint and passed to [`run`].
#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub max_conn_time: Duration,
    pub max_per_ip: usize,
    /// Simulation speed multiplier passed to each [`lava_engine::Session`].
    pub speed: f32,
}

/// Run the telnet server until the listener stops accepting (typically never).
/// Logging is the caller's responsibility — set up a `tracing` subscriber
/// before calling.
pub async fn run(cfg: Config) -> Result<()> {
    let tracker = Arc::new(lava_term::ConnTracker::default());
    let bind = format!("0.0.0.0:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("unable to bind telnet listener on {bind}"))?;
    info!(
        %bind,
        max_session_secs = cfg.max_conn_time.as_secs(),
        max_per_ip = cfg.max_per_ip,
        "lava-telnet listening"
    );

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                // Transient accept errors (fd exhaustion, etc.) shouldn't kill
                // the listener — log and keep going.
                warn!(error = %e, "accept failed");
                continue;
            }
        };
        debug!(peer = %peer, "client connected");

        // Enforce the per-IP cap before spawning the session.
        let Some(slot) = tracker.acquire(peer.ip(), cfg.max_per_ip) else {
            warn!(peer = %peer, "session refused: per-IP limit reached");
            tokio::spawn(refuse(stream));
            continue;
        };

        tokio::spawn(conn::serve(
            stream,
            conn::Params {
                peer,
                max_time: cfg.max_conn_time,
                speed: cfg.speed,
                slot,
            },
        ));
    }
}

/// Politely turn away a connection that hit the per-IP cap.
async fn refuse(mut stream: TcpStream) {
    let _ = stream.write_all(lava_term::BUSY_MESSAGE).await;
    let _ = stream.shutdown().await;
}
