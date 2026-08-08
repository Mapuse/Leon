//! lbl-kernel — minimal EFI-stub kernel.
//!
//! In the EFI-stub model the kernel is a plain UEFI application, started by
//! `lbl` (or any other loader) with `LoadImage`/`StartImage` — the same
//! mechanism a boot menu uses for every OS. Its only job is to prove the
//! flicker-free path still holds when nothing is handed across a structure:
//!
//! * It queries the current GOP mode itself — without ever calling `set_mode`
//!   — so the firmware logo is still sitting in the frame buffer.
//! * It finds the ACPI BGRT itself and keeps its on-screen rectangle safe.
//! * It exits boot services, structurally validates the memory map it now
//!   owns, and draws a sanity marker into a corner of the untouched frame
//!   buffer that can never overlap the logo.
//! * It reports the handoff on the raw serial console and in a status panel
//!   at the edge of the frame buffer, then parks, ready to be replaced by the
//!   real kernel/init system.
//!
//! The crate is organized into subfolders: GOP capture in [`gop`], BGRT
//! discovery in [`bgrt`], memory-map validation in [`memmap`], the serial
//! console in [`serial`], and the frame-buffer status panel in [`console`].
//! The GOP/BGRT discovery mirrors `lbl`'s own finders (`src/firmware/`),
//! which it cannot import: the loader uses the UEFI image file system while
//! this stub is a UEFI application. Both stay deliberately read-only about
//! the screen until the handoff.

#![no_std]
#![no_main]

use core::arch::asm;

use uefi::boot::{self, MemoryType};
use uefi::prelude::*;
use uefi::Status;

mod bgrt;
mod console;
mod gop;
mod marker;
mod memmap;
mod serial;

/// UEFI application entry point. `lbl` chainloads this exactly like any other
/// boot entry: `LoadImage` + `StartImage` with a device path. The stub talks to
/// the firmware through the global system table (the boot services in `uefi::boot`
/// and the configuration table accessor in `uefi::system`).
#[entry]
fn efi_main() -> Status {
    // Query, don't touch: this reads the mode the firmware already set up.
    let framebuffer = match gop::capture_framebuffer() {
        Ok(fb) => fb,
        Err(_) => return Status::ABORTED,
    };
    let bgrt = bgrt::find_bgrt();

    // Take over. From here on nothing may touch the screen; the logo is still
    // sitting in the frame buffer at this exact millisecond.
    let map = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };

    let map_ok = memmap::validate_map(&map);
    mask_interrupts();

    // The handoff is now complete: everything from here on is the kernel.
    // Report the handoff on the raw serial console (no firmware services left)
    // and draw a status panel into a corner of the untouched frame buffer.
    let mut logo = [0u8; 64];
    let logo = logo_text(&mut logo, bgrt);
    let mut status = [0u8; 200];
    let status = console::fmt_buf(
        &mut status,
        format_args!(
            "memmap {} - fb {}x{} {} - logo {}",
            if map_ok { "OK" } else { "FAIL" },
            framebuffer.width,
            framebuffer.height,
            format_name(framebuffer.format),
            logo,
        ),
    );
    let mut banner = [0u8; 120];
    let banner = console::fmt_buf(
        &mut banner,
        format_args!(
            "Leon kernel: handoff {} - {}",
            if map_ok { "OK" } else { "FAIL" },
            status,
        ),
    );
    serial::log(format_args!("{}", banner));
    let panel = console::show_status(&framebuffer, bgrt, "Leon kernel - handoff complete", status, map_ok);
    marker::take_over(&framebuffer, bgrt, panel);
    halt()
}

/// Textual description of the BGRT logo, if one is present.
fn logo_text(buf: &mut [u8], bgrt: Option<leon_common::Bgrt>) -> &str {
    match bgrt {
        Some(bgrt) => console::fmt_buf(
            buf,
            format_args!(
                "{}x{} @ {},{}",
                bgrt.image_width,
                bgrt.image_height,
                bgrt.offset_x,
                bgrt.offset_y
            ),
        ),
        None => "none",
    }
}

/// Human-readable name of a pixel format.
fn format_name(format: leon_common::PixelFormat) -> &'static str {
    match format {
        leon_common::PixelFormat::Rgbx => "Rgbx",
        leon_common::PixelFormat::Bgrx => "Bgrx",
        leon_common::PixelFormat::Bitmask => "Bitmask",
        leon_common::PixelFormat::BltOnly => "BltOnly",
    }
}

/// Masks all maskable interrupts on this architecture. On x86_64 that is
/// `cli`; on aarch64 the DAIF register is set so no IRQ/FIQ can fire.
#[inline]
fn mask_interrupts() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        asm!("cli", options(nomem, nostack))
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        asm!("msr daifset, #0b1111", options(nomem, nostack))
    }
}

fn halt() -> ! {
    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            asm!("hlt", options(nomem, nostack))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            asm!("wfi", options(nomem, nostack))
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    // Keep the panic message around for post-mortem inspection; the stub has
    // no console, so the CPU just parks.
    core::hint::black_box(info);
    halt()
}
