# lava

A lava lamp, rendered in your terminal via ANSI half-block characters.

## Run it locally

### As a standalone TUI

```sh
cargo run --release --example play -p lava-engine
```

←/→ cycles palettes. `q`, `Esc`, or Ctrl-C to exit. The lamp auto-sizes to
your terminal.

#### Configuration

| Var            | Type   | Default        | Description                                                                                |
|----------------|--------|----------------|--------------------------------------------------------------------------------------------|
| `LAVA_PALETTE` | string | `classic`      | `classic`, `toxic`, `bubblegum`, `mono`, `aurora`, `ocean`, `blood`, `ultraviolet` (+ aliases) |
| `LAVA_BLOBS`   | u32    | `7`            | Number of metaballs                                                                        |
| `LAVA_SPEED`   | f32    | `1.0`          | Simulation speed (`0.5` slow, `2.0` fast)                                                  |
| `LAVA_SEED`    | u64    | `0xC0FFEEF00D` | RNG seed — same seed reproduces the same lamp                                              |

```sh
LAVA_PALETTE=ocean LAVA_BLOBS=5 cargo run --release --example play -p lava-engine
```

### As an SSH server

```sh
script/server
```

Then from any other terminal:

```sh
ssh -p 2222 localhost
```

`script/server` calls `script/setup` first, which generates a dev SSH host key
in `.dev/` (gitignored) on first run.

#### Configuration

| Var                  | Type   | Default          | Description                          |
|----------------------|--------|------------------|--------------------------------------|
| `LAVA_PORT`          | u16    | `2222`           | TCP port to listen on                |
| `LAVA_HOST_KEY`      | path   | `./host_key`     | OpenSSH-format private host key      |
| `LAVA_MAX_CONN_TIME` | u64    | `300`            | Hard session timeout, in seconds     |
| `LAVA_MAX_PER_IP`    | usize  | `3`              | Concurrent connections per source IP |
| `RUST_LOG`           | string | `lava_ssh=info`  | tracing-subscriber filter            |

Logs are pretty-printed when stdout is a TTY and JSON otherwise. Log events
include `peer` (IP:port), `cols`/`rows`, `term` (client `$TERM`), `banner`
(client SSH version), `duration_secs`, and a structured `reason`
(`client_exit`, `timeout`, `disconnect`, `write_failed`).

## Library usage

```rust
use lava_engine::{term, Config, Lava, Palette};

let mut lava = Lava::with_config(80, 30, Config {
    palette: Palette::Bubblegum,
    blob_count: 6,
    speed: 1.0,
    seed: 42,
});

let mut frame = Vec::new();
loop {
    lava.step(1.0 / 30.0);
    frame.clear();
    term::render(&lava, &mut frame);
    // write `frame` bytes to a PTY, WebSocket, or stdout
}
```

## Layout

```
lava/
├── crates/
│   ├── lava-engine/    simulation + ANSI renderer (library + `play` example)
│   └── lava-ssh/       SSH server binary
└── script/
    ├── setup           generate dev host key in .dev/  (idempotent)
    ├── server          start the SSH server locally
    └── test            cargo fmt --check + clippy + test
```

## License

MIT
