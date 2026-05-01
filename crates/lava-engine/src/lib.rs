//! Headless lava lamp engine.
//!
//! Pure Rust simulation + ANSI terminal renderer, sharing one byte stream
//! between SSH (real terminals) and the browser (xterm.js via wterm).
//!
//! ```no_run
//! use lava_engine::{term, Config, Lava, Palette};
//!
//! let mut lava = Lava::with_config(80, 30, Config {
//!     palette: Palette::Bubblegum,
//!     blob_count: 6,
//!     ..Default::default()
//! });
//! let mut frame = Vec::new();
//! loop {
//!     lava.step(1.0 / 30.0);
//!     frame.clear();
//!     term::render(&lava, &mut frame);
//!     // write `frame` to a PTY, WebSocket, or stdout
//! #   break;
//! }
//! ```

mod palette;
mod rng;
mod sim;
pub mod term;

pub use palette::{Palette, ParsePaletteError};
pub use rng::Rng;
pub use sim::{Blob, Config, Lava};
