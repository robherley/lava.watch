//! Per-connection setup for the telnet transport. A small reader task parses
//! the inbound telnet stream into [`SessionMsg`]s; the frame loop itself lives
//! in [`lava_term::run_session`], which we feed via a [`TcpSink`].

use crate::telnet::{Event, Parser, INITIAL_NEGOTIATION};
use lava_engine::{Palette, Session};
use lava_term::{ConnSlot, FrameSink, SessionMsg};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::info;

// Window size assumed until the client's first NAWS report arrives.
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
// Bound a single inbound read; we discard most of what arrives anyway.
const READ_BUF: usize = 1024;

/// Everything [`serve`] needs, assembled by the listener after slot acquisition.
pub(crate) struct Params {
    pub peer: SocketAddr,
    pub max_time: Duration,
    pub frame_period: Duration,
    pub quantize: bool,
    pub speed: f32,
    /// Held for the connection's lifetime; releases the per-IP slot on drop.
    pub slot: ConnSlot,
}

/// Telnet byte sink — writes frames to the TCP socket's write half.
struct TcpSink {
    write_half: OwnedWriteHalf,
}

impl FrameSink for TcpSink {
    async fn write(&mut self, bytes: &[u8]) -> bool {
        self.write_half.write_all(bytes).await.is_ok()
    }

    async fn shutdown(&mut self) {
        let _ = self.write_half.shutdown().await;
    }
}

pub(crate) async fn serve(stream: TcpStream, params: Params) {
    let Params {
        peer,
        max_time,
        frame_period,
        quantize,
        speed,
        // Bound (not `_`) so the slot lives for the whole session and releases
        // on return.
        slot: _slot,
    } = params;
    let peer = peer.to_string();
    let started = Instant::now();

    // One frame per ~33ms: don't let Nagle batch them into visible jitter.
    let _ = stream.set_nodelay(true);
    let (mut read_half, write_half) = stream.into_split();

    // Reader task: parse the telnet stream into messages. Only recognized
    // control sequences survive `parse_input` downstream, so arbitrary
    // keystrokes are never buffered or acted on.
    let (tx, rx) = mpsc::channel::<SessionMsg>(8);
    let reader = tokio::spawn(async move {
        let mut parser = Parser::default();
        let mut read_buf = [0u8; READ_BUF];
        let mut events = Vec::new();
        let mut reply = Vec::new();
        loop {
            let n = match read_half.read(&mut read_buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            events.clear();
            reply.clear();
            parser.feed(&read_buf[..n], &mut events, &mut reply);
            for ev in events.drain(..) {
                let msg = match ev {
                    Event::Data(d) => SessionMsg::Input(d),
                    Event::Resize(c, r) => SessionMsg::Resize(c, r),
                };
                if tx.send(msg).await.is_err() {
                    return;
                }
            }
            // Option replies (refusals) ride the same channel as `Raw` so the
            // frame loop stays the only writer.
            if !reply.is_empty()
                && tx
                    .send(SessionMsg::Raw(std::mem::take(&mut reply)))
                    .await
                    .is_err()
            {
                return;
            }
        }
    });

    let palette = Palette::default();
    info!(peer, palette = palette.name(), "session start");
    let mut session = Session::with_seed(
        DEFAULT_COLS,
        DEFAULT_ROWS,
        palette,
        lava_term::random_seed(),
    );
    session.set_speed(speed);

    // The negotiation prelude goes out ahead of the shared alt-screen setup.
    let sink = TcpSink { write_half };
    let reason = lava_term::run_session(
        sink,
        session,
        rx,
        frame_period,
        max_time,
        quantize,
        INITIAL_NEGOTIATION,
    )
    .await;

    // `run_session` shut the socket down; just stop the reader.
    reader.abort();

    info!(
        peer,
        duration_secs = started.elapsed().as_secs(),
        reason = reason.as_str(),
        "session end"
    );
}
