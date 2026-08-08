//! Reads and validates the boot configuration on every boot.
//!
//! The file lives on the boot volume at `\EFI\leon\boot.toml` and is written by
//! `lbc config set` (host side). It carries the splash-menu settings
//! (`timeout`, `default_entry`, `theme`, `splash`) and the entries-file
//! location (`entries_file`); the bootloader's job is to parse and sanity-check
//! it so a corrupted or hostile file can never derail a boot. Parsing failures
//! are logged and treated as "use defaults", never as a reason to stop. The
//! resolved config is passed along into the geometry record
//! (`bootinfo.json`), which is how host tooling sees what a boot ran with.

use uefi::boot::{self, image_handle};
use uefi::cstr16;
use uefi::data_types::CStr16;
use uefi::fs::{FileSystem, Path};

use leon_common::boot_config::{self, BootConfig};

/// Boot configuration file, relative to the boot volume root.
const CONFIG_FILE: &CStr16 = cstr16!(r"\EFI\leon\boot.toml");

/// Parses `\EFI\leon\boot.toml` from the boot volume.
///
/// Every failure mode — no file, unreadable file, unparsable content — falls
/// back to an all-default `BootConfig` and is logged, so a bad config never
/// blocks the boot.
pub fn read() -> BootConfig {
    let Ok(protocol) = boot::get_image_file_system(image_handle()) else {
        return BootConfig::default();
    };
    let mut fs = FileSystem::new(protocol);
    match fs.read(Path::new(CONFIG_FILE)) {
        Ok(bytes) => {
            let Ok(content) = core::str::from_utf8(&bytes) else {
                crate::log_error!("boot.toml is not valid UTF-8; using defaults");
                return BootConfig::default();
            };
            match boot_config::parse_boot_config(content) {
                Ok(cfg) => cfg,
                Err(err) => {
                    crate::log_error!("boot.toml: {err}; using defaults");
                    BootConfig::default()
                }
            }
        }
        Err(_) => BootConfig::default(),
    }
}
