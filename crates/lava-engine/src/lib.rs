//! Headless lava lamp engine.
//!
//! Pure Rust simulation + ANSI terminal renderer + a thin interactive
//! [`Session`] layer (palette cycling, click-to-heat, badge overlay) shared
//! between every transport: SSH server, WASM-in-browser, anything else
//! that can do bytes-in / bytes-out.
//!
//! ```no_run
//! use lava_engine::{Palette, Session};
//!
//! let mut session = Session::new(80, 30, Palette::Bubblegum);
//! let mut frame = Vec::new();
//! loop {
//!     session.tick(1.0 / 30.0);
//!     frame.clear();
//!     session.render(&mut frame);
//!     // write `frame` bytes to a PTY, WebSocket, xterm.js, stdout, …
//! #   break;
//! }
//! ```

mod palette;
pub mod pixels;
mod rng;
mod session;
mod sim;
pub mod term;

pub use palette::{Palette, ParsePaletteError};
pub use rng::Rng;
pub use session::{parse_input, Input, Session};
pub use sim::{Blob, Config, Lava};
