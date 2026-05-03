//! ANSI help text shared by every transport.
//!
//! Lists palette names (each rendered in its own color), interactive keybinds,
//! and an "example" line. Only the tagline and example string are
//! transport-specific (`ssh uv@lava.watch` vs `npx lava-watch uv`), so they're
//! parameters; the keybinds and palette table are universal.

use crate::Palette;
use std::fmt::Write as _;

/// Build the help text bytes. Intended for direct write to a TTY (uses
/// `\r\n`, which is correct under raw mode and harmless otherwise).
pub fn help_text(tagline: &str, example: &str) -> Vec<u8> {
    let mut s = String::new();
    let width = Palette::ALL
        .iter()
        .map(|p| p.name().len())
        .max()
        .unwrap_or(0);
    write!(s, "\r\n  {tagline}\r\n\r\n").unwrap();
    for p in Palette::ALL {
        let names = p.input_names();
        let canonical = names[0];
        let aliases = &names[1..];
        let (r, g, b) = p.accent();
        let pad = width.saturating_sub(canonical.len());
        write!(
            s,
            "    \x1b[1;38;2;{r};{g};{b}m{canonical}\x1b[0m{:pad$}",
            "",
        )
        .unwrap();
        if !aliases.is_empty() {
            write!(s, "  (").unwrap();
            for (i, a) in aliases.iter().enumerate() {
                if i > 0 {
                    write!(s, ", ").unwrap();
                }
                write!(s, "\x1b[1;38;2;{r};{g};{b}m{a}\x1b[0m").unwrap();
            }
            write!(s, ")").unwrap();
        }
        write!(s, "\r\n").unwrap();
    }
    write!(s, "\r\n  keys (in session):\r\n").unwrap();
    write!(s, "    ← / →   cycle palettes\r\n").unwrap();
    write!(s, "    i       invert colors\r\n").unwrap();
    write!(s, "    a       toggle ascii mode\r\n").unwrap();
    write!(s, "    q       quit\r\n").unwrap();
    write!(s, "\r\n  example: {example}\r\n").unwrap();
    write!(s, "\r\n  web:     https://lava.watch\r\n").unwrap();
    write!(
        s,
        "  source:  https://github.com/robherley/lava.watch\r\n\r\n"
    )
    .unwrap();
    s.into_bytes()
}
