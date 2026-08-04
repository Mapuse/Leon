//! Shared geometry types: pixel formats, frame buffer geometry, BGRT metadata.
//!
//! The loader and the kernel no longer exchange a handoff blob: the kernel is a
//! plain UEFI application (an "EFI stub") that queries the GOP frame buffer and
//! the ACPI BGRT itself after being started with `LoadImage`/`StartImage`.
//! What is shared is the *representation* of those firmware facts — see
//! [`PixelFormat`], [`Framebuffer`], and [`Bgrt`].

/// Pixel formats of the frame buffer, mirroring the UEFI GOP pixel formats.
///
/// `Bgrx` and `Rgbx` both store 4 bytes per pixel (24-bit color + 1 unused
/// byte); the byte order of each channel differs and must be honored when
/// writing pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    /// 24-bit RGB, 32-bit stride, last byte unused.
    Rgbx = 0,
    /// 24-bit BGR, 32-bit stride, last byte unused (most common on UEFI).
    Bgrx = 1,
    /// Custom format; red/green/blue masks describe the layout.
    Bitmask = 2,
    /// The mode cannot be drawn to directly; only `BLT` is available.
    BltOnly = 3,
}

/// A GOP frame buffer as handed off to the kernel.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Framebuffer {
    /// Physical address of the frame buffer. The kernel must map this
    /// **without clearing it** to keep the firmware logo visible.
    pub base: u64,
    /// Size of the frame buffer in bytes.
    pub size: u64,
    /// Horizontal resolution in pixels.
    pub width: u32,
    /// Vertical resolution in pixels.
    pub height: u32,
    /// Pixels per scan line (>= `width`).
    pub stride: u32,
    /// Current GOP pixel format.
    pub format: PixelFormat,
}

impl Framebuffer {
    /// Byte offset of the pixel at `(x, y)` inside the frame buffer.
    #[inline]
    pub const fn offset(&self, x: u32, y: u32) -> usize {
        (y as usize * self.stride as usize + x as usize) * 4
    }
}

/// Parsed metadata of the ACPI BGRT (Boot Graphics Resource Table).
///
/// This is what allows the kernel and the separate splash project to keep the
/// motherboard logo on screen and to position UI elements relative to it.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Bgrt {
    /// Physical address of the raw BMP logo image (the "BM" magic).
    pub image_address: u64,
    /// X offset of the logo on screen, from the BGRT table.
    pub offset_x: i32,
    /// Y offset of the logo on screen, from the BGRT table.
    pub offset_y: i32,
    /// Width of the logo in pixels (parsed from the BMP header).
    pub image_width: u32,
    /// Height of the logo in pixels (parsed from the BMP header).
    pub image_height: u32,
    /// BGRT status byte. Bit 0 (`DISPLAYED`) set means the firmware logo is
    /// currently on screen.
    pub status: u8,
    /// BGRT image type. `0` means a bitmap (BMP).
    pub image_type: u8,
}

impl Bgrt {
    /// Bounding box of the logo on screen: `(x0, y0, x1, y1)`, where `x1` and
    /// `y1` are **exclusive** (`x1 = offset_x + image_width`).
    #[inline]
    pub const fn rect(&self) -> (i64, i64, i64, i64) {
        (
            self.offset_x as i64,
            self.offset_y as i64,
            self.offset_x as i64 + self.image_width as i64,
            self.offset_y as i64 + self.image_height as i64,
        )
    }
}
