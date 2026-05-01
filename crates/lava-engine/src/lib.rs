//! Headless lava lamp engine: metaball simulation + ANSI half-block renderer.
//!
//! The engine is pure Rust with no I/O and no platform deps, so it compiles
//! cleanly to wasm32 and is shared by the SSH and browser transports.
//!
//! Render output is a sequence of ANSI escape codes plus the half-block
//! character `▀` — each terminal cell encodes two vertically stacked pixels
//! (foreground = top pixel, background = bottom pixel). This doubles the
//! effective vertical resolution at no cost in cells written.

use std::io::Write;

mod rng;
pub use rng::Rng;

#[derive(Clone, Debug)]
pub struct Blob {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub radius: f32,
    /// 0.0 = cold (sinks), 1.0 = hot (rises).
    pub temp: f32,
}

pub struct LavaLamp {
    /// Pixel width — equal to terminal columns.
    pub width: u16,
    /// Pixel height — equal to 2 × terminal rows (half-block doubling).
    pub height: u16,
    pub blobs: Vec<Blob>,
    pub time: f32,
}

impl LavaLamp {
    pub fn new(cols: u16, rows: u16, blob_count: usize, seed: u64) -> Self {
        let width = cols.max(1);
        let height = rows.max(1).saturating_mul(2);
        let mut rng = Rng::new(seed);
        let mut blobs = Vec::with_capacity(blob_count);
        let w = width as f32;
        let h = height as f32;
        for _ in 0..blob_count {
            blobs.push(Blob {
                x: rng.f32_range(w * 0.2, w * 0.8),
                y: rng.f32_range(h * 0.2, h * 0.8),
                vx: rng.f32_range(-2.0, 2.0),
                vy: rng.f32_range(-2.0, 2.0),
                radius: rng.f32_range(w * 0.06, w * 0.12).max(2.5),
                temp: rng.f32_unit(),
            });
        }
        Self { width, height, blobs, time: 0.0 }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let new_w = cols.max(1);
        let new_h = rows.max(1).saturating_mul(2);
        let sx = new_w as f32 / self.width as f32;
        let sy = new_h as f32 / self.height as f32;
        let r_scale = (sx + sy) * 0.5;
        for b in &mut self.blobs {
            b.x *= sx;
            b.y *= sy;
            b.radius *= r_scale;
        }
        self.width = new_w;
        self.height = new_h;
    }

    pub fn step(&mut self, dt: f32) {
        self.time += dt;
        let w = self.width as f32;
        let h = self.height as f32;

        // Tunables — picked by feel.
        const G: f32 = 28.0;        // buoyancy magnitude
        const HEAT_RATE: f32 = 0.55; // temp change per second at extremes
        const DAMPING: f32 = 0.985;
        const VEL_SCALE: f32 = 10.0; // converts "velocity units" into pixels/sec at dt
        const RESTITUTION: f32 = 0.55;

        for b in &mut self.blobs {
            // Heat exchange near top/bottom — hot at the bottom, cool at top.
            if b.y > h * 0.82 {
                b.temp = (b.temp + HEAT_RATE * dt).min(1.0);
            }
            if b.y < h * 0.18 {
                b.temp = (b.temp - HEAT_RATE * dt).max(0.0);
            }

            // Buoyancy: temp=1 → upward, temp=0 → downward.
            let ay = G * (0.5 - b.temp);
            b.vy += ay * dt;

            // Gentle horizontal drift so blobs don't track straight up/down.
            let jitter = ((self.time * 0.6) + (b.x * 0.04) + (b.y * 0.03)).sin() * 1.4;
            b.vx += jitter * dt;

            b.vx *= DAMPING;
            b.vy *= DAMPING;

            b.x += b.vx * dt * VEL_SCALE;
            b.y += b.vy * dt * VEL_SCALE;

            // Soft walls — keep blob centers a fraction of the radius inside the bounds.
            let m = b.radius * 0.5;
            if b.x < m { b.x = m; b.vx = b.vx.abs() * RESTITUTION; }
            if b.x > w - m { b.x = w - m; b.vx = -b.vx.abs() * RESTITUTION; }
            if b.y < m { b.y = m; b.vy = b.vy.abs() * RESTITUTION; }
            if b.y > h - m { b.y = h - m; b.vy = -b.vy.abs() * RESTITUTION; }
        }
    }

    /// Sample the metaball field and the temperature-weighted heat at a point.
    fn sample(&self, x: f32, y: f32) -> (f32, f32) {
        let mut field = 0.0f32;
        let mut heat_acc = 0.0f32;
        let mut weight_sum = 0.0f32;
        for b in &self.blobs {
            let dx = x - b.x;
            let dy = y - b.y;
            let d2 = dx * dx + dy * dy + 0.5;
            let w = (b.radius * b.radius) / d2;
            field += w;
            heat_acc += w * b.temp;
            weight_sum += w;
        }
        let heat = if weight_sum > 0.0 { heat_acc / weight_sum } else { 0.0 };
        (field, heat)
    }

    /// Append a full ANSI frame to `out`. The frame begins with cursor-home
    /// (`ESC[H`) so successive frames overwrite in place — the caller is
    /// responsible for the initial screen clear (see `ENTER_ALT_SCREEN`).
    pub fn render_ansi(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1b[H");

        let cols = self.width as usize;
        let rows = (self.height / 2) as usize;
        let h = self.height as f32;

        let mut last_fg: Option<(u8, u8, u8)> = None;
        let mut last_bg: Option<(u8, u8, u8)> = None;

        for r in 0..rows {
            for c in 0..cols {
                let xt = c as f32 + 0.5;
                let yt = (r * 2) as f32 + 0.5;
                let yb = (r * 2 + 1) as f32 + 0.5;
                let (ft, ht) = self.sample(xt, yt);
                let (fb, hb) = self.sample(xt, yb);
                let top = pixel_color(ft, ht, yt / h);
                let bot = pixel_color(fb, hb, yb / h);

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
}

/// Map (field intensity, local heat, vertical position v∈[0,1]) → RGB.
fn pixel_color(field: f32, heat: f32, v: f32) -> (u8, u8, u8) {
    // Background: deep purple at top → near-black at bottom. Adds a sense of
    // depth and frames the lava against the lamp's interior.
    let bg = lerp3((10.0, 6.0, 22.0), (22.0, 10.0, 32.0), v.clamp(0.0, 1.0));

    if field < 0.55 {
        return rgb(bg);
    }

    // Outer glow — soft red bleed where the field is near the surface.
    if field < 1.0 {
        let g = (field - 0.55) / 0.45;
        let glow = lerp3(bg, (150.0, 35.0, 45.0), g * 0.55);
        return rgb(glow);
    }

    // Lava body — color by local heat, with a brightness boost in dense cores.
    let cool = (115.0, 22.0, 30.0);  // dark red
    let warm = (255.0, 95.0, 30.0);  // orange
    let hot = (255.0, 230.0, 95.0);  // bright yellow
    let h = heat.clamp(0.0, 1.0);
    let body = if h < 0.5 {
        lerp3(cool, warm, h * 2.0)
    } else {
        lerp3(warm, hot, (h - 0.5) * 2.0)
    };
    let boost = ((field - 1.0) * 0.25).clamp(0.0, 0.4);
    rgb((
        (body.0 * (1.0 + boost)).min(255.0),
        (body.1 * (1.0 + boost)).min(255.0),
        (body.2 * (1.0 + boost)).min(255.0),
    ))
}

fn lerp3(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

fn rgb(c: (f32, f32, f32)) -> (u8, u8, u8) {
    (c.0 as u8, c.1 as u8, c.2 as u8)
}

/// Switch to the alt screen, hide cursor, clear, home — write once on entry.
pub const ENTER_ALT_SCREEN: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H";
/// Show cursor, leave the alt screen — write once on exit.
pub const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?25h\x1b[?1049l";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_frame_with_truecolor_and_half_blocks() {
        let mut lamp = LavaLamp::new(40, 20, 6, 1234);
        for _ in 0..30 {
            lamp.step(1.0 / 30.0);
        }
        let mut buf = Vec::new();
        lamp.render_ansi(&mut buf);
        // Non-empty, contains a truecolor fg escape and the half-block glyph.
        assert!(!buf.is_empty());
        let s = std::str::from_utf8(&buf).expect("ansi output is utf-8");
        assert!(s.contains("\x1b[38;2;"), "expected truecolor fg escape");
        assert!(s.contains("\x1b[48;2;"), "expected truecolor bg escape");
        assert!(s.contains('▀'), "expected half-block character");
        assert!(s.starts_with("\x1b[H"), "expected leading cursor-home");
    }

    #[test]
    fn resize_preserves_blob_count() {
        let mut lamp = LavaLamp::new(40, 20, 5, 42);
        let n = lamp.blobs.len();
        lamp.resize(80, 40);
        assert_eq!(lamp.blobs.len(), n);
        assert_eq!(lamp.width, 80);
        assert_eq!(lamp.height, 80);
    }

    #[test]
    fn deterministic_for_same_seed() {
        let a = LavaLamp::new(40, 20, 5, 99);
        let b = LavaLamp::new(40, 20, 5, 99);
        for (ba, bb) in a.blobs.iter().zip(b.blobs.iter()) {
            assert_eq!(ba.x.to_bits(), bb.x.to_bits());
            assert_eq!(ba.y.to_bits(), bb.y.to_bits());
        }
    }
}
