//! `lava-web` — static HTTP server that serves a WASM-driven lava lamp.
//!
//! All assets (HTML, JS, CSS, WASM) are embedded at compile time, so the
//! produced binary is fully self-contained. The simulation runs entirely in
//! the visitor's browser — there's no per-connection state on the server,
//! no rate limiting, no streaming. Pure static.

use anyhow::{Context, Result};
use axum::{
    extract::Path,
    http::{header, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::env;
use std::net::SocketAddr;
use tracing::info;

const INDEX_HTML: &[u8] = include_bytes!("../static/index.html");
const LAVA_JS: &[u8] = include_bytes!("../static/lava.js");
const WASM_JS: &[u8] = include_bytes!("../../lava-wasm/pkg/lava_wasm.js");
const WASM_BG: &[u8] = include_bytes!("../../lava-wasm/pkg/lava_wasm_bg.wasm");

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub port: u16,
}

pub fn config_from_env() -> WebConfig {
    let port = env::var("LAVA_WEB_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);
    WebConfig { port }
}

pub async fn run(cfg: WebConfig) -> Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/{palette}", get(index_with_palette))
        .route("/static/lava.js", get(|| async { js(LAVA_JS) }))
        .route("/static/lava_wasm.js", get(|| async { js(WASM_JS) }))
        .route("/static/lava_wasm_bg.wasm", get(|| async { wasm(WASM_BG) }));

    let addr: SocketAddr = format!("0.0.0.0:{}", cfg.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    info!(%addr, "lava-web listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    html(INDEX_HTML)
}

async fn index_with_palette(Path(_palette): Path<String>) -> impl IntoResponse {
    // Palette is read by JS from window.location.pathname. The route exists
    // so /uv etc. don't 404; the body is identical to /.
    html(INDEX_HTML)
}

fn html(body: &'static [u8]) -> impl IntoResponse {
    response(body, "text/html; charset=utf-8")
}
fn js(body: &'static [u8]) -> impl IntoResponse {
    response(body, "application/javascript; charset=utf-8")
}
fn wasm(body: &'static [u8]) -> impl IntoResponse {
    response(body, "application/wasm")
}

fn response(body: &'static [u8], content_type: &'static str) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
        body,
    )
}
