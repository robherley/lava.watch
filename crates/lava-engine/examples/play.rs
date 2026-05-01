//! Local playground: render the lava lamp to your terminal at ~30fps.
//! Ctrl-C to exit cleanly.

use lava_engine::{LavaLamp, ENTER_ALT_SCREEN, LEAVE_ALT_SCREEN};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use terminal_size::{terminal_size, Height, Width};

fn main() {
    let (cols, rows) = match terminal_size() {
        Some((Width(w), Height(h))) => (w, h.saturating_sub(1)),
        None => (80, 30),
    };

    let seed = std::env::var("LAVA_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0xC0FFEE_F00D);

    let mut lamp = LavaLamp::new(cols, rows, 7, seed);

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || r.store(false, Ordering::SeqCst))
        .expect("install ctrl-c handler");

    let mut stdout = io::stdout().lock();
    stdout.write_all(ENTER_ALT_SCREEN).unwrap();
    stdout.flush().unwrap();

    let target_dt = Duration::from_millis(33);
    let mut buf: Vec<u8> = Vec::with_capacity(cols as usize * rows as usize * 24);

    while running.load(Ordering::SeqCst) {
        let frame_start = Instant::now();
        lamp.step(target_dt.as_secs_f32());
        buf.clear();
        lamp.render_ansi(&mut buf);
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

    let _ = stdout.write_all(LEAVE_ALT_SCREEN);
    let _ = stdout.flush();
}
