# lava

A lava lamp, rendered in your terminal via ANSI half-block characters.

## Run it locally

```sh
cargo run --release --example play
```

Ctrl-C to exit. The lamp auto-sizes to your terminal.

### Configuration

| Var            | Type   | Default        | Description                                               |
|----------------|--------|----------------|-----------------------------------------------------------|
| `LAVA_PALETTE` | string | `classic`      | `classic`, `ocean`, `toxic`, `sunset`, `mono` (+ aliases) |
| `LAVA_BLOBS`   | u32    | `7`            | Number of metaballs                                       |
| `LAVA_SPEED`   | f32    | `1.0`          | Simulation speed (`0.5` slow, `2.0` fast)                 |
| `LAVA_SEED`    | u64    | `0xC0FFEEF00D` | RNG seed — same seed reproduces the same lamp             |

```sh
LAVA_PALETTE=ocean LAVA_BLOBS=5 cargo run --release --example play
```

## Library usage

```rust
use lava_engine::{term, Config, Lava, Palette};

let mut lava = Lava::with_config(80, 30, Config {
    palette: Palette::Sunset,
    blob_count: 6,
    speed: 1.0,
    seed: 42,
});

let mut frame = Vec::new();
loop {
    lava.step(1.0 / 30.0);
    frame.clear();
    term::render(&lava, &mut frame);
    // write `frame` bytes to a PTY or stdout
}
```

## Layout

```
lava/
├── crates/
│   └── lava-engine/      simulation + ANSI renderer
└── README.md
```

## License

MIT
