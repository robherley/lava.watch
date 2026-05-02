//! Pixel renderer — produces a flat RGBA byte stream sized to the engine's
//! native resolution. Designed to be blitted onto a `<canvas>` via
//! `ctx.putImageData(new ImageData(bytes, width, height), 0, 0)`.
//!
//! Sibling of [`crate::term`]: same simulation sampling, different output.
//! No half-block trick, no terminal emulator, no escape parsing — the
//! browser just memcpy's the bytes onto a 2D context.

use crate::palette::pixel_color;
use crate::Lava;

/// Append `width * height * 4` bytes of RGBA to `out` (alpha is always 255).
/// `out` is cleared before writing.
pub fn render(lava: &Lava, out: &mut Vec<u8>) {
    let w = lava.width as usize;
    let h = lava.height as usize;
    let h_f = lava.height as f32;
    let pal = lava.palette.colors();

    out.clear();
    out.reserve(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (field, heat) = lava.sample(x as f32 + 0.5, y as f32 + 0.5);
            let v = (y as f32 + 0.5) / h_f;
            let (r, g, b) = pixel_color(&pal, field, heat, v, lava.inverted);
            out.push(r);
            out.push(g);
            out.push(b);
            out.push(255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Palette};

    #[test]
    fn produces_expected_byte_count() {
        let lava = Lava::with_config(
            10,
            5,
            Config {
                blob_count: 2,
                seed: 1,
                ..Default::default()
            },
        );
        // width=10, height=2*5=10 → 10*10*4 = 400 bytes.
        let mut buf = Vec::new();
        render(&lava, &mut buf);
        assert_eq!(buf.len(), 400);
        // alpha channel is always 255.
        for chunk in buf.chunks_exact(4) {
            assert_eq!(chunk[3], 255);
        }
    }

    #[test]
    fn each_palette_produces_distinct_output() {
        let make = |p: Palette| {
            let mut lava = Lava::with_config(
                12,
                6,
                Config {
                    palette: p,
                    blob_count: 2,
                    seed: 9,
                    ..Default::default()
                },
            );
            for _ in 0..5 {
                lava.step(1.0 / 30.0);
            }
            let mut buf = Vec::new();
            render(&lava, &mut buf);
            buf
        };
        assert_ne!(make(Palette::Classic), make(Palette::Ocean));
    }
}
