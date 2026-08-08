//! lbl — Leon Bootloader.
//!
//! An ultra-silent, BGRT-preserving UEFI bootloader written in Rust. lbl is a
//! *chainloader* in the style of systemd-boot: on every boot it
//!
//! 1. discovers every boot entry on the ESP — every `\EFI\<vendor>\*.efi`
//!    (recursively), including Leon's own EFI-stub kernel — labels them by
//!    file stem, and persists them to a configurable JSONC entries file
//!    (`\EFI\leon\entries.jsonc` by default);
//! 2. shows an opt-in splash menu (`splash = true` in `\EFI\leon\boot.toml`);
//! 3. chainloads the chosen (or default) entry with
//!    `LoadImage`/`StartImage`, exactly like the firmware Boot Manager does.
//!
//! Nothing is printed to the screen unless the menu is enabled. The GOP frame
//! buffer is never re-initialized (`set_mode` is never called) and the firmware
//! logo — located via the ACPI BGRT — is never touched, so the kernel that
//! eventually takes over can present the logo as if it had always been there.
//! The kernel is an EFI-stub UEFI application and queries both of these itself;
//! lbl only records the geometry for host tooling. Boot errors are appended to
//! the unified log file at `\var\logs\leon\log.md` on the boot volume.
//!
//! The crate is organized into subfolders: the boot pipeline lives in
//! [`boot`] (`config`, `entries`, `image`, `menu`), firmware probing in
//! [`firmware`] (`gop`, `bgrt`), the geometry record in [`record`], and the
//! boot log in [`logger`]. [`boot::run`] is the entry point.

#![no_std]
#![no_main]

extern crate alloc;

mod boot;
mod firmware;
mod logger;
mod record;
mod secure_boot;

#[cfg(feature = "gop-ui")]
mod ui;

use uefi::prelude::*;

#[entry]
fn main() -> Status {
    match boot::run() {
        Ok(()) => Status::SUCCESS,
        Err(err) => {
            crate::log_error!("boot failed: {err:?}");
            halt_forever()
        }
    }
}

fn halt_forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    crate::log_error!("panic: {info}");
    halt_forever()
}
