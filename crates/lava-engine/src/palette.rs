//! Named color palettes — the user-facing "vibe" knob.
//!
//! Each [`Palette`] resolves to a fixed set of color stops ([`PaletteColors`])
//! that the renderer interpolates across based on metaball field intensity
//! and local heat. To add a new palette: extend the enum, add a name + alias
//! in `FromStr`, and add a row to [`Palette::colors`].

use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Palette {
    #[default]
    Classic,
    Toxic,
    Bubblegum,
    Mono,
    Aurora,
    Ocean,
    Blood,
    Ultraviolet,
}

impl Palette {
    pub const ALL: &'static [Palette] = &[
        Palette::Classic,
        Palette::Toxic,
        Palette::Bubblegum,
        Palette::Mono,
        Palette::Aurora,
        Palette::Ocean,
        Palette::Blood,
        Palette::Ultraviolet,
    ];

    pub fn name(&self) -> &'static str {
        self.input_names()[0]
    }

    /// A single representative RGB swatch for this palette — the mid-tier
    /// "warm" color, useful for rendering the palette's name in its own
    /// color (e.g. in a help listing).
    pub fn accent(&self) -> (u8, u8, u8) {
        let (r, g, b) = self.colors().warm;
        (r as u8, g as u8, b as u8)
    }

    /// A complementary background swatch for this palette — the deep
    /// "cool" color, paired with [`Palette::accent`] for badge-style UI
    /// (palette name on a dark themed background).
    pub fn accent_bg(&self) -> (u8, u8, u8) {
        let (r, g, b) = self.colors().cool;
        (r as u8, g as u8, b as u8)
    }

    /// All accepted input strings for this palette, lowercase. The first
    /// entry is the canonical name; the rest are aliases. This is the single
    /// source of truth used by both [`FromStr`] and any consumer that needs
    /// to enumerate names (e.g. the help doc served by `lava-ssh`).
    pub fn input_names(&self) -> &'static [&'static str] {
        match self {
            Palette::Classic => &["classic", "lava", "default"],
            Palette::Toxic => &["toxic", "green", "radioactive"],
            Palette::Bubblegum => &["bubblegum", "pink", "magenta"],
            Palette::Mono => &["mono", "gray", "grey"],
            Palette::Aurora => &["aurora", "northern", "borealis"],
            Palette::Ocean => &["ocean", "cobalt", "electric", "ice"],
            Palette::Blood => &["blood", "crimson", "gore"],
            Palette::Ultraviolet => &["ultraviolet", "uv", "blacklight"],
        }
    }

    pub(crate) fn colors(&self) -> PaletteColors {
        match self {
            Palette::Classic => PaletteColors {
                bg_top: (10.0, 6.0, 22.0),
                bg_bot: (22.0, 10.0, 32.0),
                glow: (150.0, 35.0, 45.0),
                cool: (115.0, 22.0, 30.0),
                warm: (255.0, 95.0, 30.0),
                hot: (255.0, 230.0, 95.0),
            },
            Palette::Toxic => PaletteColors {
                bg_top: (8.0, 18.0, 8.0),
                bg_bot: (12.0, 28.0, 14.0),
                glow: (60.0, 120.0, 30.0),
                cool: (40.0, 90.0, 25.0),
                warm: (130.0, 220.0, 50.0),
                hot: (220.0, 255.0, 120.0),
            },
            Palette::Bubblegum => PaletteColors {
                bg_top: (15.0, 5.0, 25.0),
                bg_bot: (28.0, 10.0, 35.0),
                glow: (130.0, 40.0, 90.0),
                cool: (110.0, 30.0, 80.0),
                warm: (240.0, 90.0, 150.0),
                hot: (255.0, 200.0, 180.0),
            },
            Palette::Mono => PaletteColors {
                bg_top: (8.0, 8.0, 10.0),
                bg_bot: (18.0, 18.0, 22.0),
                glow: (60.0, 60.0, 70.0),
                cool: (90.0, 90.0, 100.0),
                warm: (180.0, 180.0, 190.0),
                hot: (240.0, 240.0, 245.0),
            },
            // Dark teal/grey background, deep teal → bright cyan → soft violet.
            Palette::Aurora => PaletteColors {
                bg_top: (6.0, 14.0, 16.0),
                bg_bot: (10.0, 20.0, 22.0),
                glow: (25.0, 90.0, 80.0),
                cool: (20.0, 80.0, 100.0),
                warm: (70.0, 210.0, 160.0),
                hot: (190.0, 140.0, 255.0),
            },
            // Near-black navy background, dark cobalt → electric blue → icy white.
            Palette::Ocean => PaletteColors {
                bg_top: (4.0, 7.0, 20.0),
                bg_bot: (7.0, 12.0, 30.0),
                glow: (18.0, 55.0, 150.0),
                cool: (12.0, 40.0, 120.0),
                warm: (55.0, 130.0, 255.0),
                hot: (195.0, 225.0, 255.0),
            },
            // Dark maroon background, lava barely distinguishable at rest, bright red when hot.
            Palette::Blood => PaletteColors {
                bg_top: (14.0, 3.0, 3.0),
                bg_bot: (20.0, 4.0, 4.0),
                glow: (48.0, 8.0, 8.0),
                cool: (75.0, 10.0, 10.0),
                warm: (170.0, 22.0, 22.0),
                hot: (230.0, 45.0, 45.0),
            },
            // Dark purple background, near-invisible lava at rest, bright violet → pale white when hot.
            Palette::Ultraviolet => PaletteColors {
                bg_top: (10.0, 4.0, 18.0),
                bg_bot: (16.0, 6.0, 28.0),
                glow: (55.0, 12.0, 100.0),
                cool: (35.0, 5.0, 70.0),
                warm: (150.0, 40.0, 255.0),
                hot: (225.0, 195.0, 255.0),
            },
        }
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
pub(crate) fn pixel_color(pal: &PaletteColors, field: f32, heat: f32, v: f32) -> (u8, u8, u8) {
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
}
