//! `ew-present` — show a test pattern in the window the game's frames go to.
//!
//! It opens the presenter on its own, draws a moving gradient into a frame the
//! size of the window and presents it, so the window, surface, swap chain and
//! upload path can be checked without the game. The runtime uses the same
//! presenter when it has a frame from the game.
//!
//! Usage: `ew-present [frames]` (default 120).

#[cfg(windows)]
fn main() {
    if let Err(e) = run() {
        eprintln!("ew-present: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ew-present: the window exists on Windows only");
}

#[cfg(windows)]
fn run() -> Result<(), String> {
    use ew_gfx::present::{Frame, Presenter};
    let frames: u64 = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(120);
    let (width, height) = (960u32, 540u32);
    let mut presenter = Presenter::open("EmblemWeave — presenter check", width, height)?;
    println!("window and swap chain ready: {}x{}", width, height);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for n in 0..frames {
        for y in 0..height {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                // A gradient that moves, so a still window is obvious.
                pixels[i] = ((x + n as u32 * 4) % 256) as u8;
                pixels[i + 1] = ((y + n as u32 * 2) % 256) as u8;
                pixels[i + 2] = 96;
                pixels[i + 3] = 255;
            }
        }
        if !presenter.show(Frame { width, height, bgra: &pixels })? {
            println!("the window was closed after {} frames", presenter.frames);
            return Ok(());
        }
    }
    println!("presented {} frames", presenter.frames);
    Ok(())
}
