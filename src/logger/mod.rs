//! Ultra-silent error logging.
//!
//! Nothing is ever printed to the screen. The only place a boot error shows up
//! is the unified log file on the boot volume:
//!
//! ```text
//! \var\logs\leon\log.md
//! ```
//!
//! This is `/var/logs/leon/log.md` relative to the ESP root. The bootloader
//! cannot reach the installed OS's real `/var/logs` before the kernel boots;
//! the kernel/init chain is expected to continue the same file at the same
//! path on the real root filesystem, which is how leon keeps a single
//! boot-to-desktop log.
//!
//! Records are appended, Markdown list style, one line per boot error. The
//! file is kept below a hard cap (64 KiB): once a new line would exceed it,
//! the oldest lines are dropped so the log stays bounded and always ends with
//! the most recent errors. If the log cannot be written (no filesystem, no
//! permission, boot services gone), the error is dropped silently — silence
//! wins over everything.

use alloc::string::{String, ToString};
use uefi::boot::{self, image_handle};
use uefi::cstr16;
use uefi::data_types::CStr16;
use uefi::fs::{FileSystem, Path};

/// Directory of the unified log file, relative to the boot volume root.
const LOG_DIR: &CStr16 = cstr16!(r"\var\logs\leon");
/// Unified log file, relative to the boot volume root.
const LOG_FILE: &CStr16 = cstr16!(r"\var\logs\leon\log.md");
/// Upper bound for the log file, in bytes. Oldest lines are dropped first.
const LOG_CAP: usize = 64 * 1024;

/// Appends a formatted error line to the unified log file.
///
/// Every failure path routes through here; the screen stays untouched.
/// The loader never exits boot services (the chainloaded kernel does), so
/// this is always safe to call.
pub fn log_line(args: core::fmt::Arguments<'_>) {
    let Ok(protocol) = boot::get_image_file_system(image_handle()) else {
        return;
    };
    let mut fs = FileSystem::new(protocol);

    if fs.create_dir_all(Path::new(LOG_DIR)).is_err() {
        return;
    }

    let mut line = String::from("- ");
    if core::fmt::write(&mut line, args).is_err() {
        return;
    }
    line.push('\n');

    let path = Path::new(LOG_FILE);
    // A non-UTF-8 log is garbage; start fresh instead of appending to it.
    let mut text = String::from_utf8(fs.read(path).unwrap_or_default()).unwrap_or_default();
    if text.len() + line.len() > LOG_CAP {
        // Keep the log bounded: drop the oldest bytes down to a line boundary
        // so the file always fits under the cap and ends with recent errors.
        let keep = text.floor_char_boundary(LOG_CAP.saturating_sub(line.len()));
        let start = text[..keep].rfind('\n').map(|i| i + 1).unwrap_or(keep);
        text = text[start..].to_string();
    }
    text.push_str(&line);
    let _ = fs.write(path, text.as_bytes());
}

/// Logs a boot error. Expands to `logger::log_line` with a literal format
/// string, so callers use the usual `format!`-style syntax.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logger::log_line(core::format_args!($($arg)*))
    };
}
