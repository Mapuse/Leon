//! Basic GOP framebuffer renderer scaffold.
//!
//! Provides a `GopRenderer` that captures the current GOP frame buffer and
//! exposes primitive draw operations. This is a starting point for porting
//! the host TUI's layout and widgets into the UEFI bootloader.

use core::slice;
use leon_common::PixelFormat;
use uefi::Result;
use uefi::Status;
use uefi::boot;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as GopPixelFormat};
use uefi::proto::console::text::Output as TextOut;

pub struct GopRenderer {
    base: *mut u8,
    size: usize,
    pub width: usize,
    pub height: usize,
    stride: usize,
    format: PixelFormat,
}

impl GopRenderer {
    /// Capture the current GOP framebuffer into a renderer instance.
    pub fn capture() -> Result<Self> {
        let handles = boot::find_handles::<GraphicsOutput>()?;
        let handle = *handles.first().ok_or(Status::NOT_FOUND)?;
        let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle)?;

        let info = gop.current_mode_info();
        let (width, height) = info.resolution();
        let stride = info.stride();
        let format = match info.pixel_format() {
            GopPixelFormat::Rgb => PixelFormat::Rgbx,
            GopPixelFormat::Bgr => PixelFormat::Bgrx,
            GopPixelFormat::Bitmask => PixelFormat::Bitmask,
            GopPixelFormat::BltOnly => PixelFormat::BltOnly,
        };

        let mut buffer = gop.frame_buffer();
        let base = buffer.as_mut_ptr();
        let size = buffer.size();
        if base as usize == 0 || size == 0 {
            return Err(Status::NOT_FOUND.into());
        }

        Ok(Self {
            base,
            size: size as usize,
            width,
            height,
            stride: stride as usize,
            format,
        })
    }

    #[inline]
    fn buffer_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.base, self.size) }
    }

    /// Draw a single pixel in 32-bit RGBA-like layout respecting the GOP
    /// pixel order (Rgbx vs Bgrx). Color is 0xRRGGBB.
    pub fn put_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let off = (y * self.stride + x) * 4;
        let r = ((color >> 16) & 0xff) as u8;
        let g = ((color >> 8) & 0xff) as u8;
        let b = (color & 0xff) as u8;
        let format = self.format;
        let buf = self.buffer_mut();
        if off + 4 > buf.len() {
            return;
        }
        match format {
            PixelFormat::Rgbx => {
                buf[off] = r;
                buf[off + 1] = g;
                buf[off + 2] = b;
                buf[off + 3] = 0;
            }
            PixelFormat::Bgrx => {
                buf[off] = b;
                buf[off + 1] = g;
                buf[off + 2] = r;
                buf[off + 3] = 0;
            }
            PixelFormat::Bitmask | PixelFormat::BltOnly => {
                // Unsupported for direct writes in this simple implementation.
            }
        }
    }

    /// Fill a rectangle with a solid color.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        let max_x = (x + w).min(self.width);
        let max_y = (y + h).min(self.height);
        for yy in y..max_y {
            for xx in x..max_x {
                self.put_pixel(xx, yy, color);
            }
        }
    }

    /// Simple text fallback that uses the UEFI text console. This is not a
    /// pixel-blitted font but is useful as a reliable fallback while the
    /// glyph blitter is implemented.
    pub fn console_text(out: &mut TextOut, x: usize, y: usize, text: &str) {
        let _ = out.set_cursor_position(x, y);
        let mut buf = [0u16; 512];
        if let Ok(s) = uefi::CStr16::from_str_with_buf(text, &mut buf) {
            let _ = out.output_string(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gop_renderer_put_pixel_and_fill_rect() {
        let mut storage = vec![0u8; 4 * 8 * 2];
        let mut renderer = GopRenderer {
            base: storage.as_mut_ptr(),
            size: storage.len(),
            width: 8,
            height: 2,
            stride: 8,
            format: PixelFormat::Rgbx,
        };

        renderer.fill_rect(1, 0, 3, 2, 0x112233);
        for y in 0..2 {
            for x in 1..4 {
                let off = (y * 8 + x) * 4;
                assert_eq!(storage[off], 0x11);
                assert_eq!(storage[off + 1], 0x22);
                assert_eq!(storage[off + 2], 0x33);
                assert_eq!(storage[off + 3], 0);
            }
        }

        renderer.put_pixel(7, 1, 0x445566);
        let off = (1 * 8 + 7) * 4;
        assert_eq!(storage[off], 0x44);
        assert_eq!(storage[off + 1], 0x55);
        assert_eq!(storage[off + 2], 0x66);
        assert_eq!(storage[off + 3], 0);
    }

    #[test]
    fn put_pixel_out_of_bounds_does_not_write() {
        let mut storage = vec![0u8; 4 * 4 * 4];
        let before = storage.clone();
        let mut renderer = GopRenderer {
            base: storage.as_mut_ptr(),
            size: storage.len(),
            width: 4,
            height: 4,
            stride: 4,
            format: PixelFormat::Rgbx,
        };

        renderer.put_pixel(4, 0, 0x010203);
        renderer.put_pixel(0, 4, 0x010203);
        assert_eq!(storage, before);
    }
}
