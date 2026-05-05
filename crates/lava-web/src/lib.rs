//! `lava-web` — static HTTP server that serves a WASM-driven lava lamp.
//!
//! All assets (HTML, JS, CSS, WASM) are embedded at compile time, so the
//! produced binary is fully self-contained. The simulation runs entirely in
//! the visitor's browser — there's no per-connection state on the server,
//! no rate limiting, no streaming. Pure static.

use anyhow::{Context, Result};
use axum::{
    extract::{ConnectInfo, Request},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::hash::{DefaultHasher, Hasher};
use std::net::SocketAddr;
use std::time::Instant;
use tower_http::compression::CompressionLayer;
use tracing::info;

// Raw asset sources — `_RAW` ones contain `__…_HASH__` placeholders that
// `run` substitutes once at startup so the in-page references include
// content-hashed query strings (`/static/lava.js?v=<hash>`).
const INDEX_HTML_RAW: &str = include_str!("../static/index.html");
const LAVA_JS_RAW: &str = include_str!("../static/lava.js");
const WASM_JS: &[u8] = include_bytes!("../../lava-wasm/pkg/lava_wasm.js");
const WASM_BG: &[u8] = include_bytes!("../../lava-wasm/pkg/lava_wasm_bg.wasm");

// Hashed asset URLs are immutable: a new binary build → new hash → fresh
// cache entry.
const STATIC_CACHE: &str = "public, max-age=31536000, immutable";

/// User-facing config. Constructed by the binary entrypoint (env, CLI flags,
/// hardcoded for tests, etc.) and passed to [`run`].
#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
}

pub async fn run(cfg: Config) -> Result<()> {
    let Assets {
        index_html,
        lava_js,
    } = build_assets();

    let app = Router::new()
        .route("/", get(move || async move { html(index_html) }))
        .route("/{palette}", get(move || async move { html(index_html) }))
        .route("/static/lava.js", get(move || async move { js(lava_js) }))
        .route("/static/lava_wasm.js", get(|| async { js(WASM_JS) }))
        .route("/static/lava_wasm_bg.wasm", get(|| async { wasm(WASM_BG) }))
        // Compress text/JS/wasm responses based on Accept-Encoding.
        .layer(CompressionLayer::new())
        // One info-level log line per request, after the response is built —
        // outer layer so timing includes compression.
        .layer(middleware::from_fn(log_request));

    let addr: SocketAddr = format!("0.0.0.0:{}", cfg.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    info!(%addr, "lava-web listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// 16-hex-char hash of `bytes`. Not cryptographic — just enough to bust
/// caches when the content changes.
fn content_hash(bytes: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    h.write(bytes);
    format!("{:016x}", h.finish())
}

/// Substituted-and-leaked asset bodies that the route closures hand out.
/// `&'static [u8]` so the closures can hold them without further plumbing.
struct Assets {
    index_html: &'static [u8],
    lava_js: &'static [u8],
}

fn build_assets() -> Assets {
    let wasm_hash = content_hash(WASM_BG);
    let wasm_js_hash = content_hash(WASM_JS);

    let lava_js = LAVA_JS_RAW
        .replace("__WASM_HASH__", &wasm_hash)
        .replace("__WASM_JS_HASH__", &wasm_js_hash);
    let lava_js: &'static [u8] = Box::leak(lava_js.into_bytes().into_boxed_slice());
    let lava_js_hash = content_hash(lava_js);

    let index_html = INDEX_HTML_RAW.replace("__LAVA_JS_HASH__", &lava_js_hash);
    let index_html: &'static [u8] = Box::leak(index_html.into_bytes().into_boxed_slice());

    Assets {
        index_html,
        lava_js,
    }
}

async fn log_request(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let peer = client_addr(&req);
    let start = Instant::now();
    let res = next.run(req).await;
    info!(
        peer,
        %method,
        path,
        status = res.status().as_u16(),
        ms = start.elapsed().as_millis() as u64,
        "request"
    );
    res
}

/// Best-effort client address. Assumes running behind trusted proxy.
fn client_addr(req: &Request) -> String {
    if let Some(xff) = req.headers().get("x-forwarded-for") {
        if let Ok(s) = xff.to_str() {
            if let Some(first) = s.split(',').next() {
                let trimmed = first.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }
    if let Some(xri) = req.headers().get("x-real-ip") {
        if let Ok(s) = xri.to_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.to_string())
        .unwrap_or_default()
}

fn html(body: &'static [u8]) -> impl IntoResponse {
    response(body, "text/html; charset=utf-8", "no-cache")
}
fn js(body: &'static [u8]) -> impl IntoResponse {
    response(body, "application/javascript; charset=utf-8", STATIC_CACHE)
}
fn wasm(body: &'static [u8]) -> impl IntoResponse {
    response(body, "application/wasm", STATIC_CACHE)
}

fn response(
    body: &'static [u8],
    content_type: &'static str,
    cache_control: &'static str,
) -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(cache_control),
            ),
        ],
        body,
    )
}
