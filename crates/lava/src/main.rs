//! `lava` — single-binary entrypoint that runs the SSH server and the
//! static web server side by side. Pure plumbing: read each subsystem's
//! env config, init logging once, then `tokio::try_join!` both. Deploys
//! as a single self-contained executable.

use anyhow::Result;
use std::io::IsTerminal;

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "lava=info,lava_ssh=info,lava_web=info".into());

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

    let ssh_cfg = lava_ssh::config_from_env();
    let web_cfg = lava_web::config_from_env();

    tokio::try_join!(lava_ssh::run(ssh_cfg), lava_web::run(web_cfg))?;
    Ok(())
}
