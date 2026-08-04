//! The handoff sanity marker.
//!
//! Draws a small square into a corner of the untouched frame buffer that can
//! never overlap the BGRT logo, proving the flicker-free path end to end.

use leon_common::{Bgrt, Framebuffer, PixelFormat};

/// Draws a handoff sanity marker on the untouched frame buffer.
pub fn take_over(fb: &Framebuffer, bgrt: Option<Bgrt>) {
    if fb.base == 0 || !matches!(fb.format, PixelFormat::Rgbx | PixelFormat::Bgrx) {
        return;
    }
    let size = 16u32;
    let (x, y) = free_corner(fb, bgrt, size);
    for dy in 0..size {
        for dx in 0..size {
            write_pixel(fb, x + dx, y + dy, 0x2e, 0xcc, 0x71);
        }
    }
}

/// Picks a screen corner for the marker that does not overlap the logo.
fn free_corner(fb: &Framebuffer, bgrt: Option<Bgrt>, size: u32) -> (u32, u32) {
    let fallback = (0, fb.height.saturating_sub(size));
    let candidates = [fallback, (fb.width.saturating_sub(size), 0)];
    match candidates
        .iter()
        .copied()
        .find(|(x, y)| !overlaps_logo(bgrt, *x, *y, size))
    {
        Some(corner) => corner,
        None => fallback,
    }
}

/// True if the square at `(x, y)` with the given size intersects the logo box.
fn overlaps_logo(bgrt: Option<Bgrt>, x: u32, y: u32, size: u32) -> bool {
    let (x0, y0, x1, y1) = match bgrt {
        Some(bgrt) => bgrt.rect(),
        None => return false,
    };
    let (ax0, ay0, ax1, ay1) = (
        x as i64,
        y as i64,
        x as i64 + size as i64,
        y as i64 + size as i64,
    );
    ax0 < x1 && ax1 > x0 && ay0 < y1 && ay1 > y0
}

/// Writes one pixel to the frame buffer, honoring the pixel format.
fn write_pixel(fb: &Framebuffer, x: u32, y: u32, r: u8, g: u8, b: u8) {
    if x >= fb.width || y >= fb.height {
        return;
    }
    let pixel = (fb.base as usize + fb.offset(x, y)) as *mut u8;
    unsafe {
        match fb.format {
            PixelFormat::Bgrx => {
                pixel.add(0).write_volatile(b);
                pixel.add(1).write_volatile(g);
                pixel.add(2).write_volatile(r);
            }
            PixelFormat::Rgbx => {
                pixel.add(0).write_volatile(r);
                pixel.add(1).write_volatile(g);
                pixel.add(2).write_volatile(b);
            }
            PixelFormat::Bitmask | PixelFormat::BltOnly => {}
        }
    }
}
