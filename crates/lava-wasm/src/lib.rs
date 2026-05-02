//! WebAssembly bindings for [`lava_engine::Session`]. Built with `wasm-pack`
//! (`script/build-wasm`); the JS shim + `.wasm` blob get embedded into the
//! `lava-web` static bundle via `include_bytes!`.
//!
//! There are two output paths the browser side can pick:
//!
//! - **`render`** — the full ANSI frame, suitable for terminal emulators
//!   like xterm.js. (Includes the post-cycle overlay.)
//! - **`renderRgba`** — raw RGBA bytes at the engine's pixel resolution,
//!   suitable for `ctx.putImageData()` on a `<canvas>`. (No overlay; the
//!   host composites a badge in HTML/CSS instead.)
//!
//! Canvas-style usage:
//!
//! ```js
//! import init, { LavaSession } from "./lava_wasm.js";
//! await init();
//! const session = new LavaSession(800, 300, "ultraviolet");
//! const [w, h] = session.pixelDimensions();
//! const canvas = document.querySelector("canvas");
//! canvas.width = w; canvas.height = h;
//! const ctx = canvas.getContext("2d");
//! const tick = () => {
//!   session.tick(1 / 60);
//!   const rgba = session.renderRgba();
//!   ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba), w, h), 0, 0);
//!   requestAnimationFrame(tick);
//! };
//! requestAnimationFrame(tick);
//! ```

use lava_engine::Session;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct LavaSession {
    inner: Session,
    buf: Vec<u8>,
}

#[wasm_bindgen]
impl LavaSession {
    /// Construct a session sized to `cols × rows`. The engine's internal
    /// pixel grid is `cols` wide and `2 × rows` tall (half-block trick) —
    /// canvas hosts can pass the canvas pixel dimensions as `(w, h/2)` to
    /// match exactly. `palette` is matched via `Palette`'s `FromStr`
    /// (case-insensitive, aliases supported); unknown / `null` falls back
    /// to the default.
    #[wasm_bindgen(constructor)]
    pub fn new(cols: u16, rows: u16, palette: Option<String>) -> Self {
        let palette = palette
            .as_deref()
            .map(Session::palette_from_str)
            .unwrap_or_default();
        Self {
            inner: Session::new(cols, rows, palette),
            buf: Vec::new(),
        }
    }

    /// Advance the simulation by `dt` seconds. Decrements the overlay
    /// countdown if active.
    pub fn tick(&mut self, dt: f32) {
        self.inner.tick(dt);
    }

    /// Render the next ANSI frame and return the bytes (with overlay).
    pub fn render(&mut self) -> Vec<u8> {
        self.buf.clear();
        self.inner.render(&mut self.buf);
        self.buf.clone()
    }

    /// Render the next frame as raw RGBA pixels at the engine's pixel
    /// resolution — `pixelDimensions()` × 4 bytes. No overlay; let the
    /// host paint that on top in HTML/CSS.
    #[wasm_bindgen(js_name = renderRgba)]
    pub fn render_rgba(&mut self) -> Vec<u8> {
        self.buf.clear();
        self.inner.render_rgba(&mut self.buf);
        self.buf.clone()
    }

    /// Feed a chunk of input bytes from xterm.js (`onData`). Returns `true`
    /// if the input requested exit (Ctrl-C / Ctrl-D). Canvas hosts that
    /// don't speak terminal escapes can use [`cycle_next`], [`cycle_prev`],
    /// and [`click_pixel`] instead.
    #[wasm_bindgen(js_name = feedInput)]
    pub fn feed_input(&mut self, data: &[u8]) -> bool {
        self.inner.feed_input(data)
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.inner.resize(cols, rows);
    }

    #[wasm_bindgen(js_name = cycleNext)]
    pub fn cycle_next(&mut self) {
        self.inner.cycle_next();
    }

    #[wasm_bindgen(js_name = cyclePrev)]
    pub fn cycle_prev(&mut self) {
        self.inner.cycle_prev();
    }

    #[wasm_bindgen(js_name = toggleInverted)]
    pub fn toggle_inverted(&mut self) {
        self.inner.toggle_inverted();
    }

    /// Heat blobs near `(x, y)` in engine pixel coordinates. For canvas
    /// hosts: convert from canvas pixel coords with the same `(w, h)`
    /// returned by `pixelDimensions()`.
    #[wasm_bindgen(js_name = clickPixel)]
    pub fn click_pixel(&mut self, x: f32, y: f32) {
        self.inner.click_pixel(x, y);
    }

    #[wasm_bindgen(js_name = pixelDimensions)]
    pub fn pixel_dimensions(&self) -> Vec<u16> {
        let (w, h) = self.inner.pixel_dimensions();
        vec![w, h]
    }

    #[wasm_bindgen(js_name = currentPaletteName)]
    pub fn current_palette_name(&self) -> String {
        self.inner.current_palette().name().to_string()
    }

    /// Returns `[r, g, b]` for the current palette's accent color (the
    /// "warm" stop) — for styling a badge / cursor / etc. in HTML/CSS.
    #[wasm_bindgen(js_name = currentPaletteAccent)]
    pub fn current_palette_accent(&self) -> Vec<u8> {
        let (r, g, b) = self.inner.current_palette().accent();
        vec![r, g, b]
    }

    /// Returns `[r, g, b]` for the current palette's accent-background
    /// color (the deep "cool" stop) — pairs with the accent for badge fill.
    #[wasm_bindgen(js_name = currentPaletteAccentBg)]
    pub fn current_palette_accent_bg(&self) -> Vec<u8> {
        let (r, g, b) = self.inner.current_palette().accent_bg();
        vec![r, g, b]
    }

    #[wasm_bindgen(js_name = isOverlayActive)]
    pub fn is_overlay_active(&self) -> bool {
        self.inner.is_overlay_active()
    }
}
