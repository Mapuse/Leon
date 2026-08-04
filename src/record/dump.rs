//! Real boot geometry capture for host tooling.
//!
//! Every boot, the loader serialises the live GOP frame buffer geometry, the
//! firmware BGRT logo metadata, and the resolved boot configuration as JSON on
//! the boot volume at `\EFI\leon\bootinfo.json`. Host tools (`lbt`) read this
//! file so they author/preview splash themes against *exactly* what the
//! firmware on this machine provided, instead of any assumed resolution or
//! logo.
//!
//! This is a pure record of the *query* — nothing is handed to the kernel
//! anymore. The kernel is an EFI-stub UEFI application that queries GOP and
//! BGRT itself (see `kernel/src/main.rs`). The schema is:
//!
//! ```json
//! {
//!   "framebuffer": { "base": 0, "size": 0, "width": 0, "height": 0,
//!                     "stride": 0, "format": "Bgrx" },
//!   "bgrt": { "image_address": 0, "offset_x": 0, "offset_y": 0,
//!             "image_width": 0, "image_height": 0, "status": 0, "image_type": 0 }
//!          | null,
//!   "boot_config": { "timeout": 5, "default_entry": "Cudane Linux",
//!                    "theme": "splash.py", "splash": true,
//!                    "entries_file": "\\EFI\\leon\\entries.jsonc" }
//! }
//! ```

use alloc::format;
use alloc::string::{String, ToString};
use uefi::boot::{self, image_handle};
use uefi::cstr16;
use uefi::data_types::CStr16;
use uefi::fs::{FileSystem, Path};

use leon_common::boot_config::BootConfig;
use leon_common::{Bgrt, Framebuffer};

/// Directory of the capture file, relative to the boot volume root.
const DUMP_DIR: &CStr16 = cstr16!(r"\EFI\leon");
/// Capture file, relative to the boot volume root.
const DUMP_FILE: &CStr16 = cstr16!(r"\EFI\leon\bootinfo.json");

/// Writes the live boot geometry as JSON to the boot volume.
///
/// Best-effort: if the volume can't be written the boot continues silently.
/// Must be called while boot services are still active.
pub fn write(framebuffer: &Framebuffer, bgrt: Option<Bgrt>, boot_config: &BootConfig) {
    let Ok(protocol) = boot::get_image_file_system(image_handle()) else {
        return;
    };
    let mut fs = FileSystem::new(protocol);
    if fs.create_dir_all(Path::new(DUMP_DIR)).is_err() {
        return;
    }

    let fb_json = format!(
        r#"{{"base":{},"size":{},"width":{},"height":{},"stride":{},"format":"{:?}"}}"#,
        framebuffer.base,
        framebuffer.size,
        framebuffer.width,
        framebuffer.height,
        framebuffer.stride,
        framebuffer.format
    );
    let bgrt_json = match bgrt {
        Some(b) => format!(
            r#"{{"image_address":{},"offset_x":{},"offset_y":{},"image_width":{},"image_height":{},"status":{},"image_type":{}}}"#,
            b.image_address,
            b.offset_x,
            b.offset_y,
            b.image_width,
            b.image_height,
            b.status,
            b.image_type
        ),
        None => String::from("null"),
    };
    let config_json = format!(
        r#"{{"timeout":{},"default_entry":{},"theme":{},"splash":{},"entries_file":{}}}"#,
        config_u64(boot_config.timeout),
        config_str(boot_config.default_entry.as_deref()),
        config_str(boot_config.theme.as_deref()),
        config_bool(boot_config.splash),
        config_str(boot_config.entries_file.as_deref()),
    );
    let json = format!(
        r#"{{"framebuffer":{},"bgrt":{},"boot_config":{}}}"#,
        fb_json, bgrt_json, config_json
    );

    let _ = fs.write(Path::new(DUMP_FILE), json.as_bytes());
}

fn config_u64(v: Option<u32>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => String::from("null"),
    }
}

fn config_str(v: Option<&str>) -> String {
    match v {
        Some(s) => format!("\"{s}\""),
        None => String::from("null"),
    }
}

fn config_bool(v: Option<bool>) -> String {
    match v {
        Some(b) => b.to_string(),
        None => String::from("null"),
    }
}
