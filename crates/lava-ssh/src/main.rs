//! Standalone `lava-ssh` binary — initializes logging then runs the server.
//! For the all-in-one deployment, see the top-level `lava` crate which calls
//! `lava_ssh::run` alongside other transports.

use anyhow::Result;
use std::io::IsTerminal;

/// Pretty + colored when stdout is a TTY, JSON otherwise — so dev sessions
/// are readable and `journalctl` / `docker logs` / log shippers get
/// machine-parseable output. Override the level via `RUST_LOG`.
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
    lava_ssh::run(lava_ssh::config_from_env()).await
}
