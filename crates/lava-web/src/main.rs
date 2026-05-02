//! Standalone `lava-web` binary — initializes logging then runs the server.
//! For the all-in-one deployment, see the top-level `lava` crate.

use anyhow::Result;
use std::io::IsTerminal;

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "lava_web=info".into());

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
    lava_web::run(lava_web::config_from_env()).await
}
