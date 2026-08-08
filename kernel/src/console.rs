//! Framebuffer status panel.
//!
//! After `ExitBootServices` the kernel owns the frame buffer but must not
//! destroy the firmware logo (the whole point of the flicker-free path). This
//! draws a compact status panel into a bottom (or, if the logo is there, top)
//! band that never overlaps the BGRT rectangle, using a small embedded 5x7
//! font at 2x scale. Text is built on the stack — no allocation is allowed
//! after the handoff.

use core::fmt;

use leon_common::{Bgrt, Framebuffer, PixelFormat};

/// Draw-scale: each font pixel becomes a `SCALE`x`SCALE` screen pixel.
const SCALE: u32 = 2;
/// Font glyph width in font pixels.
const GLYPH_W: u32 = 5;
/// Font glyph height in font pixels.
const GLYPH_H: u32 = 7;
/// Horizontal advance per character in font pixels.
const ADVANCE: u32 = GLYPH_W + 2;
/// Height of one text line in font pixels.
const LINE_H: u32 = GLYPH_H + 3;
/// Panel padding in screen pixels.
const PAD: u32 = 6;

const FG: (u8, u8, u8) = (0xE8, 0xE8, 0xE8);
const BG: (u8, u8, u8) = (0x14, 0x14, 0x18);
const OK_COLOR: (u8, u8, u8) = (0x2E, 0xCC, 0x71);
const FAIL_COLOR: (u8, u8, u8) = (0xE7, 0x4C, 0x3C);

/// Draws the status panel: a title line and a status line.
///
/// `ok` switches the status line color between green (all good) and red.
/// Returns the band the panel occupies, so the handoff marker can avoid it.
pub fn show_status(
    fb: &Framebuffer,
    bgrt: Option<Bgrt>,
    title: &str,
    status: &str,
    ok: bool,
) -> Option<(u32, u32)> {
    if fb.base == 0 || !matches!(fb.format, PixelFormat::Rgbx | PixelFormat::Bgrx) {
        return None;
    }
    let line_h = LINE_H * SCALE;
    let panel_h = PAD * 2 + line_h * 2 + SCALE;
    let panel_y = pick_band(fb, bgrt, panel_h);

    fill_rect(fb, 0, panel_y, fb.width, panel_h, BG);
    let text_color = if ok { OK_COLOR } else { FAIL_COLOR };
    draw_text(fb, PAD, panel_y + PAD, title, FG);
    draw_text(fb, PAD, panel_y + PAD + line_h + SCALE, status, text_color);
    Some((panel_y, panel_h))
}

/// Picks a band across the bottom, falling back to the top when the logo
/// occupies the bottom band. Never touches the BGRT rectangle.
fn pick_band(fb: &Framebuffer, bgrt: Option<Bgrt>, height: u32) -> u32 {
    let bottom = fb.height.saturating_sub(height);
    if !overlaps(bgrt, 0, bottom, fb.width, height) {
        return bottom;
    }
    if !overlaps(bgrt, 0, 0, fb.width, height) {
        return 0;
    }
    bottom
}

/// True if the rectangle `(x, y, w, h)` intersects the BGRT logo rectangle.
fn overlaps(bgrt: Option<Bgrt>, x: u32, y: u32, w: u32, h: u32) -> bool {
    let (x0, y0, x1, y1) = match bgrt {
        Some(bgrt) => bgrt.rect(),
        None => return false,
    };
    let (ax0, ay0) = (x as i64, y as i64);
    let (ax1, ay1) = (x as i64 + w as i64, y as i64 + h as i64);
    ax0 < x1 && ax1 > x0 && ay0 < y1 && ay1 > y0
}

/// Draws a text line left-aligned at `(x, y)` in screen pixels.
fn draw_text(fb: &Framebuffer, x: u32, y: u32, s: &str, color: (u8, u8, u8)) {
    let mut cx = x;
    for c in s.chars() {
        if let Some(glyph) = glyph(c) {
            draw_glyph(fb, cx, y, glyph, color);
        }
        cx += ADVANCE * SCALE;
    }
}

/// Draws one 5x7 glyph at 2x scale.
fn draw_glyph(fb: &Framebuffer, x: u32, y: u32, glyph: &[u8; 7], color: (u8, u8, u8)) {
    for (row, &bits) in glyph.iter().enumerate() {
        for col in 0..GLYPH_W {
            if bits & (1 << col) == 0 {
                continue;
            }
            fill_rect(
                fb,
                x + col * SCALE,
                y + row as u32 * SCALE,
                SCALE,
                SCALE,
                color,
            );
        }
    }
}

/// Fills a screen rectangle with a color.
fn fill_rect(fb: &Framebuffer, x: u32, y: u32, w: u32, h: u32, color: (u8, u8, u8)) {
    let (r, g, b) = color;
    for dy in 0..h {
        for dx in 0..w {
            write_pixel(fb, x + dx, y + dy, r, g, b);
        }
    }
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

/// A stack-backed `fmt::Write` used to build the status strings without
/// allocating (the global allocator is dead after the handoff).
struct Buf<'a> {
    data: &'a mut [u8],
    len: usize,
}

impl fmt::Write for Buf<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let n = s.len();
        let room = self.data.len() - self.len;
        if n > room {
            return Err(fmt::Error);
        }
        self.data[self.len..self.len + n].copy_from_slice(s.as_bytes());
        self.len += n;
        Ok(())
    }
}

/// Formats into a caller-provided stack buffer and returns the resulting
/// (ASCII) string slice.
pub fn fmt_buf<'a>(data: &'a mut [u8], args: fmt::Arguments<'_>) -> &'a str {
    let mut buf = Buf { data, len: 0 };
    let _ = fmt::write(&mut buf, args);
    let bytes = &buf.data[..buf.len];
    core::str::from_utf8(bytes).unwrap_or("?")
}

/// Looks up a 5x7 glyph, falling back to `?` for anything unmapped.
fn glyph(c: char) -> Option<&'static [u8; 7]> {
    FONT.iter().find(|(ch, _)| *ch == c).map(|(_, g)| g)
}

/// 5x7 glyphs (7 rows, 5 columns, bit 0 = leftmost column). Only the
/// characters the status panel needs are embedded.
#[rustfmt::skip]
const FONT: &[(char, [u8; 7])] = &[
    (' ', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
    ('|', [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('.', [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C]),
    (',', [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08]),
    (':', [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00]),
    ('-', [0x00, 0x00, 0x00, 0x1E, 0x00, 0x00, 0x00]),
    ('@', [0x0E, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0E]),
    ('?', [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04]),
    ('L', [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F]),
    ('O', [0x0F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0F]),
    ('K', [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11]),
    ('B', [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E]),
    ('0', [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E]),
    ('1', [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E]),
    ('2', [0x0F, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F]),
    ('3', [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E]),
    ('4', [0x04, 0x0A, 0x12, 0x12, 0x1E, 0x12, 0x12]),
    ('5', [0x1E, 0x10, 0x10, 0x1E, 0x01, 0x01, 0x1E]),
    ('6', [0x0F, 0x10, 0x10, 0x16, 0x11, 0x11, 0x0E]),
    ('7', [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
    ('8', [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E]),
    ('9', [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x01, 0x0E]),
    ('a', [0x0E, 0x11, 0x11, 0x0F, 0x11, 0x11, 0x11]),
    ('b', [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E]),
    ('d', [0x09, 0x09, 0x09, 0x0F, 0x09, 0x09, 0x0F]),
    ('e', [0x1E, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1E]),
    ('f', [0x07, 0x0E, 0x08, 0x0F, 0x08, 0x08, 0x08]),
    ('g', [0x0F, 0x11, 0x11, 0x0F, 0x01, 0x01, 0x0E]),
    ('h', [0x10, 0x10, 0x1E, 0x11, 0x11, 0x11, 0x11]),
    ('k', [0x10, 0x12, 0x14, 0x1C, 0x14, 0x12, 0x11]),
    ('l', [0x0C, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1F]),
    ('m', [0x15, 0x15, 0x1E, 0x11, 0x11, 0x11, 0x11]),
    ('n', [0x11, 0x1E, 0x11, 0x11, 0x11, 0x11, 0x11]),
    ('o', [0x0F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0F]),
    ('p', [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10]),
    ('r', [0x11, 0x13, 0x15, 0x19, 0x10, 0x10, 0x10]),
    ('x', [0x11, 0x0A, 0x04, 0x04, 0x0A, 0x11, 0x11]),
];
