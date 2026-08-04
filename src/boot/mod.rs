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

use alloc::vec::Vec;

use uefi::Result;
use uefi::boot::{self, image_handle};
use uefi::fs::FileSystem;

#[cfg(feature = "gop-ui")]
use crate::ui::GopRenderer;

use entries::Entry;

/// Runs one boot and returns the UEFI status to report to the firmware.
pub fn run() -> Result<()> {
    // Capture the current GOP frame buffer without re-initializing it, and
    // locate the firmware logo via the ACPI BGRT. Both are recorded for host
    // tooling; the kernel re-queries them itself.
    let framebuffer = crate::firmware::gop::capture_framebuffer()?;
    let bgrt = crate::firmware::bgrt::find();

    // Read + validate the boot configuration (`\EFI\leon\boot.toml`, written
    // by `lbt config set`). A missing or broken file simply yields defaults.
    let boot_config = config::read();

    // Discover every boot entry on the ESP and persist it (best-effort).
    let mut entries: Vec<Entry> = Vec::new();
    if let Ok(protocol) = boot::get_image_file_system(image_handle()) {
        let mut fs = FileSystem::new(protocol);
        entries = entries::discover(&mut fs);
        entries::write_entries_file(&mut fs, &boot_config, &entries);
    }

    // Persist the real boot geometry for host tooling (`lbt`). Best-effort.
    crate::record::write(&framebuffer, bgrt, &boot_config);

    #[cfg(feature = "gop-ui")]
    {
        use uefi::proto::console::text::Output;
        use uefi::system;

        if let Ok(mut renderer) = GopRenderer::capture() {
            renderer.fill_rect(0, 0, renderer.width, renderer.height, 0x002b2b2b);
            if let Ok(Some(mut stdout)) = system::with_stdout(|out| Ok(out.clone())) {
                GopRenderer::console_text(&mut stdout, 1, 1, "Leon UEFI GOP UI placeholder");
                GopRenderer::console_text(&mut stdout, 1, 2, "Press Enter to continue to the text menu");
                let _ = stdout.set_cursor_position(0, 4);
            }
        }
    }
    // A `ui::GopRenderer` scaffold exists in `src/ui` and may be used to port
    // the host npyscreen-like layout into the bootloader in future work.

    // Pick the entry to boot: the menu choice if the splash menu was shown,
    // otherwise `default_entry`, otherwise the first discovery.
    let chosen = if boot_config.splash == Some(true) {
        menu::run(&boot_config, &entries)
    } else {
        boot_config
            .default_entry
            .as_deref()
            .and_then(|want| {
                entries
                    .iter()
                    .position(|e| entries::cstr_lossy(e.label.as_ref()).eq(want))
            })
            .unwrap_or(0)
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
