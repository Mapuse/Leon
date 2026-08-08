//! Boot entry discovery and persistence.
//!
//! On every boot, lbl scans the ESP for every `\EFI\<vendor>\*.efi` file — the
//! same convention the firmware's own Boot Manager uses — and treats each one
//! as a boot entry. Nothing is hardcoded: Leon's own kernel is no longer
//! special, it lives at `\EFI\leon\kernel.efi` and is chainloaded exactly like
//! any other entry with `LoadImage`/`StartImage`.
//!
//! Discovery is recursive (vendor subdirectories are scanned too, matching
//! `lbt discover`), and labels are file stems (`kernel`, `shimx64`, ...) so a
//! `default_entry` written by `lbt config set` always matches at boot.
//!
//! The discovered entries are written to a JSONC file on the boot volume
//! (configurable via the `entries_file` boot config key, default
//! `\EFI\leon\entries.jsonc`) so host tooling (`lbt list`) sees exactly what a
//! boot found. Writing is best-effort; the in-memory discovery is the source
//! of truth for the menu.

use alloc::string::String;
use alloc::vec::Vec;

use uefi::cstr16;
use uefi::data_types::CStr16;
use uefi::fs::{FileSystem, Path};
use uefi::{CString16, Char16};

use leon_common::boot_config::BootConfig;

/// Root of every vendor directory on the ESP.
pub const EFI_ROOT: &CStr16 = cstr16!(r"\EFI");
/// Firmware fallback directory — contains this same loader on many ESPs.
const BOOT_DIR: &CStr16 = cstr16!("BOOT");
/// Default entries file path, relative to the boot volume root.
const DEFAULT_ENTRIES_FILE: &CStr16 = cstr16!(r"\EFI\leon\entries.jsonc");
/// Directory that always holds the entries file, so it can be created up front.
const LEON_DIR: &CStr16 = cstr16!(r"\EFI\leon");
/// Hard cap on directory depth during discovery. ESP trees are flat or one
/// level deep; this bounds recursion on pathological layouts (e.g. self
/// links), which some FAT implementations expose as `\EFI\EFI\...`.
const MAX_DEPTH: usize = 8;

/// One boot entry: a UEFI application on the ESP.
pub struct Entry {
    /// Display label (also what `default_entry` matches against).
    pub label: CString16,
    /// Absolute ESP path, `\EFI\...`.
    pub path: CString16,
}

/// Discovers every `*.efi` under `\EFI\<vendor>\`, recursively, in
/// directory-then-name order. The firmware fallback directory is skipped so
/// the loader itself never shows up as a boot entry. Entry labels are the
/// file name without its `.efi` extension, matching what host tooling (`lbt`)
/// reports — so `default_entry` written by `lbt config set` matches here.
pub fn discover(fs: &mut FileSystem) -> Vec<Entry> {
    let mut entries = Vec::new();
    let Ok(iter) = fs.read_dir(Path::new(EFI_ROOT)) else {
        return entries;
    };
    for item in iter.flatten() {
        let name = trim_fat_padding(item.file_name());
        let name = name.as_ref();
        if is_self_or_parent(name) || !item.is_directory() {
            continue;
        }
        if eq_ignore_ascii_case(name, BOOT_DIR) {
            continue;
        }
        let Some(dir_path) = join_path(EFI_ROOT, name) else {
            continue;
        };
        collect_dir(fs, dir_path.as_ref(), 1, &mut entries);
    }
    entries
}

/// Recursively collects every `*.efi` file under `dir` as a boot entry,
/// mirroring the host tool's recursive ESP scan (`lbt discover`).
fn collect_dir(fs: &mut FileSystem, dir: &CStr16, depth: usize, out: &mut Vec<Entry>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(iter) = fs.read_dir(Path::new(dir)) else {
        return;
    };
    for item in iter.flatten() {
        let name = trim_fat_padding(item.file_name());
        let name = name.as_ref();
        if is_self_or_parent(name) {
            continue;
        }
        if item.is_directory() {
            let Some(sub) = join_path(dir, name) else {
                continue;
            };
            collect_dir(fs, sub.as_ref(), depth + 1, out);
        } else if item.is_regular_file() && ends_with_efi(name) {
            let Some(full) = join_path(dir, name) else {
                continue;
            };
            out.push(Entry {
                label: without_efi(name),
                path: full,
            });
        }
    }
}

/// Whether a directory entry is the `.` or `..` self/parent link. The FAT
/// filesystem pads short (8.3) names to a fixed width with trailing spaces and
/// the firmware hands those back verbatim, so this checks the padded form.
fn is_self_or_parent(name: &CStr16) -> bool {
    let slice = name.to_u16_slice();
    slice == [b'.' as u16] || slice == [b'.' as u16, b'.' as u16]
}

/// FAT pads short (8.3) directory entry names to a fixed width with trailing
/// spaces; the firmware returns them verbatim. Strip the padding so names
/// compare and join cleanly. Long (LFN) names are never padded.
fn trim_fat_padding(name: &CStr16) -> CString16 {
    let mut end = name.to_u16_slice().len();
    let slice = name.to_u16_slice();
    while end > 0 && slice[end - 1] == b' ' as u16 {
        end -= 1;
    }
    let mut v: Vec<u16> = slice[..end].to_vec();
    v.push(0);
    CString16::try_from(v).unwrap_or_else(|_| CString16::from(name))
}

/// The file name without the trailing `.efi` extension (any case), so labels
/// read `kernel`, `shimx64`, `BOOTX64` — the same stems `lbt` reports.
fn without_efi(name: &CStr16) -> CString16 {
    let slice = name.to_u16_slice();
    let keep = slice.len().saturating_sub(EFI_SUFFIX.len());
    let mut v: Vec<u16> = slice[..keep].to_vec();
    v.push(0);
    CString16::try_from(v).unwrap_or_else(|_| CString16::from(name))
}

/// Writes the discovered entries to the configured JSONC file on the boot
/// volume. Best-effort: a read-only volume never derails the boot.
pub fn write_entries_file(fs: &mut FileSystem, cfg: &BootConfig, entries: &[Entry]) {
    let Some(path) = entries_path(cfg) else {
        return;
    };
    if fs.create_dir_all(Path::new(LEON_DIR)).is_err() {
        return;
    }
    let _ = fs.write(Path::new(path.as_ref()), entries_json(entries).as_bytes());
}

/// Resolves the entries file path from the boot config, defaulting to
/// `\EFI\leon\entries.jsonc`.
fn entries_path(cfg: &BootConfig) -> Option<CString16> {
    match &cfg.entries_file {
        Some(path) => CString16::try_from(path.as_str()).ok(),
        None => Some(CString16::from(DEFAULT_ENTRIES_FILE)),
    }
}

/// Renders the entries list as a JSONC document:
///
/// ```jsonc
/// // Auto-generated by lbl on every boot. Do not edit.
/// {
///   "entries": [
///     { "label": "kernel", "path": "\\EFI\\leon\\kernel.efi" }
///   ]
/// }
/// ```
fn entries_json(entries: &[Entry]) -> String {
    let mut out = String::from("// Auto-generated by lbl on every boot. Do not edit.\n");
    out.push_str("{\n  \"entries\": [\n");
    for (i, e) in entries.iter().enumerate() {
        out.push_str("    { ");
        out.push_str("\"label\": \"");
        out.push_str(&json_escape(&cstr_lossy(e.label.as_ref())));
        out.push_str("\", \"path\": \"");
        out.push_str(&json_escape(&cstr_lossy(e.path.as_ref())));
        out.push_str("\" }");
        if i + 1 < entries.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// Escapes a string for inclusion in a JSON string literal.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Case-insensitive `\EFI`-style path builder: `parent \ name`, NUL-terminated.
pub fn join_path(parent: &CStr16, name: &CStr16) -> Option<CString16> {
    let mut v = Vec::with_capacity(parent.to_u16_slice().len() + 1 + name.to_u16_slice().len() + 1);
    v.extend_from_slice(parent.to_u16_slice());
    v.push(b'\\' as u16);
    v.extend_from_slice(name.to_u16_slice());
    v.push(0);
    CString16::try_from(v).ok()
}

/// ASCII case-insensitive comparison of two UEFI strings.
pub fn eq_ignore_ascii_case(a: &CStr16, b: &CStr16) -> bool {
    let a = a.to_u16_slice();
    let b = b.to_u16_slice();
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| ascii_lower(*x) == ascii_lower(*y))
}

/// Whether a UEFI file name ends in `.efi` (any case). FAT-padded short names
/// (e.g. `GRUB.EFI   `) are accepted; only long names and non-EFI files match.
pub fn ends_with_efi(name: &CStr16) -> bool {
    let slice = name.to_u16_slice();
    let mut end = slice.len();
    while end > 0 && slice[end - 1] == b' ' as u16 {
        end -= 1;
    }
    let name = &slice[..end];
    name.len() >= EFI_SUFFIX.len()
        && name[name.len() - EFI_SUFFIX.len()..]
            .iter()
            .zip(EFI_SUFFIX)
            .all(|(x, y)| ascii_lower(*x) == y)
}

/// The `.efi` extension, lowercased, used by both `ends_with_efi` and
/// `without_efi`.
const EFI_SUFFIX: [u16; 4] = [b'.' as u16, b'e' as u16, b'f' as u16, b'i' as u16];

/// Lowercases an ASCII code unit; leaves everything else unchanged.
fn ascii_lower(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) {
        c + (b'a' as u16 - b'A' as u16)
    } else {
        c
    }
}

/// Lossy UTF-16 to UTF-8, for logging and JSON output only.
pub fn cstr_lossy(c: &CStr16) -> String {
    c.to_u16_slice()
        .iter()
        .map(|&u| char::from_u32(u as u32).unwrap_or('?'))
        .collect()
}

/// Appends a decimal `u32` to a UTF-16 string (menu countdown).
pub fn push_u32(s: &mut CString16, mut n: u32) {
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
    if i == digits.len() {
        s.push(Char16::try_from('0').expect("'0' is a valid Char16"));
        return;
    }
    for &d in &digits[i..] {
        s.push(Char16::try_from(d as char).expect("ASCII digit is a valid Char16"));
    }
}
