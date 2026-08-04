//! GOP (Graphics Output Protocol) handling.
//!
//! The golden rule of a silent handoff: **never call `set_mode`**. Calling it
//! forces the graphics hardware to re-initialize, which blanks the screen and
//! destroys the firmware logo. We only query the current mode and lift its
//! frame buffer out.

use leon_common::{Framebuffer, PixelFormat};
use uefi::boot;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as GopPixelFormat};
use uefi::{Result, Status};

/// Captures the currently active GOP mode's frame buffer.
///
/// Returns `Err` if no GOP device exists, the protocol can't be opened, or the
/// frame buffer isn't actually usable (e.g. `BltOnly` with a zero buffer).
pub fn capture_framebuffer() -> Result<Framebuffer> {
    let handles = boot::find_handles::<GraphicsOutput>()?;
    let handle = *handles.first().ok_or(Status::NOT_FOUND)?;
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle)?;

    // Query, don't touch: this reads the mode the firmware already set up.
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
