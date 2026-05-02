//! Metaball simulation. The engine's core type: [`Lava`].
//!
//! Pure Rust, no I/O, no platform deps — compiles cleanly to wasm32.

use crate::palette::Palette;
use crate::rng::Rng;

const G: f32 = 28.0; // buoyancy magnitude
const HEAT_RATE: f32 = 0.55; // temp change per second at extremes
const DAMPING: f32 = 0.985;
const VEL_SCALE: f32 = 10.0; // converts "velocity units" into pixels/sec at dt
const RESTITUTION: f32 = 0.55;

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

/// Public, user-facing configuration. Construct with struct-literal syntax
/// using `..Default::default()` to override only what you need.
#[derive(Clone, Debug)]
pub struct Config {
    pub palette: Palette,
    pub blob_count: u32,
    /// Simulation speed multiplier — 1.0 is normal.
    pub speed: f32,
    pub seed: u64,
}

impl Default for Config {
    #[allow(clippy::unusual_byte_groupings)] // seed spells "coffee food"
    fn default() -> Self {
        Self {
            palette: Palette::Classic,
            blob_count: 7,
            speed: 0.8,
            seed: 0xC0FFEE_F00D,
        }
    }
}

pub struct Lava {
    /// Pixel width — equal to terminal columns.
    pub width: u16,
    /// Pixel height — equal to 2 × terminal rows (half-block doubling).
    pub height: u16,
    pub blobs: Vec<Blob>,
    pub time: f32,
    pub palette: Palette,
    pub speed: f32,
    /// When true, every rendered pixel's RGB is bit-flipped (255 − channel).
    pub inverted: bool,
}

impl Lava {
    /// Construct with default config. Same as `with_config(_, _, Config::default())`.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::with_config(cols, rows, Config::default())
    }

    pub fn with_config(cols: u16, rows: u16, config: Config) -> Self {
        let width = cols.max(1);
        let height = rows.max(1).saturating_mul(2);
        let mut rng = Rng::new(config.seed);
        let blob_count = config.blob_count.max(1) as usize;
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
        Self {
            width,
            height,
            blobs,
            time: 0.0,
            palette: config.palette,
            speed: config.speed,
            inverted: false,
        }
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
        let dt = dt * self.speed;
        self.time += dt;
        let w = self.width as f32;
        let h = self.height as f32;

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
            if b.x < m {
                b.x = m;
                b.vx = b.vx.abs() * RESTITUTION;
            }
            if b.x > w - m {
                b.x = w - m;
                b.vx = -b.vx.abs() * RESTITUTION;
            }
            if b.y < m {
                b.y = m;
                b.vy = b.vy.abs() * RESTITUTION;
            }
            if b.y > h - m {
                b.y = h - m;
                b.vy = -b.vy.abs() * RESTITUTION;
            }
        }
    }

    /// Bump the temperature of all blobs within `radius` pixels of `(x, y)`,
    /// with a distance-based falloff (closer blobs get more heat). Saturates
    /// at `temp = 1.0`. Coordinates are in the engine's pixel space — the
    /// canvas is `width` × `height` (where `height = 2 × terminal rows`).
    pub fn heat(&mut self, x: f32, y: f32, radius: f32) {
        if radius <= 0.0 {
            return;
        }
        let r2 = radius * radius;
        for b in &mut self.blobs {
            let dx = b.x - x;
            let dy = b.y - y;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 {
                let falloff = 1.0 - (d2 / r2).sqrt();
                b.temp = (b.temp + falloff).min(1.0);
            }
        }
    }

    /// Sample the metaball field and the temperature-weighted heat at a point.
    /// Used by renderers; not generally useful to call directly.
    pub(crate) fn sample(&self, x: f32, y: f32) -> (f32, f32) {
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
        let heat = if weight_sum > 0.0 {
            heat_acc / weight_sum
        } else {
            0.0
        };
        (field, heat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_preserves_blob_count() {
        let mut lava = Lava::with_config(
            40,
            20,
            Config {
                blob_count: 5,
                ..Default::default()
            },
        );
        let n = lava.blobs.len();
        lava.resize(80, 40);
        assert_eq!(lava.blobs.len(), n);
        assert_eq!(lava.width, 80);
        assert_eq!(lava.height, 80);
    }

    #[test]
    fn deterministic_for_same_seed() {
        let cfg = Config {
            blob_count: 5,
            seed: 99,
            ..Default::default()
        };
        let a = Lava::with_config(40, 20, cfg.clone());
        let b = Lava::with_config(40, 20, cfg);
        for (ba, bb) in a.blobs.iter().zip(b.blobs.iter()) {
            assert_eq!(ba.x.to_bits(), bb.x.to_bits());
            assert_eq!(ba.y.to_bits(), bb.y.to_bits());
        }
    }

    #[test]
    fn speed_scales_simulation() {
        let cfg_slow = Config {
            speed: 0.0,
            blob_count: 3,
            seed: 1,
            ..Default::default()
        };
        let cfg_fast = Config {
            speed: 2.0,
            blob_count: 3,
            seed: 1,
            ..Default::default()
        };
        let mut a = Lava::with_config(40, 20, cfg_slow);
        let mut b = Lava::with_config(40, 20, cfg_fast);
        for _ in 0..30 {
            a.step(1.0 / 30.0);
            b.step(1.0 / 30.0);
        }
        assert_eq!(a.time, 0.0);
        assert!(b.time > 0.0);
    }
}
