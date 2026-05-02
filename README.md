# lava

A lava lamp simulation served two ways: as ANSI half-blocks over SSH, or as
RGBA pixels on a `<canvas>` in the browser. One Rust simulation, two
output formats, one self-contained static binary.

## Run it locally

```sh
script/server
```

Then from any other terminal:

```sh
ssh -p 2222 localhost      # SSH transport
open http://localhost:8080/   # web transport (WASM in your browser)
```

`script/server` does three things on each invocation:

1. `script/setup` — generates a dev SSH host key in `.dev/` (gitignored) if missing.
2. `script/build-wasm` — `wasm-pack`s the engine to `wasm32-unknown-unknown` so `lava-web` can embed it.
3. `cargo run --release -p lava` — runs the all-in-one binary that hosts both transports.

### Pick a palette

By **SSH username** or by **URL path** — same parser, same aliases.

```sh
ssh ultraviolet@localhost   # or `uv@`, `blacklight@`
ssh pink@localhost          # → bubblegum
ssh ice@localhost           # → ocean
```

```
http://localhost:8080/aurora
http://localhost:8080/toxic
http://localhost:8080/uv
```

Anything that doesn't parse falls back to `classic`. Once connected,
**← / →** cycles palettes (the new name flashes briefly in the
bottom-left badge), and **left-click** anywhere on the lamp to heat that
spot — nearby blobs warm up and rise.

For the full palette list + aliases, connect as `help`:

```sh
ssh help@localhost
```

Prints a colored cheat-sheet and disconnects (no PTY required, no
connection slot consumed).

## Configuration

All env vars; both transports read what they need from the same environment.

| Var                  | Type   | Default          | Used by    | Description                          |
|----------------------|--------|------------------|------------|--------------------------------------|
| `LAVA_PORT`          | u16    | `2222`           | ssh        | SSH listen port                      |
| `LAVA_HOST_KEY`      | path   | `./host_key`     | ssh        | OpenSSH-format private host key      |
| `LAVA_MAX_CONN_TIME` | u64    | `300`            | ssh        | Hard session timeout, in seconds     |
| `LAVA_MAX_PER_IP`    | usize  | `3`              | ssh        | Concurrent SSH connections per IP    |
| `LAVA_WEB_PORT`      | u16    | `8080`           | web        | HTTP listen port                     |
| `RUST_LOG`           | string | `lava=info,…`    | both       | tracing-subscriber filter            |

Logs are pretty-printed when stdout is a TTY and JSON otherwise. SSH events
include `peer` (IP:port), `cols`/`rows`, `term` (client `$TERM`), `banner`
(client SSH version), `palette`, `duration_secs`, and a structured `reason`
(`client_exit`, `timeout`, `disconnect`, `write_failed`).

## Architecture

```
                ┌────────────────────────────┐
                │        lava-engine         │  pure Rust, no I/O
                │  sim · palette · session   │
                │  ┌──────────┐ ┌─────────┐  │
                │  │  term    │ │ pixels  │  │  two output paths
                │  │ (ANSI)   │ │ (RGBA)  │  │  same simulation
                │  └──────────┘ └─────────┘  │
                └─────┬──────────────┬───────┘
                      │              │
                      ▼              ▼
              ┌──────────────┐ ┌──────────────┐
              │  lava-ssh    │ │  lava-wasm   │
              │  (russh)     │ │(wasm-bindgen)│
              └──────┬───────┘ └──────┬───────┘
                     │                │
                     │                ▼
                     │         ┌──────────────┐
                     │         │   lava-web   │  axum static server,
                     │         │              │  embeds wasm bundle
                     │         └──────┬───────┘
                     ▼                ▼
                  ┌────────────────────────────┐
                  │            lava            │  single static binary,
                  │ tokio::try_join!(ssh, web) │  runs both servers
                  └────────────────────────────┘
```

The browser runs the simulation **client-side via WebAssembly**. No
WebSocket, no per-connection state, no rate limits — `lava-web` is pure
static hosting and the page calls `wasm.renderRgba()` → `ctx.putImageData`
on every animation frame. The whole web bundle (HTML, JS, WASM) is
`include_bytes!`'d into the binary.

## Library usage

ANSI / terminal output:

```rust
use lava_engine::{Palette, Session};

let mut session = Session::new(80, 30, Palette::Bubblegum);
let mut frame = Vec::new();
loop {
    session.tick(1.0 / 30.0);
    frame.clear();
    session.render(&mut frame);
    // write `frame` bytes to a PTY, stdout, …
}
```

RGBA pixel output (same engine, different sink):

```rust
session.render_rgba(&mut frame);
// frame is `width * height * 4` bytes — feed to a canvas, PNG encoder, …
```

## Layout

```
lava/
├── crates/
│   ├── lava/           single-binary entrypoint (ssh + web)
│   ├── lava-engine/    simulation, palettes, term + pixels renderers, Session
│   ├── lava-ssh/       SSH server (lib + bin)
│   ├── lava-wasm/      wasm-bindgen wrapper exposing the canvas API
│   └── lava-web/       axum static-asset server (lib + bin)
└── script/
    ├── setup           generate dev host key (idempotent)
    ├── build-wasm      wasm-pack build → static bundle
    ├── server          setup + build-wasm + run unified binary
    └── test            cargo fmt --check + clippy + test
```

## License

MIT
