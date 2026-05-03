//! ASCII renderer — produces an ANSI byte stream from a [`Lava`].
//!
//! Same shape as [`crate::term`], but each cell is a printable ASCII
//! character whose density tracks field intensity (` .:-=+*#%@`). One sample
//! per cell instead of two — half the vertical resolution of the half-block
//! path, but the chunky-text aesthetic is the point. Truecolor SGR is still
//! emitted: foreground = the lava's current color at the sample point,
//! background = the palette's bg gradient at the same y, so the lamp's
//! vertical falloff stays visible behind the texture.

use crate::palette::pixel_color;
use crate::Lava;
use std::io::Write;

/// Light-to-dense ramp keyed off field intensity. Index 0 = empty space,
/// index 9 = densest part of a blob.
const RAMP: &[u8; 10] = b" .:-=+*#%@";

/// Append a full ANSI-ASCII frame to `out`. Begins with cursor-home so
/// successive frames overwrite in place — caller handles initial alt-screen
/// entry (see [`crate::term::ENTER_ALT_SCREEN`]).
pub fn render(lava: &Lava, out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[H");

    let cols = lava.width as usize;
    let rows = (lava.height / 2) as usize;
    let h = lava.height as f32;
    let pal = lava.palette.colors();

    let mut last_fg: Option<(u8, u8, u8)> = None;
    let mut last_bg: Option<(u8, u8, u8)> = None;

    for r in 0..rows {
        for c in 0..cols {
            let xt = c as f32 + 0.5;
            // Sample at the center of the row's two-pixel band so the
            // cell represents both halves equally.
            let yt = (r * 2) as f32 + 1.0;
            let (field, heat) = lava.sample(xt, yt);

            let fg = pixel_color(&pal, field, heat, yt / h, lava.inverted);
            // bg = the bg gradient at this y, regardless of local field —
            // keeps the lamp's vertical color falloff visible behind the
            // ASCII texture instead of letting the terminal default show.
            let bg = pixel_color(&pal, 0.0, 0.0, yt / h, lava.inverted);
            let ch = ramp_char(field);

            if last_fg != Some(fg) {
                let _ = write!(out, "\x1b[38;2;{};{};{}m", fg.0, fg.1, fg.2);
                last_fg = Some(fg);
            }
            if last_bg != Some(bg) {
                let _ = write!(out, "\x1b[48;2;{};{};{}m", bg.0, bg.1, bg.2);
                last_bg = Some(bg);
            }
            out.push(ch);
        }
        out.extend_from_slice(b"\x1b[0m");
        if r + 1 < rows {
            out.extend_from_slice(b"\r\n");
        }
        last_fg = None;
        last_bg = None;
    }
}

/// Map field intensity to a ramp character. `field` is roughly:
/// `< 0.55` background, `0.55..1.0` glow, `>= 1.0` blob body (bigger inside
/// overlapping blobs). Linear scaling tuned so most of the screen reads as
/// space and blob centers approach `@`.
fn ramp_char(field: f32) -> u8 {
    let t = (field / 1.4).clamp(0.0, 1.0);
    let n = (RAMP.len() - 1) as f32;
    let idx = (t * n).round() as usize;
    RAMP[idx.min(RAMP.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn ramp_runs_from_space_to_at() {
        assert_eq!(ramp_char(0.0), b' ');
        assert_eq!(ramp_char(10.0), b'@');
    }

    #[test]
    fn renders_truecolor_ascii_characters() {
        let mut lava = Lava::with_config(
            40,
            20,
            Config {
                blob_count: 6,
                seed: 1234,
                ..Default::default()
            },
        );
        for _ in 0..30 {
            lava.step(1.0 / 30.0);
        }
        let mut buf = Vec::new();
        render(&lava, &mut buf);
        let s = std::str::from_utf8(&buf).expect("ascii output is utf-8");
        assert!(s.starts_with("\x1b[H"));
        assert!(s.contains("\x1b[38;2;"));
        assert!(s.contains("\x1b[48;2;"));
        // No half-block char.
        assert!(!s.contains('▀'));
        // At least one ramp char.
        assert!(RAMP.iter().any(|&c| s.contains(c as char)));
    }
}
