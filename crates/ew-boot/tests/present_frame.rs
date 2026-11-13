//! The frame path from tiled texture storage to the window, on a real GPU.
//!
//! Ignored by default: it opens a window and needs a Vulkan device. Run it with
//! `cargo test -p ew-boot --test present_frame -- --ignored`.

#![cfg(windows)]

use ew_boot::nvn_tex::{layout, untile_bgra, TexDesc};

/// A small RGBA8 texture, tiled the way the game's swap texture is, untiled and
/// shown in a window for a few frames.
#[test]
#[ignore]
fn a_tiled_frame_reaches_the_window() {
    let d = TexDesc { width: 64, height: 64, format: 0x25, ..TexDesc::default() };
    let lay = layout(&d).expect("an RGBA8 texture has a layout");
    let base = 0u64;
    let mut mem = vec![0u8; lay.size() as usize];
    // A diagonal gradient in RGBA, written through the tiling.
    for y in 0..64u64 {
        for x in 0..64u64 {
            let at = base + lay.offset(0, 0, x, y, 0);
            mem[at as usize..at as usize + 4].copy_from_slice(&[(x * 4) as u8, (y * 4) as u8, 0x20, 0xFF]);
        }
    }
    let bgra = untile_bgra(&mem, base, &d, &lay).expect("the frame untiles");
    assert_eq!(bgra.len(), 64 * 64 * 4);
    // The first texel is (0, 0, 0x20, 0xFF) in RGBA, so (0x20, 0, 0, 0xFF) in BGRA.
    assert_eq!(&bgra[..4], &[0x20, 0, 0, 0xFF]);

    let mut presenter = ew_gfx::present::Presenter::open("EmblemWeave — frame path", 320, 240).expect("the window opens");
    for _ in 0..5 {
        let open = presenter.show(ew_gfx::present::Frame { width: 64, height: 64, bgra: &bgra }).expect("the frame is shown");
        assert!(open, "the window stays open");
    }
    assert_eq!(presenter.frames, 5);
}
