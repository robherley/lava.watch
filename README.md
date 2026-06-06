# lava.watch

A lava lamp simulation served three ways: as ANSI half-blocks over SSH or
telnet, or as RGBA pixels on a `<canvas>` in the browser. One Rust
simulation, two output formats, one self-contained static binary.

## Run it locally

```sh
script/server
```

Then from any other terminal:

```sh
ssh -p 2222 localhost         # SSH transport
telnet localhost 5282         # telnet transport (no auth, default palette)
open http://localhost:8080/   # web transport (WASM in your browser)
```

`script/server` does three things on each invocation:

1. `script/setup` — generates a dev SSH host key in `.dev/` (gitignored) if missing.
2. `script/build-wasm` — `wasm-pack`s the engine to `wasm32-unknown-unknown` so `lava-web` can embed it.
3. `cargo run --release -p lava` — runs the all-in-one binary that hosts every transport.

The telnet transport is the unauthenticated cousin of SSH: same engine, same
ANSI half-block frames, but no crypto and no username — so palette selection
(an SSH username, e.g. `ssh uv@…`) isn't available and every session uses the
default palette. Interactive keys (`←`/`→`, `i`, `a`, `?`, `q`) still work.

## Run it from npm

If you just want the lamp without cloning or running an SSH client:

```sh
npx lava-watch          # default palette
npx lava-watch uv       # ultraviolet
npx lava-watch --ascii  # start in ASCII mode (or hit `a` once running)
npx lava-watch --help   # help info
```

Same engine, compiled to WebAssembly and shipped as a Node CLI — no network,
no SSH required. Requires Node ≥ 18. Source: `npm/`, build with
`script/build-npm`.

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
bottom-left badge), **i** inverts colors, **a** swaps in the ASCII
renderer (` .:-=+*#%@` density ramp instead of half-blocks), and
**left-click** anywhere on the lamp to heat that spot — nearby blobs
warm up and rise.

For the full palette list + aliases, two equivalent ways:

```sh
ssh help@localhost
ssh localhost -- --help
```

Both print the colored help text and disconnect (no PTY required, no
connection slot consumed). The second form goes through `exec_request`,
so any `-- <anything>` falls back to the help doc — there are no other
commands to run.

## Configuration

All env vars; every transport reads what it needs from the same environment.

| Var                     | Type   | Default          | Used by      | Description                                                                |
|-------------------------|--------|------------------|--------------|----------------------------------------------------------------------------|
| `LAVA_SSH_PORT`         | u16    | `2222`           | ssh          | SSH listen port (falls back to the legacy `LAVA_PORT` if unset)            |
| `LAVA_HOST_KEY`         | string | *(required)*     | ssh          | Contents of an OpenSSH-format private host key (not a path)            |
| `LAVA_HOST_KEY_PASSWORD`| string | *(none)*         | ssh          | Passphrase for `LAVA_HOST_KEY` if it's encrypted                           |
| `LAVA_TELNET_PORT`      | u16    | `5282`           | telnet       | Telnet listen port (`5282` = "LAVA" on a phone keypad)                     |
| `LAVA_MAX_CONN_TIME`    | u64    | `300`            | ssh, telnet  | Hard session timeout, in seconds                                           |
| `LAVA_MAX_PER_IP`       | usize  | `3`              | ssh, telnet  | Concurrent connections per IP (each transport counts separately)           |
| `LAVA_SPEED`            | f32    | `0.8`            | ssh, telnet  | Simulation speed multiplier (`1.0` = engine "natural" rate, lower = slower) |
| `LAVA_FPS`              | u32    | `24`             | ssh, telnet  | Frames sent per second (1–60). Lower ≈ linearly less bandwidth; motion is unaffected (wall-clock `dt`) |
| `LAVA_QUANTIZE`         | bool   | `false`          | ssh, telnet  | Snap colors to a coarse grid to shrink frames. Off by default — full truecolor (more bandwidth, no banding) |
| `LAVA_WEB_PORT`         | u16    | `8080`           | web          | HTTP listen port                                                           |
| `RUST_LOG`              | string | `lava=info,…`    | all          | tracing-subscriber filter                                                  |

Logs are pretty-printed when stdout is a TTY and JSON otherwise. SSH events
include `peer` (IP:port), `cols`/`rows`, `term` (client `$TERM`), `banner`
(client SSH version), `palette`, `duration_secs`, and a structured `reason`
(`client_exit`, `timeout`, `disconnect`, `write_failed`). Telnet events carry
the same `peer`, `palette`, `duration_secs`, and `reason` fields.

## Architecture

```
                  ┌────────────────────────────┐
                  │        lava-engine         │
                  │  sim · palette · session   │
                  │  ┌──────────┐ ┌─────────┐  │
                  │  │  term    │ │ pixels  │  │
                  │  │ (ANSI)   │ │ (RGBA)  │  │
                  │  └──────────┘ └─────────┘  │
                  └──┬───────┬──────────┬──────┘
                     │       │          │
            ┌────────┘       │          └────────┐
            ▼                ▼                   ▼
     ┌──────────────┐ ┌──────────────┐  ┌──────────────┐
     │  lava-ssh    │ │ lava-telnet  │  │  lava-wasm   │
     │  (russh)     │ │  (raw TCP)   │  │(wasm-bindgen)│
     └──────┬───────┘ └──────┬───────┘  └──────┬───────┘
            │                │                 │
            │                │                 ▼
            │                │          ┌──────────────┐
            │                │          │   lava-web   │  axum static server,
            │                │          │              │  embeds wasm bundle
            │                │          └──────┬───────┘
            ▼                ▼                 ▼
         ┌─────────────────────────────────────────┐
         │                  lava                   │  single static binary,
         │ tokio::try_join!(ssh, telnet, web)      │  runs every server
         └─────────────────────────────────────────┘
```

The two terminal transports (`lava-ssh`, `lava-telnet`) are thin: each just
adapts its byte channel (an SSH channel handle, a TCP socket) into a
`FrameSink` and hands off to `lava-term`, which owns the shared frame loop,
the per-IP connection tracker, and the timing/size constants.

Streaming a full truecolor repaint every frame is expensive, so the terminal
path trims bandwidth four ways: a configurable frame rate
(`LAVA_FPS`); opt-in **color quantization** (channels snap to ~32 levels so
gradients coalesce into runs); **delta rendering** (only cells that changed since the
last frame are re-sent, cursor-addressed in place); and a tight **max render
size** so an oversized terminal can't blow up the byte stream. These apply
only to the ANSI transports — the browser renders RGBA client-side and is
unaffected.

The browser runs the simulation **client-side via WebAssembly**. The whole
web bundle (HTML, JS, WASM) is `include_bytes!`'d into the binary.

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
│   ├── lava/           single-binary entrypoint (ssh + telnet + web)
│   ├── lava-engine/    simulation, palettes, term + pixels renderers, Session
│   ├── lava-ssh/       SSH server library (russh)
│   ├── lava-telnet/    telnet server library (raw TCP, minimal IAC handling)
│   ├── lava-term/      shared frame loop, per-IP tracker + constants (ssh + telnet)
│   ├── lava-wasm/      wasm-bindgen wrapper exposing the canvas + Node CLI API
│   └── lava-web/       axum static-asset server library
├── npm/                lava-watch CLI — lava-wasm wrapped as an npx-able Node bin
└── script/
    ├── setup           generate dev host key (idempotent)
    ├── build-wasm      wasm-pack build → static bundle (web target)
    ├── build-npm       wasm-pack build → npm/pkg (nodejs target)
    ├── server          setup + build-wasm + run unified binary
    ├── lava-watch      build-npm + run the CLI (dev-loop `npx lava-watch`)
    └── test            cargo fmt --check + clippy + test
```

## License

MIT
