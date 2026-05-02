//! `lava-ssh` — SSH server that streams a lava lamp to every connection.
//!
//! Public-facing toy. Hardened against the obvious abuse vectors:
//! all client input is dropped (only known control sequences are routed to
//! the [`lava_engine::Session`]), PTY size is clamped, only one session
//! channel per connection, non-shell SSH requests are refused, and per-IP
//! connection count is bounded.

mod handler;
mod route;
mod tracker;

use anyhow::{Context, Result};
use russh::keys::PrivateKey;
use russh::server::{Config as RusshConfig, Server};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

const PRE_SHELL_TIMEOUT: Duration = Duration::from_secs(30);
// Per-channel SSH flow-control window: how many bytes the client may send us
// before stalling. Russh's default is much larger; we keep it tight because
// we discard most client input and never grant more credit.
const SSH_WINDOW_SIZE: u32 = 32 * 1024;

/// User-facing config. Constructed by the binary entrypoint (env, CLI flags,
/// hardcoded for tests, etc.) and passed to [`run`].
#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub host_key: PathBuf,
    pub max_conn_time: Duration,
    pub max_per_ip: usize,
}

/// Run the SSH server until it stops accepting connections (typically never).
/// Logging is the caller's responsibility — set up a `tracing` subscriber
/// before calling.
pub async fn run(cfg: Config) -> Result<()> {
    let key: PrivateKey = russh::keys::load_secret_key(&cfg.host_key, None).with_context(|| {
        format!(
            "loading host key from {} (generate with: ssh-keygen -t ed25519 -f {} -N '')",
            cfg.host_key.display(),
            cfg.host_key.display(),
        )
    })?;

    let russh_cfg = Arc::new(RusshConfig {
        keys: vec![key],
        inactivity_timeout: Some(PRE_SHELL_TIMEOUT),
        window_size: SSH_WINDOW_SIZE,
        ..Default::default()
    });

    let mut server = handler::LavaServer {
        config: Arc::new(cfg.clone()),
        tracker: Arc::new(tracker::ConnTracker::default()),
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
