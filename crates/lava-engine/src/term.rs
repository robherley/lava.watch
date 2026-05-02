//! Terminal renderer — produces an ANSI byte stream from a [`LavaLamp`].
//!
//! Each terminal cell is rendered as a half-block character `▀` whose
//! foreground encodes the top pixel and background the bottom pixel,
//! doubling the effective vertical resolution at no cost in cells written.
//! Adjacent cells with the same fg/bg skip redundant escapes for compactness.
//!
//! The same byte stream drives a real terminal (over SSH) and xterm.js
//! (via wterm) in the browser — both speak ANSI.

use crate::palette::PaletteColors;
use crate::Lava;
use std::io::Write;

/// Switch to the alt screen, hide cursor, clear, home — write once on entry.
pub const ENTER_ALT_SCREEN: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H";
/// Show cursor, leave the alt screen — write once on exit.
pub const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?25h\x1b[?1049l";

/// Append a full ANSI frame to `out`. The frame begins with cursor-home
/// (`ESC[H`) so successive frames overwrite in place — the caller is
/// responsible for the initial screen clear (see [`ENTER_ALT_SCREEN`]).
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
            let yt = (r * 2) as f32 + 0.5;
            let yb = (r * 2 + 1) as f32 + 0.5;
            let (ft, ht) = lava.sample(xt, yt);
            let (fb, hb) = lava.sample(xt, yb);
            let top = pixel_color(&pal, ft, ht, yt / h);
            let bot = pixel_color(&pal, fb, hb, yb / h);

            if last_fg != Some(top) {
                let _ = write!(out, "\x1b[38;2;{};{};{}m", top.0, top.1, top.2);
                last_fg = Some(top);
            }
            if last_bg != Some(bot) {
                let _ = write!(out, "\x1b[48;2;{};{};{}m", bot.0, bot.1, bot.2);
                last_bg = Some(bot);
            }
            out.extend_from_slice("▀".as_bytes());
        }
        // Reset attributes at end of line so any trailing terminal width
        // beyond our render doesn't inherit our bg color.
        out.extend_from_slice(b"\x1b[0m");
        if r + 1 < rows {
            out.extend_from_slice(b"\r\n");
        }
        last_fg = None;
        last_bg = None;
    }
}

/// Map (palette, field intensity, local heat, vertical position v∈[0,1]) → RGB.
fn pixel_color(pal: &PaletteColors, field: f32, heat: f32, v: f32) -> (u8, u8, u8) {
    let bg = lerp3(pal.bg_top, pal.bg_bot, v.clamp(0.0, 1.0));

    if field < 0.55 {
        return rgb(bg);
    }

    if field < 1.0 {
        let g = (field - 0.55) / 0.45;
        let glow = lerp3(bg, pal.glow, g * 0.55);
        return rgb(glow);
    }

    let h = heat.clamp(0.0, 1.0);
    let body = if h < 0.5 {
        lerp3(pal.cool, pal.warm, h * 2.0)
    } else {
        lerp3(pal.warm, pal.hot, (h - 0.5) * 2.0)
    };
    let boost = ((field - 1.0) * 0.25).clamp(0.0, 0.4);
    rgb((
        (body.0 * (1.0 + boost)).min(255.0),
        (body.1 * (1.0 + boost)).min(255.0),
        (body.2 * (1.0 + boost)).min(255.0),
    ))
}

/// Component-wise linear interpolation between two RGB triples.
/// Returns `a` at `t=0`, `b` at `t=1`. `t` is not clamped — pass values in
/// `[0, 1]` for a true blend; values outside that range extrapolate.
fn lerp3(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

/// Convert a float RGB triple to `u8` channels by truncating toward zero.
/// `f32 as u8` saturates at the integer bounds, so out-of-range or NaN
/// inputs land at `0` or `255` rather than wrapping — but `pixel_color`
/// is expected to feed values already in `[0.0, 255.0]`.
fn rgb(c: (f32, f32, f32)) -> (u8, u8, u8) {
    (c.0 as u8, c.1 as u8, c.2 as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Palette};

    #[test]
    fn renders_a_frame_with_truecolor_and_half_blocks() {
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
        assert!(!buf.is_empty());
        let s = std::str::from_utf8(&buf).expect("ansi output is utf-8");
        assert!(s.contains("\x1b[38;2;"), "expected truecolor fg escape");
        assert!(s.contains("\x1b[48;2;"), "expected truecolor bg escape");
        assert!(s.contains('▀'), "expected half-block character");
        assert!(s.starts_with("\x1b[H"), "expected leading cursor-home");
    }

    #[test]
    fn each_palette_produces_distinct_output() {
        let make = |p: Palette| {
            let mut lava = Lava::with_config(
                20,
                10,
                Config {
                    palette: p,
                    blob_count: 3,
                    seed: 7,
                    ..Default::default()
                },
            );
            for _ in 0..10 {
                lava.step(1.0 / 30.0);
            }
            let mut buf = Vec::new();
            render(&lava, &mut buf);
            buf
        };
        let classic = make(Palette::Classic);
        let ocean = make(Palette::Ocean);
        assert_ne!(classic, ocean);
    }
}
