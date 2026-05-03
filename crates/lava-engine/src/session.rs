//! High-level interactive session — wraps [`crate::Lava`] with palette
//! cycling, click-to-heat, and a transient palette-name overlay. Used by
//! every transport (SSH, browser-via-WASM) as the single source of behavior.
//!
//! Transports just do I/O:
//! - feed bytes from the client into [`Session::feed_input`]
//! - call [`Session::tick`] on a timer (typically 30fps)
//! - call [`Session::render`] to produce the next ANSI frame to send back

use crate::{ascii, term, Config as LavaConfig, Lava, Palette};

/// Which renderer [`Session::render`] dispatches to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// `▀` half-block, two pixel samples packed per terminal cell.
    HalfBlock,
    /// Density ramp ASCII (` .:-=+*#%@`), one sample per cell.
    Ascii,
}

/// Bit-flip every channel — matches the engine's photographic-negative
/// transform so chrome inverts in lockstep with the lamp.
fn invert_if(c: (u8, u8, u8), inverted: bool) -> (u8, u8, u8) {
    if inverted {
        (255 - c.0, 255 - c.1, 255 - c.2)
    } else {
        c
    }
}

/// How many frames the bottom-left palette badge stays visible after a switch.
const OVERLAY_FRAMES: u32 = 90;

/// Radius (in engine pixels) of the heat splash when the user clicks.
const HEAT_RADIUS_PX: f32 = 12.0;

/// Parsed session-level input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    PaletteNext,
    PalettePrev,
    /// Toggle the photographic-negative render.
    ToggleInverted,
    /// Toggle between half-block and ASCII renderers.
    ToggleAscii,
    /// Show / hide the bottom keybind strip.
    ToggleMenu,
    Exit,
    /// Left-button click at a 1-indexed terminal cell.
    Click {
        col: u16,
        row: u16,
    },
}

/// Parse a chunk of input bytes from a terminal-style client (real terminal
/// over SSH, xterm.js in the browser) into a session [`Input`]. Returns
/// `None` for unrecognized input.
pub fn parse_input(data: &[u8]) -> Option<Input> {
    match data {
        // CSI / SS3 right + left arrow keys.
        b"\x1b[C" | b"\x1bOC" => return Some(Input::PaletteNext),
        b"\x1b[D" | b"\x1bOD" => return Some(Input::PalettePrev),
        // 'i' / 'I' toggle inverted render.
        b"i" | b"I" => return Some(Input::ToggleInverted),
        // 'a' / 'A' toggle ASCII renderer.
        b"a" | b"A" => return Some(Input::ToggleAscii),
        // 'h' / 'H' show/hide the bottom keybind strip.
        b"h" | b"H" => return Some(Input::ToggleMenu),
        // 'q' / 'Q' quit (alongside Ctrl-C / Ctrl-D below).
        b"q" | b"Q" => return Some(Input::Exit),
        _ => {}
    }
    // 0x03 = Ctrl-C (ETX), 0x04 = Ctrl-D (EOT) — checked anywhere in chunk.
    if data.iter().any(|&b| b == 0x03 || b == 0x04) {
        return Some(Input::Exit);
    }
    parse_mouse_press(data).map(|(col, row)| Input::Click { col, row })
}

/// Parse an SGR mouse press: `\x1b[<{button};{col};{row}M`.
/// Returns `Some((col, row))` only for left-button presses.
fn parse_mouse_press(data: &[u8]) -> Option<(u16, u16)> {
    let s = std::str::from_utf8(data).ok()?;
    let body = s.strip_prefix("\x1b[<")?.strip_suffix('M')?;
    let mut parts = body.split(';');
    let button: u32 = parts.next()?.parse().ok()?;
    let col: u16 = parts.next()?.parse().ok()?;
    let row: u16 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || button != 0 {
        return None;
    }
    Some((col, row))
}

/// Owns the [`Lava`] simulation plus a small amount of UI state (current
/// palette index, overlay countdown).
pub struct Session {
    lava: Lava,
    palette_idx: usize,
    overlay_frames: u32,
    mode: RenderMode,
    show_menu: bool,
}

impl Session {
    pub fn new(cols: u16, rows: u16, palette: Palette) -> Self {
        let lava = Lava::with_config(
            cols,
            rows,
            LavaConfig {
                palette,
                ..LavaConfig::default()
            },
        );
        let palette_idx = Palette::ALL.iter().position(|p| *p == palette).unwrap_or(0);
        Self {
            lava,
            palette_idx,
            overlay_frames: 0,
            mode: RenderMode::HalfBlock,
            show_menu: true,
        }
    }

    /// Resolve a string (SSH username, URL path segment, etc.) to a palette
    /// via [`Palette`]'s `FromStr`, falling back to the default on miss.
    pub fn palette_from_str(s: &str) -> Palette {
        s.parse().unwrap_or_default()
    }

    pub fn current_palette(&self) -> Palette {
        Palette::ALL[self.palette_idx]
    }

    /// Logical dimensions in terminal cells (rows = pixel-height / 2 because
    /// of the half-block trick).
    pub fn dimensions(&self) -> (u16, u16) {
        (self.lava.width, self.lava.height / 2)
    }

    /// Native pixel dimensions of the engine's grid — what the [`pixels`]
    /// renderer outputs and what a `<canvas>` should be sized to.
    pub fn pixel_dimensions(&self) -> (u16, u16) {
        (self.lava.width, self.lava.height)
    }

    /// Advance the simulation by `dt` seconds. Decrements the overlay
    /// countdown if active.
    pub fn tick(&mut self, dt: f32) {
        self.lava.step(dt);
        if self.overlay_frames > 0 {
            self.overlay_frames -= 1;
        }
    }

    /// Append the next ANSI frame to `out` — lava body, the persistent
    /// bottom keybind strip (unless hidden), and the transient palette
    /// badge in the screen's center if a cycle just happened. Caller
    /// handles initial alt-screen entry / mouse-mode setup. The renderer is
    /// picked from [`Session::render_mode`].
    pub fn render(&self, out: &mut Vec<u8>) {
        match self.mode {
            RenderMode::HalfBlock => term::render(&self.lava, out),
            RenderMode::Ascii => ascii::render(&self.lava, out),
        }
        if self.show_menu {
            self.render_keybind_strip(out);
        }
        if self.overlay_frames > 0 {
            self.render_palette_badge(out);
        }
    }

    /// Bottom-row keybind strip — left-aligned with a small indent, full
    /// row width, muted glow fg on bg-gradient bg so it blends with the
    /// bottom of the lamp instead of competing with it. Skipped silently
    /// if the terminal is too narrow to fit the text.
    fn render_keybind_strip(&self, out: &mut Vec<u8>) {
        use std::io::Write;
        const TEXT: &str = "← / →  palette  ·  i  invert  ·  a  ascii  ·  h  hide  ·  q  quit";
        const LEAD: u16 = 2;
        let cols = self.lava.width;
        let rows = self.lava.height / 2;
        let visual_len = TEXT.chars().count() as u16;
        if visual_len + LEAD > cols {
            return;
        }
        let pad_right = cols - visual_len - LEAD;
        let p = self.current_palette();
        let inv = self.lava.inverted;
        let (fr, fg, fb) = invert_if(p.glow(), inv);
        let (br, bg, bb) = invert_if(p.bg(), inv);
        let _ = write!(
            out,
            "\x1b[{rows};1H\x1b[38;2;{fr};{fg};{fb};48;2;{br};{bg};{bb}m{}{TEXT}{}\x1b[0m",
            " ".repeat(LEAD as usize),
            " ".repeat(pad_right as usize),
        );
    }

    /// Palette name flash, centered on screen — fires for [`OVERLAY_FRAMES`]
    /// after a palette cycle.
    fn render_palette_badge(&self, out: &mut Vec<u8>) {
        use std::io::Write;
        let p = self.current_palette();
        let label = format!(" {} ", p.name());
        let label_width = label.chars().count() as u16;
        let cols = self.lava.width;
        let rows = self.lava.height / 2;
        let row_pos = (rows / 2).max(1);
        let col_pos = (cols.saturating_sub(label_width) / 2 + 1).max(1);
        let (fr, fg, fb) = p.accent();
        let (br, bg, bb) = p.accent_bg();
        let _ = write!(
            out,
            "\x1b[{row_pos};{col_pos}H\x1b[1;38;2;{fr};{fg};{fb};48;2;{br};{bg};{bb}m{label}\x1b[0m"
        );
    }

    /// Append the next frame as RGBA pixel bytes to `out` — for canvas-style
    /// renderers that don't speak ANSI. The badge overlay is **not**
    /// rendered into the pixel buffer; callers can composite it with
    /// HTML/CSS instead (see `current_palette` + `is_overlay_active`).
    pub fn render_rgba(&self, out: &mut Vec<u8>) {
        crate::pixels::render(&self.lava, out);
    }

    /// True while the post-cycle palette badge would still be shown.
    /// Intended for non-ANSI renderers that draw the badge themselves.
    pub fn is_overlay_active(&self) -> bool {
        self.overlay_frames > 0
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.lava.resize(cols, rows);
    }

    /// Override the simulation speed multiplier. `1.0` is the engine's
    /// "natural" rate; lower → slower, ambient; higher → frenetic.
    pub fn set_speed(&mut self, speed: f32) {
        self.lava.speed = speed;
    }

    pub fn cycle_next(&mut self) {
        self.palette_idx = (self.palette_idx + 1) % Palette::ALL.len();
        self.lava.palette = Palette::ALL[self.palette_idx];
        self.overlay_frames = OVERLAY_FRAMES;
    }

    pub fn cycle_prev(&mut self) {
        self.palette_idx = (self.palette_idx + Palette::ALL.len() - 1) % Palette::ALL.len();
        self.lava.palette = Palette::ALL[self.palette_idx];
        self.overlay_frames = OVERLAY_FRAMES;
    }

    /// Flip the photographic-negative render flag.
    pub fn toggle_inverted(&mut self) {
        self.lava.inverted = !self.lava.inverted;
    }

    pub fn render_mode(&self) -> RenderMode {
        self.mode
    }

    pub fn set_render_mode(&mut self, mode: RenderMode) {
        self.mode = mode;
    }

    /// Flip between half-block and ASCII renderers.
    pub fn toggle_ascii(&mut self) {
        self.mode = match self.mode {
            RenderMode::HalfBlock => RenderMode::Ascii,
            RenderMode::Ascii => RenderMode::HalfBlock,
        };
    }

    /// Show / hide the bottom keybind strip.
    pub fn toggle_menu(&mut self) {
        self.show_menu = !self.show_menu;
    }

    pub fn is_menu_visible(&self) -> bool {
        self.show_menu
    }

    /// Heat blobs near a 1-indexed terminal cell. Accounts for the half-block
    /// double-pixel-row mapping on the y-axis.
    pub fn click(&mut self, col: u16, row: u16) {
        let x = col.saturating_sub(1) as f32 + 0.5;
        let y = row.saturating_sub(1) as f32 * 2.0 + 1.0;
        self.lava.heat(x, y, HEAT_RADIUS_PX);
    }

    /// Heat blobs near `(x, y)` in engine pixel coords (origin top-left,
    /// matching the `pixel_dimensions()` grid). For canvas/web clients that
    /// don't deal in terminal cells.
    pub fn click_pixel(&mut self, x: f32, y: f32) {
        self.lava.heat(x, y, HEAT_RADIUS_PX);
    }

    /// Apply a parsed [`Input`]. Returns `true` if the input requested exit.
    pub fn apply(&mut self, input: Input) -> bool {
        match input {
            Input::PaletteNext => self.cycle_next(),
            Input::PalettePrev => self.cycle_prev(),
            Input::ToggleInverted => self.toggle_inverted(),
            Input::ToggleAscii => self.toggle_ascii(),
            Input::ToggleMenu => self.toggle_menu(),
            Input::Click { col, row } => self.click(col, row),
            Input::Exit => return true,
        }
        false
    }

    /// Parse + apply a chunk of raw input bytes. Convenience for transports
    /// that just want to forward bytes from the client. Returns `true` if
    /// exit was requested.
    pub fn feed_input(&mut self, data: &[u8]) -> bool {
        parse_input(data).map(|i| self.apply(i)).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_arrow_keys() {
        assert_eq!(parse_input(b"\x1b[C"), Some(Input::PaletteNext));
        assert_eq!(parse_input(b"\x1b[D"), Some(Input::PalettePrev));
        assert_eq!(parse_input(b"\x1bOC"), Some(Input::PaletteNext));
        assert_eq!(parse_input(b"\x1bOD"), Some(Input::PalettePrev));
    }

    #[test]
    fn parse_ctrl_c_and_d() {
        assert_eq!(parse_input(b"\x03"), Some(Input::Exit));
        assert_eq!(parse_input(b"\x04"), Some(Input::Exit));
    }

    #[test]
    fn parse_q_quits() {
        assert_eq!(parse_input(b"q"), Some(Input::Exit));
        assert_eq!(parse_input(b"Q"), Some(Input::Exit));
    }

    #[test]
    fn parse_sgr_mouse_press_left_button_only() {
        assert_eq!(
            parse_input(b"\x1b[<0;42;15M"),
            Some(Input::Click { col: 42, row: 15 })
        );
        // Non-left buttons are ignored.
        assert_eq!(parse_input(b"\x1b[<1;42;15M"), None);
        assert_eq!(parse_input(b"\x1b[<2;42;15M"), None);
        // Release ('m' lowercase) is ignored.
        assert_eq!(parse_input(b"\x1b[<0;42;15m"), None);
    }

    #[test]
    fn palette_from_str_resolves_aliases_and_falls_back() {
        assert_eq!(Session::palette_from_str("uv"), Palette::Ultraviolet);
        assert_eq!(Session::palette_from_str("PINK"), Palette::Bubblegum);
        assert_eq!(Session::palette_from_str("nonsense"), Palette::default());
    }

    #[test]
    fn cycle_advances_palette_and_renders_centered_badge() {
        let mut s = Session::new(40, 20, Palette::Classic);
        let initial = s.current_palette();
        s.cycle_next();
        assert_ne!(s.current_palette(), initial);
        let mut buf = Vec::new();
        s.render(&mut buf);
        let bytes = std::str::from_utf8(&buf).unwrap();
        // Badge is centered vertically — for 20 rows, that's row 10.
        assert!(
            bytes.contains("\x1b[10;"),
            "expected centered badge positioning"
        );
        // And the new palette name should appear (inside the badge label).
        let new_name = s.current_palette().name();
        assert!(
            bytes.contains(new_name),
            "expected palette name {new_name:?} in output"
        );
    }

    #[test]
    fn render_includes_keybind_strip_by_default() {
        let s = Session::new(80, 24, Palette::Classic);
        let mut buf = Vec::new();
        s.render(&mut buf);
        let bytes = std::str::from_utf8(&buf).unwrap();
        assert!(bytes.contains("palette"));
        assert!(bytes.contains("hide"));
    }

    #[test]
    fn toggle_menu_hides_keybind_strip() {
        let mut s = Session::new(80, 24, Palette::Classic);
        s.toggle_menu();
        let mut buf = Vec::new();
        s.render(&mut buf);
        let bytes = std::str::from_utf8(&buf).unwrap();
        assert!(!bytes.contains("hide"));
    }

    #[test]
    fn h_key_toggles_menu_via_feed_input() {
        let mut s = Session::new(80, 24, Palette::Classic);
        assert!(s.is_menu_visible());
        assert!(!s.feed_input(b"h"));
        assert!(!s.is_menu_visible());
        assert!(!s.feed_input(b"H"));
        assert!(s.is_menu_visible());
    }

    #[test]
    fn feed_input_routes_clicks_to_heat() {
        let mut s = Session::new(40, 20, Palette::Classic);
        // Click should not panic and should not request exit.
        assert!(!s.feed_input(b"\x1b[<0;10;5M"));
    }

    #[test]
    fn feed_input_returns_true_on_exit() {
        let mut s = Session::new(40, 20, Palette::Classic);
        assert!(s.feed_input(b"\x03"));
    }
}
