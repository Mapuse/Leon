//! `lbt fs` / `lbt rootfs`: inspect directories and root filesystems.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

fn dir_or_cwd(p: Option<&str>) -> PathBuf {
    match p {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("."),
    }
}

fn entry_kind(md: &std::fs::Metadata) -> char {
    if md.is_dir() {
        'd'
    } else if md.is_symlink() {
        'l'
    } else {
        '-'
    }
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    Ok(entries)
}

pub fn ls(path: Option<&str>) -> Result<()> {
    let p = dir_or_cwd(path);
    if !p.is_dir() {
        bail!("{} is not a directory", p.display());
    }
    println!("{}:", p.display());
    for entry in sorted_entries(&p)? {
        let md = std::fs::metadata(&entry).unwrap_or_else(|_| {
            std::fs::symlink_metadata(&entry).expect("entry metadata")
        });
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        let suffix = if md.is_dir() { "/" } else { "" };
        println!("  {}{}  {:>12}  {}", entry_kind(&md), suffix, md.len(), name);
    }
    Ok(())
}

pub fn info(path: Option<&str>) -> Result<()> {
    let p = dir_or_cwd(path);
    let md = std::fs::metadata(&p).with_context(|| format!("stating {}", p.display()))?;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let kind = if md.is_dir() {
        "directory"
    } else if md.is_file() {
        "file"
    } else {
        "other"
    };
    println!("path: {}", p.display());
    println!("kind: {kind}");
    println!("size: {} bytes", md.len());
    println!("perm: {:o}", md.permissions().mode() & 0o7777);
    println!("ino:  {}", md.ino());
    println!(
        "mod:  {}",
        std::time::SystemTime::now()
            .duration_since(md.modified()?)
            .map(|d| format!("{:.0}s ago", d.as_secs()))
            .unwrap_or_else(|_| "recent".to_string())
    );
    Ok(())
}

pub fn tree(path: Option<&str>, depth: Option<&str>) -> Result<()> {
    let p = dir_or_cwd(path);
    let max: usize = depth.map(|d| d.parse().unwrap_or(2)).unwrap_or(2);
    if !p.is_dir() {
        bail!("{} is not a directory", p.display());
    }
    println!("{}", p.display());
    print_tree(&p, max, 0, "");
    Ok(())
}

fn print_tree(dir: &Path, max: usize, level: usize, indent: &str) {
    let Ok(entries) = sorted_entries(dir) else {
        return;
    };
    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name().unwrap_or_default().to_string_lossy().to_string();
        let last = i + 1 == entries.len();
        let branch = if last { "└── " } else { "├── " };
        let md = std::fs::metadata(entry).ok();
        let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        println!("{indent}{branch}{}{}", name, if is_dir { "/" } else { "" });
        if is_dir && level + 1 < max {
            let sub = if last { "    " } else { "│   " };
            print_tree(entry, max, level + 1, &format!("{indent}{sub}"));
        }
    }
}

/// Boot-critical paths inside a rootfs.
fn rootfs_paths() -> &'static [(&'static str, &'static str)] {
    &[
        ("EFI/BOOT/BOOTX64.EFI", "x86_64 bootloader"),
        ("EFI/BOOT/BOOTAA64.EFI", "arm64 bootloader"),
        ("EFI/leon/kernel.efi", "EFI-stub kernel"),
        ("EFI/leon/boot.toml", "boot config (optional)"),
    ]
}

pub fn rootfs_show(path: Option<&str>) -> Result<()> {
    let p = dir_or_cwd(path);
    if !p.is_dir() {
        bail!("{} is not a directory", p.display());
    }
    let total = |dir: &Path| -> u64 {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .filter(|m| m.is_file())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    };
    let file_count = |dir: &Path| -> usize {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .filter(|m| m.is_file())
                    .count()
            })
            .unwrap_or(0)
    };
    println!("rootfs: {}", p.display());
    println!("  size: {} bytes", total(&p));
    println!("  files: {}", file_count(&p));
    for (rel, doc) in rootfs_paths() {
        let full = p.join(rel);
        let present = full.is_file();
        println!("  {rel} [{doc}]: {}", if present { "present" } else { "missing" });
    }
    Ok(())
}

pub fn rootfs_check(path: Option<&str>) -> Result<()> {
    let p = dir_or_cwd(path);
    if !p.is_dir() {
        bail!("{} is not a directory", p.display());
    }
    let mut missing = Vec::new();
    let boot_x64 = p.join("EFI/BOOT/BOOTX64.EFI");
    let boot_aa64 = p.join("EFI/BOOT/BOOTAA64.EFI");
    let kernel = p.join("EFI/leon/kernel.efi");
    if !boot_x64.is_file() && !boot_aa64.is_file() {
        missing.push("EFI/BOOT/BOOTX64.EFI or BOOTAA64.EFI");
    }
    if !kernel.is_file() {
        missing.push("EFI/leon/kernel.efi");
    }
    if missing.is_empty() {
        println!("rootfs OK: {}", p.display());
        Ok(())
    } else {
        bail!(
            "rootfs incomplete at {}: missing {}",
            p.display(),
            missing.join(", ")
        );
    }
}

pub fn rootfs_tree(path: Option<&str>, depth: Option<&str>) -> Result<()> {
    tree(path, depth)
}
