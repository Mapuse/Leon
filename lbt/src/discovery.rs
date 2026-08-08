//! Generic block-device / ESP / boot-entry discovery.
//!
//! Everything here is discovered from the live system — no device, CPU, GPU,
//! partition, or OS is assumed. ESPs are found by enumerating every block
//! device (`lsblk`) and matching the FAT filesystem or the standard GPT ESP
//! GUID; boot entries are read from *every* mounted ESP's real `EFI` tree.

use std::collections::HashSet;
use std::io::Seek;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

/// GPT partition-type GUID for an EFI System Partition.
pub const ESP_GPT_UUID: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";

/// One block device as reported by `lsblk` (or the sysfs fallback).
#[derive(Debug, Clone)]
pub struct BlockDev {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub fstype: Option<String>,
    pub parttype: Option<String>,
    pub label: Option<String>,
    pub uuid: Option<String>,
    pub mountpoint: Option<String>,
}

impl BlockDev {
    pub fn is_esp(&self) -> bool {
        self.fstype.as_deref() == Some("vfat") || self.parttype.as_deref() == Some(ESP_GPT_UUID)
    }
}

/// An EFI System Partition found on this machine.
#[derive(Debug, Clone)]
pub struct EspVolume {
    pub path: String,
    pub mountpoint: Option<PathBuf>,
    pub label: Option<String>,
    pub uuid: Option<String>,
}

impl EspVolume {
    pub fn describe(&self) -> String {
        let id = match (&self.label, &self.uuid) {
            (Some(l), Some(u)) => format!("{l} {u}"),
            (Some(l), None) => l.clone(),
            (None, Some(u)) => u.clone(),
            (None, None) => String::new(),
        };
        match &self.mountpoint {
            Some(mp) => {
                format!("{path} [{id}] -> {mp}", path = self.path, mp = mp.display())
            }
            None => format!(
                "{} [{id}] detected, not mounted (reading needs root)",
                self.path
            ),
        }
    }
}

/// A real OS / bootloader entry discovered on an ESP.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BootEntry {
    pub label: String,
    /// Path relative to the ESP root, forward slashes, e.g. `EFI/BOOT/BOOTX64.EFI`.
    pub path: String,
}

/// Enumerates every block device on the machine via `lsblk` when available,
/// otherwise from the live mount table.
pub fn discover_block_devices() -> Result<Vec<BlockDev>> {
    match lsblk() {
        Ok(devs) => Ok(devs),
        Err(_) => block_devices_from_mounts(),
    }
}

/// Finds every ESP on the machine, from all block devices via `lsblk` when
/// available, otherwise from the live mount table.
pub fn discover_esp_volumes() -> Result<Vec<EspVolume>> {
    let devs = discover_block_devices()?;
    let mut seen = HashSet::new();
    Ok(devs
        .into_iter()
        .filter(BlockDev::is_esp)
        .filter_map(|d| {
            if !seen.insert(d.path.clone()) {
                return None;
            }
            Some(EspVolume {
                path: d.path,
                mountpoint: d.mountpoint.map(PathBuf::from),
                label: d.label,
                uuid: d.uuid,
            })
        })
        .collect())
}

/// Discovers boot entries on every mounted ESP: every `.efi` file in the real
/// `EFI` tree, plus systemd-boot and GRUB menu entries.
pub fn discover_boot_entries(esps: &[EspVolume]) -> Result<Vec<BootEntry>> {
    let mut esp_set = HashSet::new();
    for esp in esps {
        let Some(mp) = &esp.mountpoint else {
            continue;
        };
        let efi = mp.join("EFI");
        if !efi.is_dir() {
            continue;
        }
        collect_efi_files(&efi, &efi, &mut esp_set);
        collect_systemd_boot(mp, &mut esp_set);
        collect_grub(mp, &efi, &mut esp_set);
    }
    // Also scan common /boot locations for EFI-stub kernels or standalone
    // EFI binaries that aren't on a mounted ESP. This helps detect OS kernels
    // that were installed to /boot (non-ESP) but are EFI-capable. When a file
    // was already found on a mounted ESP (e.g. /boot/efi/EFI/... from the ESP
    // scan vs /boot/efi/... from the /boot scan) the ESP-relative form wins;
    // dedup by canonical path so the same physical file never appears twice.
    let mut boot_set = HashSet::new();
    collect_boot_dir(&mut boot_set);
    Ok(merge_entries(esps, esp_set, boot_set))
}

/// Merges ESP-scan and /boot-scan entries, deduplicating by (label, canonical
/// path) so the same physical file reported under two spellings never appears
/// twice (the ESP-relative form wins). Distinct entries that share a file but
/// not a label — e.g. several GRUB `menuentry`s in one `grub.cfg` — are kept.
fn merge_entries(
    esps: &[EspVolume],
    esp_set: HashSet<BootEntry>,
    boot_set: HashSet<BootEntry>,
) -> Vec<BootEntry> {
    let mut entries: Vec<BootEntry> = Vec::new();
    let mut seen: HashSet<(String, std::path::PathBuf)> = HashSet::new();
    for e in esp_set {
        if seen.insert((e.label.clone(), esp_file_key(esps, &e))) {
            entries.push(e);
        }
    }
    for e in boot_set {
        let key = (
            e.label.clone(),
            real_path(Path::new(&e.path)).unwrap_or_else(|| std::path::PathBuf::from(&e.path)),
        );
        if seen.insert(key) {
            entries.push(e);
        }
    }
    entries.sort_by_key(|e| e.label.clone());
    entries
}

/// The canonical path of an ESP-scan entry. ESP entries use paths relative to
/// the ESP mountpoint (e.g. `EFI/BOOT/BOOTX64.EFI`), so they are resolved
/// against each mounted ESP before canonicalizing.
fn esp_file_key(esps: &[EspVolume], e: &BootEntry) -> std::path::PathBuf {
    for esp in esps {
        if let Some(mp) = &esp.mountpoint
            && let Ok(c) = std::fs::canonicalize(mp.join(&e.path))
        {
            return c;
        }
    }
    real_path(Path::new(&e.path)).unwrap_or_else(|| std::path::PathBuf::from(&e.path))
}

/// Resolves `path` to a canonical, symlink-free form so two spellings of the
/// same file (e.g. `EFI/BOOT/BOOTX64.EFI` on a mounted ESP and the absolute
/// `/boot/efi/EFI/BOOT/BOOTX64.EFI`) compare equal.
fn real_path(path: &Path) -> Option<std::path::PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// Scan common /boot paths for EFI-capable files and add them as entries.
fn collect_boot_dir(out: &mut HashSet<BootEntry>) {
    const PATHS: [&str; 3] = ["/boot", "/boot/efi", "/boot/EFI"];
    let paths: Vec<&std::path::Path> = PATHS.iter().map(std::path::Path::new).collect();
    collect_boot_dir_paths(&paths, out);
}

/// Scan configured paths for EFI-capable files and add them as entries.
fn collect_boot_dir_paths(paths: &[&Path], out: &mut HashSet<BootEntry>) {
    for dir in paths {
        walk_boot_dir(dir, out);
    }
}

/// Recursively scans `dir` (skipping symlinks) for EFI-capable files.
fn walk_boot_dir(dir: &Path, out: &mut HashSet<BootEntry>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let Ok(ft) = ent.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let path = ent.path();
        if path.is_dir() {
            walk_boot_dir(&path, out);
            continue;
        }
        if let Some(label) = boot_label(&path) {
            out.insert(BootEntry {
                label,
                path: path.to_string_lossy().to_string(),
            });
        }
    }
}

/// The label to show for `path`, if it is an EFI-capable file: `.efi` files
/// must validate as PE/COFF (`MZ` + `PE\0\0`); other files only if they look
/// like an EFI-stub kernel (ELF with the ASCII marker "efi stub" in the first
/// 64 KiB).
fn boot_label(path: &Path) -> Option<String> {
    let is_efi = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("efi"));
    if (is_efi && is_pe_executable(path)) || is_elf_efi_stub(path) {
        return path
            .file_stem()
            .or_else(|| path.file_name())
            .map(|s| s.to_string_lossy().to_string());
    }
    None
}

fn is_elf_efi_stub(path: &Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = Vec::new();
    let _ = f.by_ref().take(64 * 1024).read_to_end(&mut buf);
    if buf.len() >= 4 && &buf[0..4] == b"\x7fELF" {
        let low = buf.to_ascii_lowercase();
        return low.windows(8).any(|w| w == b"efi stub");
    }
    false
}

/// Return true if the given path is a PE/COFF executable (basic DOS "MZ" +
/// PE\0\0 header verification).
fn is_pe_executable(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut dos = [0u8; 64];
    if f.read_exact(&mut dos).is_err() {
        return false;
    }
    if &dos[0..2] != b"MZ" {
        return false;
    }
    // e_lfanew is a little-endian u32 at offset 0x3c
    let e_lfanew = u32::from_le_bytes([dos[0x3c], dos[0x3d], dos[0x3e], dos[0x3f]]) as usize;
    // Seek and read 4 bytes at e_lfanew for PE\0\0
    if f.seek(std::io::SeekFrom::Start(e_lfanew as u64)).is_err() {
        return false;
    }
    let mut pe = [0u8; 4];
    if f.read_exact(&mut pe).is_err() {
        return false;
    }
    &pe == b"PE\0\0"
}

fn lsblk() -> Result<Vec<BlockDev>> {
    let out = std::process::Command::new("lsblk")
        .args([
            "-bJo",
            "NAME,PATH,TYPE,FSTYPE,PARTTYPE,LABEL,UUID,MOUNTPOINT",
        ])
        .output()
        .context("running lsblk")?;
    if !out.status.success() {
        return Err(anyhow!("lsblk exited with {}", out.status));
    }
    parse_lsblk(&out.stdout)
}

fn parse_lsblk(bytes: &[u8]) -> Result<Vec<BlockDev>> {
    let json: serde_json::Value =
        serde_json::from_slice(bytes).context("parsing lsblk JSON output")?;
    let mut devs = Vec::new();
    collect_devices(&json, &mut devs);
    Ok(devs)
}

fn collect_devices(value: &serde_json::Value, out: &mut Vec<BlockDev>) {
    let Some(arr) = value
        .get("blockdevices")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for node in arr {
        out.push(dev_from_json(node));
        collect_children(node, out);
    }
}

fn collect_children(node: &serde_json::Value, out: &mut Vec<BlockDev>) {
    let Some(children) = node.get("children").and_then(serde_json::Value::as_array) else {
        return;
    };
    for child in children {
        out.push(dev_from_json(child));
    }
}

fn dev_from_json(node: &serde_json::Value) -> BlockDev {
    let name = get_str(node, "name").unwrap_or_default();
    BlockDev {
        name: name.clone(),
        path: get_str(node, "path").unwrap_or_else(|| format!("/dev/{name}")),
        kind: get_str(node, "type").unwrap_or_default(),
        fstype: get_str(node, "fstype"),
        parttype: get_str(node, "parttype"),
        label: get_str(node, "label"),
        uuid: get_str(node, "uuid"),
        mountpoint: get_str(node, "mountpoint"),
    }
}

fn get_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .map(String::from)
}

/// Fallback when `lsblk` is unavailable: walk the live mount table and treat
/// every mount whose root holds an `EFI` directory as an ESP.
fn block_devices_from_mounts() -> Result<Vec<BlockDev>> {
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut devs = Vec::new();
    for line in mounts.lines() {
        let mut it = line.split_ascii_whitespace();
        let (Some(src), Some(dest)) = (it.next(), it.next()) else {
            continue;
        };
        let is_esp = PathBuf::from(dest).join("EFI").is_dir();
        devs.push(BlockDev {
            name: src.rsplit('/').next().unwrap_or(src).to_string(),
            path: src.to_string(),
            kind: if src.starts_with("/dev/") {
                "part".to_string()
            } else {
                "other".to_string()
            },
            fstype: is_esp.then(|| "vfat".to_string()),
            parttype: is_esp.then(|| ESP_GPT_UUID.to_string()),
            label: None,
            uuid: None,
            mountpoint: Some(dest.to_string()),
        });
    }
    Ok(devs)
}

fn collect_efi_files(dir: &Path, efi_root: &Path, out: &mut HashSet<BootEntry>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let p = entry.path();
        if p.is_dir() {
            collect_efi_files(&p, efi_root, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("efi")) {
            let label = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let rel = p
                .strip_prefix(efi_root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            out.insert(BootEntry {
                label,
                path: format!("EFI/{rel}"),
            });
        }
    }
}

fn collect_systemd_boot(mp: &Path, out: &mut HashSet<BootEntry>) {
    let entries_dir = mp.join("EFI/systemd/loader/entries");
    let Ok(rd) = std::fs::read_dir(&entries_dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("conf"))
        {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&p) else {
            continue;
        };
        let label = content
            .lines()
            .find_map(|l| l.trim().strip_prefix("title "))
            .map(|t| t.trim().to_string());
        let Some(label) = label else {
            continue;
        };
        let path = p
            .strip_prefix(mp)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        out.insert(BootEntry { label, path });
    }
}

fn collect_grub(mp: &Path, efi_root: &Path, out: &mut HashSet<BootEntry>) {
    let mut cfgs = Vec::new();
    find_grub_cfgs(efi_root, &mut cfgs);
    for cfg in cfgs {
        let Ok(content) = std::fs::read_to_string(&cfg) else {
            continue;
        };
        for line in content.lines() {
            let t = line.trim_start();
            let Some(rest) = t.strip_prefix("menuentry ") else {
                continue;
            };
            let name = rest
                .strip_prefix('\'')
                .and_then(|s| s.split('\'').next())
                .or_else(|| rest.strip_prefix('"').and_then(|s| s.split('"').next()));
            let Some(name) = name else {
                continue;
            };
            let path = cfg
                .strip_prefix(mp)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            out.insert(BootEntry {
                label: name.to_string(),
                path,
            });
        }
    }
}

fn find_grub_cfgs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let p = entry.path();
        if p.is_dir() {
            find_grub_cfgs(&p, out);
        } else if p
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("grub.cfg"))
        {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lsblk_json() {
        let json = br#"{"blockdevices":[
            {"name":"sda","path":"/dev/sda","type":"disk","fstype":null,"parttype":null,"label":null,"uuid":null,"mountpoint":null,
             "children":[
                {"name":"sda1","path":"/dev/sda1","type":"part","fstype":"vfat","parttype":"c12a7328-f81f-11d2-ba4b-00a0c93ec93b","label":null,"uuid":"A663-9EF0","mountpoint":"/boot/efi"},
                {"name":"sda2","path":"/dev/sda2","type":"part","fstype":"ext4","parttype":"0fc63daf-8483-4772-8e79-3d69d8477de4","label":null,"uuid":null,"mountpoint":"/"}
             ]},
            {"name":"loop0","path":"/dev/loop0","type":"loop","fstype":"squashfs","parttype":null,"label":null,"uuid":null,"mountpoint":"/snap/x"}
        ]}"#;
        let devs = parse_lsblk(json).unwrap();
        assert_eq!(devs.len(), 4);
        let esp = devs.iter().find(|d| d.name == "sda1").unwrap();
        assert!(esp.is_esp());
        assert_eq!(esp.mountpoint.as_deref(), Some("/boot/efi"));
        assert!(!devs.iter().find(|d| d.name == "sda2").unwrap().is_esp());
        assert!(!devs.iter().find(|d| d.name == "loop0").unwrap().is_esp());
    }

    #[test]
    fn finds_efi_boot_files() {
        let dir = std::env::temp_dir().join("lbt_esp_test");
        std::fs::create_dir_all(dir.join("EFI/BOOT")).unwrap();
        std::fs::create_dir_all(dir.join("EFI/ubuntu")).unwrap();
        std::fs::write(dir.join("EFI/BOOT/BOOTX64.EFI"), b"x").unwrap();
        std::fs::write(dir.join("EFI/BOOT/BOOTX64.jpg"), b"x").unwrap();
        std::fs::write(dir.join("EFI/ubuntu/shimx64.efi"), b"x").unwrap();
        std::fs::write(
            dir.join("EFI/ubuntu/grub.cfg"),
            b"set timeout=5\nmenuentry 'Ubuntu' {\n}\nmenuentry \"Advanced options\" {\n}\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("EFI/systemd/loader/entries")).unwrap();
        std::fs::write(
            dir.join("EFI/systemd/loader/entries/cudane.conf"),
            b"title Cudane Linux\nefi /EFI/leon/kernel.efi\n",
        )
        .unwrap();

        let mut set = HashSet::new();
        collect_efi_files(&dir.join("EFI"), &dir.join("EFI"), &mut set);
        collect_systemd_boot(&dir, &mut set);
        collect_grub(&dir, &dir.join("EFI"), &mut set);

        let entries = discover_boot_entries(&[EspVolume {
            path: "/dev/test".to_string(),
            mountpoint: Some(dir.clone()),
            label: None,
            uuid: None,
        }])
        .unwrap();

        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"BOOTX64"));
        assert!(labels.contains(&"shimx64"));
        assert!(labels.contains(&"Ubuntu"));
        assert!(labels.contains(&"Cudane Linux"));
        assert!(!labels.contains(&"BOOTX64.jpg"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn merge_dedups_esp_and_boot_scan_entries() {
        let dir = std::env::temp_dir().join("lbt_merge_test");
        std::fs::create_dir_all(dir.join("EFI/BOOT")).unwrap();
        let pe_path = dir.join("EFI/BOOT/BOOTX64.EFI");
        std::fs::write(&pe_path, b"MZ\x00\x00").unwrap();

        let esps = [EspVolume {
            path: "/dev/test".to_string(),
            mountpoint: Some(dir.clone()),
            label: None,
            uuid: None,
        }];

        // The ESP scan reports a relative path; the /boot scan reports the
        // absolute path to the *same* physical file (e.g. when /boot/efi is a
        // mounted ESP). Both must collapse to a single entry.
        let mut esp_set = HashSet::new();
        esp_set.insert(BootEntry {
            label: "BOOTX64".to_string(),
            path: "EFI/BOOT/BOOTX64.EFI".to_string(),
        });
        let mut boot_set = HashSet::new();
        boot_set.insert(BootEntry {
            label: "BOOTX64".to_string(),
            path: pe_path.to_string_lossy().to_string(),
        });

        let entries = merge_entries(&esps, esp_set, boot_set);
        assert_eq!(entries.len(), 1, "same file must appear once: {entries:?}");
        assert_eq!(entries[0].path, "EFI/BOOT/BOOTX64.EFI");
        assert!(entries[0].path.starts_with("EFI/"), "ESP form wins");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_pe_and_elf_candidates() {
        let dir = std::env::temp_dir().join("lbt_boot_test");
        let _ = std::fs::create_dir_all(&dir);
        // Create a fake PE/COFF .efi file: MZ header + e_lfanew pointing to PE\0\0
        let pe_path = dir.join("fake.efi");
        let mut pe = vec![0u8; 0x80];
        pe[0..2].copy_from_slice(b"MZ");
        let e_lfanew: u32 = 0x40;
        pe[0x3c..0x40].copy_from_slice(&e_lfanew.to_le_bytes());
        // write PE signature at e_lfanew
        if pe.len() < (e_lfanew as usize + 4) {
            pe.resize(e_lfanew as usize + 4, 0);
        }
        pe[e_lfanew as usize..e_lfanew as usize + 4].copy_from_slice(b"PE\0\0");
        std::fs::write(&pe_path, &pe).unwrap();

        // Create an ELF file with 'EFI stub' marker in the body
        let elf_path = dir.join("vmlinuz-efi");
        let mut elf = vec![0u8; 16];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf.extend_from_slice(b"some header and then EFI stub marker here");
        std::fs::write(&elf_path, &elf).unwrap();

        assert!(is_pe_executable(&pe_path));
        assert!(is_elf_efi_stub(&elf_path));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovers_efi_stub_files_from_boot_paths() {
        let dir = std::env::temp_dir().join("lbt_boot_dir_test");
        let boot_dir = dir.join("boot");
        std::fs::create_dir_all(&boot_dir).unwrap();
        std::fs::create_dir_all(boot_dir.join("subdir")).unwrap();

        let elf_path = boot_dir.join("subdir/kernel");
        let mut elf = vec![0u8; 16];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf.extend_from_slice(b"This is an EFI stub kernel file.");
        std::fs::write(&elf_path, &elf).unwrap();

        let efi_path = boot_dir.join("fallback.efi");
        let mut pe = vec![0u8; 0x80];
        pe[0..2].copy_from_slice(b"MZ");
        let e_lfanew: u32 = 0x40;
        pe[0x3c..0x40].copy_from_slice(&e_lfanew.to_le_bytes());
        pe[e_lfanew as usize..e_lfanew as usize + 4].copy_from_slice(b"PE\0\0");
        std::fs::write(&efi_path, &pe).unwrap();

        let mut set = HashSet::new();
        collect_boot_dir_paths(&[&boot_dir], &mut set);

        assert!(set.iter().any(|e| e.path.ends_with("kernel")));
        assert!(set.iter().any(|e| e.path.ends_with("fallback.efi")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
