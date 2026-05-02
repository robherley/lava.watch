//! Named color palettes — the user-facing "vibe" knob.
//!
//! Palette **data** lives in `palettes.toml` at the crate root; `build.rs`
//! turns it into the [`Palette`] enum, [`Palette::ALL`], [`Palette::input_names`],
//! and [`Palette::colors`] (`include!`'d below). Adding a palette = appending
//! a TOML entry, no hand-edits here.

use std::str::FromStr;

include!(concat!(env!("OUT_DIR"), "/palettes_generated.rs"));

impl Palette {
    pub fn name(&self) -> &'static str {
        self.input_names()[0]
    }

    /// A single representative RGB swatch for this palette — the mid-tier
    /// "warm" color, useful for rendering the palette's name in its own
    /// color (help listing, badge fg, etc.).
    pub fn accent(&self) -> (u8, u8, u8) {
        let (r, g, b) = self.colors().warm;
        (r as u8, g as u8, b as u8)
    }

    /// A complementary background swatch — the deep "cool" color, paired
    /// with [`Palette::accent`] for badge-style UI.
    pub fn accent_bg(&self) -> (u8, u8, u8) {
        let (r, g, b) = self.colors().cool;
        (r as u8, g as u8, b as u8)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePaletteError;

impl std::fmt::Display for ParsePaletteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown palette (try: ")?;
        for (i, p) in Palette::ALL.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            f.write_str(p.name())?;
        }
        write!(f, ")")
    }
}

impl std::error::Error for ParsePaletteError {}

impl FromStr for Palette {
    type Err = ParsePaletteError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_ascii_lowercase();
        for p in Palette::ALL {
            if p.input_names().contains(&s.as_str()) {
                return Ok(*p);
            }
        }
        Err(ParsePaletteError)
    }
}

/// Color stops a renderer interpolates between based on field intensity and
/// local heat. Crate-private — external callers should use [`Palette`] and
/// let the renderer resolve colors internally.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PaletteColors {
    pub bg_top: (f32, f32, f32),
    pub bg_bot: (f32, f32, f32),
    pub glow: (f32, f32, f32),
    pub cool: (f32, f32, f32),
    pub warm: (f32, f32, f32),
    pub hot: (f32, f32, f32),
}

/// Map (palette colors, field intensity, local heat, vertical position
/// `v ∈ [0, 1]`) → RGB. Shared by every renderer (ANSI, RGBA pixels, …).
/// `inverted` flips every channel (255 − c) for the photographic-negative
/// effect.
pub(crate) fn pixel_color(
    pal: &PaletteColors,
    field: f32,
    heat: f32,
    v: f32,
    inverted: bool,
) -> (u8, u8, u8) {
    let finish = |c: (u8, u8, u8)| {
        if inverted {
            (255 - c.0, 255 - c.1, 255 - c.2)
        } else {
            c
        }
    };

    let bg = lerp3(pal.bg_top, pal.bg_bot, v.clamp(0.0, 1.0));

    if field < 0.55 {
        return finish(rgb(bg));
    }

    if field < 1.0 {
        let g = (field - 0.55) / 0.45;
        let glow = lerp3(bg, pal.glow, g * 0.55);
        return finish(rgb(glow));
    }

    let h = heat.clamp(0.0, 1.0);
    let body = if h < 0.5 {
        lerp3(pal.cool, pal.warm, h * 2.0)
    } else {
        lerp3(pal.warm, pal.hot, (h - 0.5) * 2.0)
    };
    let boost = ((field - 1.0) * 0.25).clamp(0.0, 0.4);
    finish(rgb((
        (body.0 * (1.0 + boost)).min(255.0),
        (body.1 * (1.0 + boost)).min(255.0),
        (body.2 * (1.0 + boost)).min(255.0),
    )))
}

/// Component-wise linear interpolation between two RGB triples.
fn lerp3(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

/// Truncate a float RGB triple to `u8` channels. Saturating at the integer
/// bounds (Rust's `f32 as u8` semantics).
fn rgb(c: (f32, f32, f32)) -> (u8, u8, u8) {
    (c.0 as u8, c.1 as u8, c.2 as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_via_from_str() {
        for p in Palette::ALL {
            assert_eq!(p.name().parse::<Palette>().unwrap(), *p);
        }
        assert_eq!("ice".parse::<Palette>().unwrap(), Palette::Ocean);
        assert_eq!("GREEN".parse::<Palette>().unwrap(), Palette::Toxic);
        assert!("not-a-palette".parse::<Palette>().is_err());
    }

    #[test]
    fn all_palettes_listed() {
        // Sanity — palettes.toml has 8 entries; if you add/remove, update here.
        assert_eq!(Palette::ALL.len(), 8);
    }
}
