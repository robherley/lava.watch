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
    Ocean,
    Toxic,
    Bubblegum,
    Mono,
}

impl Palette {
    pub const ALL: &'static [Palette] = &[
        Palette::Classic,
        Palette::Ocean,
        Palette::Toxic,
        Palette::Bubblegum,
        Palette::Mono,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Palette::Classic => "classic",
            Palette::Ocean => "ocean",
            Palette::Toxic => "toxic",
            Palette::Bubblegum => "bubblegum",
            Palette::Mono => "mono",
        }
    }

    pub(crate) fn colors(&self) -> PaletteColors {
        match self {
            Palette::Classic => PaletteColors {
                bg_top: (10.0, 6.0, 22.0),
                bg_bot: (22.0, 10.0, 32.0),
                glow:   (150.0, 35.0, 45.0),
                cool:   (115.0, 22.0, 30.0),
                warm:   (255.0, 95.0, 30.0),
                hot:    (255.0, 230.0, 95.0),
            },
            Palette::Ocean => PaletteColors {
                bg_top: (5.0, 10.0, 25.0),
                bg_bot: (10.0, 18.0, 38.0),
                glow:   (30.0, 90.0, 130.0),
                cool:   (15.0, 60.0, 100.0),
                warm:   (60.0, 180.0, 220.0),
                hot:    (200.0, 240.0, 255.0),
            },
            Palette::Toxic => PaletteColors {
                bg_top: (8.0, 18.0, 8.0),
                bg_bot: (12.0, 28.0, 14.0),
                glow:   (60.0, 120.0, 30.0),
                cool:   (40.0, 90.0, 25.0),
                warm:   (130.0, 220.0, 50.0),
                hot:    (220.0, 255.0, 120.0),
            },
            Palette::Bubblegum => PaletteColors {
                bg_top: (15.0, 5.0, 25.0),
                bg_bot: (28.0, 10.0, 35.0),
                glow:   (130.0, 40.0, 90.0),
                cool:   (110.0, 30.0, 80.0),
                warm:   (240.0, 90.0, 150.0),
                hot:    (255.0, 200.0, 180.0),
            },
            Palette::Mono => PaletteColors {
                bg_top: (8.0, 8.0, 10.0),
                bg_bot: (18.0, 18.0, 22.0),
                glow:   (60.0, 60.0, 70.0),
                cool:   (90.0, 90.0, 100.0),
                warm:   (180.0, 180.0, 190.0),
                hot:    (240.0, 240.0, 245.0),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePaletteError;

impl std::fmt::Display for ParsePaletteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown palette (try: classic, ocean, toxic, sunset, mono)")
    }
}

impl std::error::Error for ParsePaletteError {}

impl FromStr for Palette {
    type Err = ParsePaletteError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "classic" | "lava" | "default" => Ok(Palette::Classic),
            "ocean" | "blue" | "deep" => Ok(Palette::Ocean),
            "toxic" | "green" | "radioactive" => Ok(Palette::Toxic),
            "bubblegum" | "pink" | "magenta" => Ok(Palette::Bubblegum),
            "mono" | "gray" | "grey" => Ok(Palette::Mono),
            _ => Err(ParsePaletteError),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_via_from_str() {
        for p in Palette::ALL {
            assert_eq!(p.name().parse::<Palette>().unwrap(), *p);
        }
        assert_eq!("blue".parse::<Palette>().unwrap(), Palette::Ocean);
        assert_eq!("GREEN".parse::<Palette>().unwrap(), Palette::Toxic);
        assert!("not-a-palette".parse::<Palette>().is_err());
    }
}
