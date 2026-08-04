//! GOP frame buffer capture for the EFI-stub kernel.
//!
//! Reads the mode the firmware already set up — never calling `set_mode` — and
//! normalizes it into the shared `leon_common::Framebuffer`.

use uefi::boot;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as GopPixelFormat};
use uefi::{Result, Status};

use leon_common::{Framebuffer, PixelFormat};

/// Captures the currently active GOP mode's frame buffer.
pub fn capture_framebuffer() -> Result<Framebuffer> {
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
    let base = buffer.as_mut_ptr() as u64;
    let size = buffer.size() as u64;
    if base == 0 || size == 0 {
        return Err(Status::NOT_FOUND.into());
    }

    Ok(Framebuffer {
        base,
        size,
        width: width as u32,
        height: height as u32,
        stride: stride as u32,
        format,
    })
}
