//! End-to-end smoke test: spin up the real listener on an ephemeral port,
//! connect a plain TCP client, and verify the telnet handshake, the
//! alt-screen entry, that frames actually stream, and that `q` ends the
//! session cleanly.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// Telnet bytes we expect to see in the opening negotiation.
const IAC: u8 = 255;
const WILL: u8 = 251;
const DO: u8 = 253;
const OPT_ECHO: u8 = 1;
const OPT_NAWS: u8 = 31;

/// Grab a free port by binding to :0 and immediately releasing it.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn connect(port: u16) -> TcpStream {
    // The server task may not have bound yet — retry briefly.
    for _ in 0..50 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)).await {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("server never came up on port {port}");
}

#[tokio::test]
async fn streams_frames_and_quits() {
    let port = free_port();
    tokio::spawn(lava_telnet::run(lava_telnet::Config {
        port,
        max_conn_time: Duration::from_secs(5),
        max_per_ip: 3,
        speed: 0.8,
        frame_period: Duration::from_millis(33),
        quantize: true,
    }));

    let mut stream = connect(port).await;

    // Read enough of the opening bytes to cover negotiation + alt-screen + at
    // least one frame.
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    while got.len() < 256 {
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
            .await
            .expect("read timed out")
            .expect("read failed");
        assert_ne!(n, 0, "server closed before sending frames");
        got.extend_from_slice(&buf[..n]);
    }

    // Telnet handshake: we offer ECHO and request NAWS.
    assert!(
        contains(&got, &[IAC, WILL, OPT_ECHO]),
        "missing WILL ECHO negotiation"
    );
    assert!(
        contains(&got, &[IAC, DO, OPT_NAWS]),
        "missing DO NAWS negotiation"
    );
    // Engine framing: alt-screen entry and a synchronized-output frame marker.
    assert!(contains(&got, b"\x1b[?1049h"), "missing alt-screen entry");
    assert!(contains(&got, b"\x1b[?2026h"), "missing frame sync marker");

    // `q` should end the session — the server writes the outro and closes.
    stream.write_all(b"q").await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(2), drain_until_eof(&mut stream))
        .await
        .expect("session did not close after quit");
    assert!(closed, "expected clean EOF after quit");
}

#[tokio::test]
async fn enforces_per_ip_limit() {
    let port = free_port();
    tokio::spawn(lava_telnet::run(lava_telnet::Config {
        port,
        max_conn_time: Duration::from_secs(5),
        max_per_ip: 1,
        speed: 0.8,
        frame_period: Duration::from_millis(33),
        quantize: true,
    }));

    // First connection takes the only slot for this IP.
    let mut first = connect(port).await;
    let mut buf = [0u8; 256];
    tokio::time::timeout(Duration::from_secs(2), first.read(&mut buf))
        .await
        .expect("read timed out")
        .expect("read failed");

    // Second connection from the same IP is refused with a message, then closed.
    let mut second = connect(port).await;
    let mut msg = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), second.read_to_end(&mut msg))
        .await
        .expect("refused connection should close promptly")
        .expect("read failed");
    assert!(
        String::from_utf8_lossy(&msg).contains("too many connections"),
        "expected refusal message, got {:?}",
        String::from_utf8_lossy(&msg)
    );
}

/// Read until EOF; returns true on a clean close within the buffer budget.
async fn drain_until_eof(stream: &mut TcpStream) -> bool {
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => return true,
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
