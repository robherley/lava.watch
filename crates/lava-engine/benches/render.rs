//! Hot-path microbenchmarks. Run with `cargo bench -p lava-engine`.
//!
//! What's measured:
//! - `tick`: one physics step on the default 7-blob lamp (cheap).
//! - `term::render` at typical SSH terminal sizes — the dominant cost on
//!   the SSH path is the per-cell metaball sample plus ANSI byte writing.
//! - `pixels::render` at canvas-typical pixel grids — twice the samples per
//!   cell area than the half-block term path.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use lava_engine::{Palette, Session};

fn warmed_session(cols: u16, rows: u16) -> Session {
    let mut s = Session::new(cols, rows, Palette::Classic);
    // 30 ticks of warmup so blobs spread out a bit before we measure.
    for _ in 0..30 {
        s.tick(1.0 / 30.0);
    }
    s
}

fn bench_tick(c: &mut Criterion) {
    let mut s = warmed_session(80, 30);
    c.bench_function("Session::tick", |b| {
        b.iter(|| s.tick(black_box(1.0 / 30.0)));
    });
}

fn bench_term_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("term::render");
    for &(cols, rows) in &[(80u16, 24u16), (120, 40), (200, 50)] {
        let s = warmed_session(cols, rows);
        let mut buf = Vec::with_capacity(cols as usize * rows as usize * 24);
        group.bench_function(format!("{cols}x{rows}"), |b| {
            b.iter(|| {
                buf.clear();
                s.render(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_pixels_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("pixels::render");
    // (cols, rows) — engine pixel grid is `cols × 2*rows`. So `(640, 180)`
    // → 640×360, the lava-web default; `(960, 270)` → 960×540 for a denser
    // canvas.
    for &(cols, rows) in &[(640u16, 180u16), (960, 270)] {
        let s = warmed_session(cols, rows);
        let mut buf = Vec::with_capacity(cols as usize * rows as usize * 8);
        group.bench_function(format!("{cols}x{}", rows * 2), |b| {
            b.iter(|| {
                buf.clear();
                s.render_rgba(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_tick, bench_term_render, bench_pixels_render);
criterion_main!(benches);
