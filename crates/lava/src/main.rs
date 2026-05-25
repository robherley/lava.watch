//! `lava` — single-binary entrypoint that runs the SSH, telnet, and static
//! web servers side by side. The libs are env-agnostic — this binary reads
//! the environment, builds typed configs, inits logging once, then
//! `tokio::try_join!`s the servers. Single self-contained executable.

use anyhow::Result;
use std::io::IsTerminal;
use std::time::Duration;

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "lava=info,lava_ssh=info,lava_telnet=info,lava_web=info".into());

    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if std::io::stdout().is_terminal() {
        builder.init();
    } else {
        builder.json().init();
    }
}

fn parse_env<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|s| s.parse().ok())
}

fn ssh_config() -> lava_ssh::Config {
    lava_ssh::Config {
        // `LAVA_PORT` is the old name — still honored as a fallback.
        port: parse_env::<u16>("LAVA_SSH_PORT")
            .or_else(|| parse_env::<u16>("LAVA_PORT"))
            .unwrap_or(2222),
        host_key: std::env::var("LAVA_HOST_KEY").unwrap_or_default(),
        host_key_password: std::env::var("LAVA_HOST_KEY_PASSWORD").ok(),
        max_conn_time: Duration::from_secs(parse_env::<u64>("LAVA_MAX_CONN_TIME").unwrap_or(300)),
        max_per_ip: parse_env::<usize>("LAVA_MAX_PER_IP").unwrap_or(3),
        speed: parse_env::<f32>("LAVA_SPEED").unwrap_or(0.8),
    }
}

fn telnet_config() -> lava_telnet::Config {
    lava_telnet::Config {
        // 5282 = "LAVA" on a phone keypad.
        port: parse_env::<u16>("LAVA_TELNET_PORT").unwrap_or(5282),
        // Session limits are shared with the SSH transport.
        max_conn_time: Duration::from_secs(parse_env::<u64>("LAVA_MAX_CONN_TIME").unwrap_or(300)),
        max_per_ip: parse_env::<usize>("LAVA_MAX_PER_IP").unwrap_or(3),
        speed: parse_env::<f32>("LAVA_SPEED").unwrap_or(0.8),
    }
}

fn web_config() -> lava_web::Config {
    lava_web::Config {
        port: parse_env::<u16>("LAVA_WEB_PORT").unwrap_or(8080),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    tokio::try_join!(
        lava_ssh::run(ssh_config()),
        lava_telnet::run(telnet_config()),
        lava_web::run(web_config()),
    )?;
    Ok(())
}
