//! Terminal renderer — produces an ANSI byte stream from a [`Lava`].
//!
//! Each terminal cell is rendered as a half-block character `▀` whose
//! foreground encodes the top pixel and background the bottom pixel,
//! doubling the effective vertical resolution at no cost in cells written.
//! Adjacent cells with the same fg/bg skip redundant escapes for compactness.
//!
//! The same byte stream drives a real terminal (over SSH) and xterm.js
//! in the browser — both speak ANSI.

use crate::palette::{pixel_color, PaletteColors};
use crate::Lava;
use std::io::Write;

/// Switch to the alt screen, hide cursor, clear, home — write once on entry.
pub const ENTER_ALT_SCREEN: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H";
/// Show cursor, leave the alt screen — write once on exit.
pub const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?25h\x1b[?1049l";

/// Enable SGR mouse reporting (button events only, extended coords).
/// xterm.js and real terminals both honour this — the client begins sending
/// `\x1b[<{button};{col};{row}M` on press.
pub const MOUSE_ENABLE: &[u8] = b"\x1b[?1000h\x1b[?1006h";
/// Disable SGR mouse reporting — write once before tearing down a session.
pub const MOUSE_DISABLE: &[u8] = b"\x1b[?1006l\x1b[?1000l";

/// Begin synchronized output (DEC mode 2026). Modern terminals (iTerm2,
/// ghostty, kitty, alacritty, foot, wezterm, contour, …) buffer everything
/// between `BEGIN_SYNC` and `END_SYNC` and flip the screen atomically — no
/// half-drawn frames, no tearing on full-frame redraws over a slow link.
/// Terminals that don't recognise the sequence ignore it silently.
pub const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
/// End synchronized output and flush the buffered frame to the screen.
pub const END_SYNC: &[u8] = b"\x1b[?2026l";

pub(crate) type Color = (u8, u8, u8);

/// A single rendered terminal cell: foreground + background truecolor and the
/// glyph drawn between them. Colors are pre-[`quantize`]d, so two equal `Cell`s
/// produce identical bytes — which is what makes both the SGR run-coalescing
/// and the frame diffing correct.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Cell {
    pub fg: Color,
    pub bg: Color,
    pub glyph: char,
}

/// Channel quantization step. A full truecolor gradient changes by a digit or
/// two nearly every cell, which defeats both run-coalescing and frame diffing;
/// snapping each channel to a multiple of this (~32 levels) lets neighbours and
/// successive frames share colors. The lamp's soft gradients hide the banding.
const COLOR_STEP: u16 = 8;

/// Round each channel of `c` to the nearest [`COLOR_STEP`] multiple.
pub(crate) fn quantize(c: Color) -> Color {
    let q = |v: u8| (((v as u16 + COLOR_STEP / 2) / COLOR_STEP) * COLOR_STEP).min(255) as u8;
    (q(c.0), q(c.1), q(c.2))
}

/// Sample the half-block cell at terminal position `(col, row)` — fg encodes
/// the top pixel, bg the bottom, doubling effective vertical resolution.
/// `quant` snaps the colors to the [`quantize`] grid (bandwidth) when set.
pub(crate) fn cell(lava: &Lava, pal: &PaletteColors, col: usize, row: usize, quant: bool) -> Cell {
    let h = lava.height as f32;
    let xt = col as f32 + 0.5;
    let yt = (row * 2) as f32 + 0.5;
    let yb = (row * 2 + 1) as f32 + 0.5;
    let (ft, ht) = lava.sample(xt, yt);
    let (fb, hb) = lava.sample(xt, yb);
    let top = pixel_color(pal, ft, ht, yt / h, lava.inverted);
    let bot = pixel_color(pal, fb, hb, yb / h, lava.inverted);
    let (top, bot) = if quant {
        (quantize(top), quantize(bot))
    } else {
        (top, bot)
    };
    Cell {
        fg: top,
        bg: bot,
        glyph: '▀',
    }
}

/// Emit one cell's color escapes — coalesced against the last colors emitted
/// in this frame — followed by its glyph.
fn emit_cell(
    cell: &Cell,
    last_fg: &mut Option<Color>,
    last_bg: &mut Option<Color>,
    out: &mut Vec<u8>,
) {
    if *last_fg != Some(cell.fg) {
        let _ = write!(out, "\x1b[38;2;{};{};{}m", cell.fg.0, cell.fg.1, cell.fg.2);
        *last_fg = Some(cell.fg);
    }
    if *last_bg != Some(cell.bg) {
        let _ = write!(out, "\x1b[48;2;{};{};{}m", cell.bg.0, cell.bg.1, cell.bg.2);
        *last_bg = Some(cell.bg);
    }
    let mut b = [0u8; 4];
    out.extend_from_slice(cell.glyph.encode_utf8(&mut b).as_bytes());
}

/// Append a full ANSI frame to `out`: cursor-home (`ESC[H`), then every cell in
/// row order, so successive frames overwrite in place (the caller does the
/// initial clear — see [`ENTER_ALT_SCREEN`]). If `prev` is given it's filled
/// with this frame's cells, priming [`render_delta`]. `sample` yields the cell
/// at `(col, row)`.
pub(crate) fn render_full<F>(
    cols: usize,
    rows: usize,
    sample: F,
    mut prev: Option<&mut Vec<Cell>>,
    out: &mut Vec<u8>,
) where
    F: Fn(usize, usize) -> Cell,
{
    out.extend_from_slice(b"\x1b[H");
    if let Some(p) = prev.as_deref_mut() {
        p.clear();
        p.reserve(cols * rows);
    }

    let mut last_fg: Option<Color> = None;
    let mut last_bg: Option<Color> = None;
    for r in 0..rows {
        for c in 0..cols {
            let cell = sample(c, r);
            emit_cell(&cell, &mut last_fg, &mut last_bg, out);
            if let Some(p) = prev.as_deref_mut() {
                p.push(cell);
            }
        }
        // Reset at end of line so trailing terminal width beyond our render
        // doesn't inherit our bg color.
        out.extend_from_slice(b"\x1b[0m");
        if r + 1 < rows {
            out.extend_from_slice(b"\r\n");
        }
        last_fg = None;
        last_bg = None;
    }
}

/// Append only the cells that changed since `prev` (sized `cols * rows`,
/// updated in place). Each changed run is cursor-addressed once; unchanged
/// cells are skipped. The alt-screen persists between frames, so untouched
/// cells stay as they were — the caller must prime `prev` with [`render_full`]
/// first (and re-prime after anything that rewrites the whole screen).
pub(crate) fn render_delta<F>(
    cols: usize,
    rows: usize,
    sample: F,
    prev: &mut [Cell],
    out: &mut Vec<u8>,
) where
    F: Fn(usize, usize) -> Cell,
{
    let mut last_fg: Option<Color> = None;
    let mut last_bg: Option<Color> = None;
    // Where the cursor sits (where the next write would land), if known.
    let mut pen: Option<(usize, usize)> = None;
    for r in 0..rows {
        for c in 0..cols {
            let idx = r * cols + c;
            let cell = sample(c, r);
            if prev[idx] == cell {
                continue;
            }
            prev[idx] = cell;
            if pen != Some((r, c)) {
                let _ = write!(out, "\x1b[{};{}H", r + 1, c + 1);
            }
            emit_cell(&cell, &mut last_fg, &mut last_bg, out);
            // The glyph advanced the cursor one column.
            pen = Some((r, c + 1));
        }
    }
}

/// Append a full ANSI frame for `lava` (half-block). Convenience for non-delta
/// callers — the browser/wasm path and tests; streaming transports drive
/// [`render_full`] / [`render_delta`] with [`cell`] via `Session`.
pub fn render(lava: &Lava, out: &mut Vec<u8>) {
    let pal = lava.palette.colors();
    let cols = lava.width as usize;
    let rows = (lava.height / 2) as usize;
    // Full-quality (no quantization) — this convenience path serves the
    // browser/wasm and tests, not the bandwidth-sensitive transports.
    render_full(cols, rows, |c, r| cell(lava, &pal, c, r, false), None, out);
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
