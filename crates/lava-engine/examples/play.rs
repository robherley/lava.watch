//! Local playground: render the lava lamp to your terminal at ~30fps.
//!
//! Controls:
//!   ← / →   cycle palette
//!   q / Esc  quit
//!
//! Env knobs (all optional):
//!   LAVA_PALETTE=classic|ocean|...
//!   LAVA_BLOBS=<u32>
//!   LAVA_SPEED=<f32>
//!   LAVA_SEED=<u64>

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal,
};
use lava_engine::{term, Config, Lava, Palette};
use std::io::{self, Write};
use std::time::{Duration, Instant};

fn env<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|s| s.parse().ok())
}

fn main() -> std::io::Result<()> {
    let (cols, rows) = terminal::size()?;

    let defaults = Config::default();
    let config = Config {
        palette: env::<Palette>("LAVA_PALETTE").unwrap_or(defaults.palette),
        blob_count: env::<u32>("LAVA_BLOBS").unwrap_or(defaults.blob_count),
        speed: env::<f32>("LAVA_SPEED").unwrap_or(defaults.speed),
        seed: env::<u64>("LAVA_SEED").unwrap_or(defaults.seed),
    };

    let mut palette_idx = Palette::ALL
        .iter()
        .position(|p| *p == config.palette)
        .unwrap_or(0);

    // Reserve one row at the bottom for the palette name.
    let lamp_rows = rows.saturating_sub(1);
    let mut lava = Lava::with_config(cols, lamp_rows, config);

    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(term::ENTER_ALT_SCREEN)?;
    stdout.flush()?;

    let target_dt = Duration::from_millis(33);
    let mut buf: Vec<u8> = Vec::with_capacity(cols as usize * lamp_rows as usize * 24);

    'main: loop {
        let frame_start = Instant::now();

        // Drain all pending events before rendering the next frame.
        while event::poll(Duration::ZERO).unwrap_or(false) {
            match event::read().unwrap_or(Event::FocusLost) {
                Event::Key(key) => match (key.code, key.modifiers) {
                    (KeyCode::Char('c'), KeyModifiers::CONTROL)
                    | (KeyCode::Char('q'), _)
                    | (KeyCode::Esc, _) => break 'main,
                    (KeyCode::Right, _) => {
                        palette_idx = (palette_idx + 1) % Palette::ALL.len();
                        lava.palette = Palette::ALL[palette_idx];
                    }
                    (KeyCode::Left, _) => {
                        palette_idx = (palette_idx + Palette::ALL.len() - 1) % Palette::ALL.len();
                        lava.palette = Palette::ALL[palette_idx];
                    }
                    _ => {}
                },
                Event::Resize(cols, rows) => {
                    let lamp_rows = rows.saturating_sub(1);
                    lava.resize(cols, lamp_rows);
                    buf = Vec::with_capacity(cols as usize * lamp_rows as usize * 24);
                }
                _ => {}
            }
        }

        lava.step(target_dt.as_secs_f32());
        buf.clear();
        term::render(&lava, &mut buf);

        // Palette name in the reserved bottom row, dimmed so it doesn't fight the lamp.
        let _ = write!(
            buf,
            "\r\n\x1b[2K\x1b[90m  ← {} →\x1b[0m",
            lava.palette.name()
        );

        if stdout.write_all(&buf).is_err() {
            break;
        }
        if stdout.flush().is_err() {
            break;
        }

        let elapsed = frame_start.elapsed();
        if elapsed < target_dt {
            std::thread::sleep(target_dt - elapsed);
        }
    }

    let _ = stdout.write_all(term::LEAVE_ALT_SCREEN);
    let _ = stdout.flush();
    terminal::disable_raw_mode()?;

    Ok(())
}
