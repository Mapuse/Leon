//! The boot pipeline.
//!
//! This module drives a boot from firmware facts to a chainloaded image:
//! capture the GOP frame buffer, locate the BGRT logo, read the boot config,
//! discover every boot entry on the ESP, record the real geometry for host
//! tooling, and finally chainload the chosen (or default) entry with
//! `LoadImage`/`StartImage`.

pub mod config;
pub mod entries;
pub mod image;
pub mod menu;
pub mod serial;

use alloc::vec::Vec;

use uefi::Result;
use uefi::boot::{self, image_handle};
use uefi::fs::FileSystem;

use entries::Entry;

/// Runs one boot and returns the UEFI status to report to the firmware.
pub fn run() -> Result<()> {
    // Capture the current GOP frame buffer without re-initializing it, and
    // locate the firmware logo via the ACPI BGRT. Both are recorded for host
    // tooling; the kernel re-queries them itself.
    let framebuffer = crate::firmware::gop::capture_framebuffer()?;
    let bgrt = crate::firmware::bgrt::find();

    // Read + validate the boot configuration (`\EFI\leon\boot.toml`, written
    // by `lbc config set` or the on-device menuconfig). A missing or broken
    // file simply yields defaults.
    let mut boot_config = config::read();

    // Keep the boot-volume filesystem alive: the menuconfig edits the config
    // in place and persists it back to `boot.toml` on every committed change.
    let mut fs = boot::get_image_file_system(image_handle())
        .map(FileSystem::new)
        .ok();

    // Discover every boot entry on the ESP and persist it (best-effort).
    let mut entries: Vec<Entry> = Vec::new();
    if let Some(fs) = fs.as_mut() {
        entries = entries::discover(fs);
        entries::write_entries_file(fs, &boot_config, &entries);
    }

    // Persist the real boot geometry for host tooling (`lbt`). Best-effort.
    crate::record::write(&framebuffer, bgrt, &boot_config);

    // Report the Secure Boot state: when it is on, an unsigned entry is
    // rejected by the firmware at `LoadImage` time, so warn up front (both on
    // the menu, if shown, and in the boot log).
    let secure_boot = crate::secure_boot::state();
    if let Some(warn) = crate::secure_boot::warning(secure_boot) {
        crate::log_error!("boot: {warn}");
    }

    // Pick the entry to boot: the menuconfig choice if the splash menu was
    // shown, otherwise `default_entry`, otherwise the first discovery.
    let chosen = if boot_config.splash == Some(true) {
        menu::run(fs.as_mut(), &mut boot_config, &entries, secure_boot)
    } else {
        menu::default_index(&boot_config, &entries)
    };
    let Some(entry) = entries.get(chosen) else {
        crate::log_error!("boot: no boot entries discovered on the ESP");
        return Ok(());
    };

    // Chainload it exactly like the firmware Boot Manager would.
    crate::log_error!(
        "boot: starting {}",
        entries::cstr_lossy(entry.path.as_ref())
    );
    image::boot(entry)
}
