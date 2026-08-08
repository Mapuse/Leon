//! `lbt image`: build bootable ISO, GPT disk (IMG), and ESP images, from a
//! rootfs directory or from the repo's staged ESP tree.
//!
//! The FAT filesystem and GPT partition-table writers are pure Rust
//! ([`crate::img`]); only the ISO 9660 assembly (xorriso/genisoimage) and the
//! root ext4 partition (`mkfs.ext4 -d`) still call host tools.

use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use super::util;
use crate::img::{fat, gpt};

const SECTOR: u64 = 512;
const MIB: u64 = 1024 * 1024;
/// First partition offset: 1 MiB (sector 2048), keeping the GPT backup area
/// and the primary table well clear.
const ESP_LBA: u64 = 2048;

fn parse_mib(s: Option<&str>, default: u64) -> Result<u64> {
    match s {
        Some(s) => s
            .parse()
            .map_err(|_| anyhow!("`--size` must be a number of MiB (got `{s}`)")),
        None => Ok(default),
    }
}

/// Creates (truncating) a file of exactly `bytes` bytes.
fn create_file(path: &Path, bytes: u64) -> Result<std::fs::File> {
    let f = std::fs::File::options()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.set_len(bytes)?;
    Ok(f)
}

/// Copies the whole contents of `src` into `dst` at byte `offset`.
fn splice_volume(dst: &mut std::fs::File, src: &Path, offset: u64) -> Result<()> {
    let f = std::fs::File::open(src).with_context(|| format!("opening {}", src.display()))?;
    let len = f.metadata()?.len();
    let mut buf = vec![0u8; 1 << 20];
    let mut pos = 0u64;
    while pos < len {
        let chunk = buf.len().min((len - pos) as usize);
        f.read_exact_at(&mut buf[..chunk], pos)?;
        dst.write_all_at(&buf[..chunk], offset + pos)?;
        pos += chunk as u64;
    }
    Ok(())
}

/// The rootfs directory to put into an ESP: `--rootfs` or the staged tree.
fn esp_source(rootfs: Option<&str>) -> Result<(PathBuf, Option<String>)> {
    match rootfs {
        Some(r) => Ok((PathBuf::from(r), None)),
        None => {
            let repo = util::repo_root()?;
            Ok((repo.join("build").join("esp"), Some(format!("from {}", repo.display()))))
        }
    }
}

/// Build an ESP (EFI System Partition) disk image: a single-FAT-partition GPT
/// disk. The FAT filesystem is written natively (no mtools).
pub fn esp(
    rootfs: Option<&str>,
    out: Option<&str>,
    size: Option<&str>,
    arch: Option<&str>,
) -> Result<()> {
    let (src, label_hint) = esp_source(rootfs)?;
    if !src.is_dir() {
        bail!(
            "{} is not a directory{}; use `lbc boot stage` to create it",
            src.display(),
            label_hint.map(|h| format!(" ({h})")).unwrap_or_default()
        );
    }

    let size_mb = parse_mib(size, 64)?;
    let out = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("leon-esp.img"));

    let disk_bytes = size_mb * MIB;
    let last_lba = disk_bytes / SECTOR - 1;
    let part_end_lba = last_lba - 33; // last usable sector
    let part_sectors = part_end_lba - ESP_LBA + 1;
    let part_bytes = part_sectors * SECTOR;

    // The FAT volume, written standalone then spliced under the GPT partition.
    let vol = std::env::temp_dir().join(format!("lbt-esp-{}.img", std::process::id()));
    let _ = std::fs::remove_file(&vol);
    {
        let mut f = create_file(&vol, part_bytes)?;
        fat::write_fat(
            &mut f,
            &fat::FatOptions {
                label: "LEON".to_string(),
                size_bytes: part_bytes,
            },
            Some(&src),
            &[],
        )?;
    }

    let mut disk = create_file(&out, disk_bytes)?;
    gpt::write_gpt(
        &mut disk,
        disk_bytes,
        &[gpt::Partition {
            start_lba: ESP_LBA,
            end_lba: part_end_lba,
            type_guid: gpt::PART_ESP,
            name: "EFI System Partition".to_string(),
        }],
    )?;
    splice_volume(&mut disk, &vol, ESP_LBA * SECTOR)?;
    let _ = std::fs::remove_file(&vol);

    println!("ESP image written: {}", out.display());
    if rootfs.is_none() {
        let arch = arch.map(str::to_string).unwrap_or_else(|| util::host_arch().to_string());
        let boot_file = util::boot_file(&arch)?;
        println!("  bootloader: \\EFI\\BOOT\\{boot_file}");
    }
    println!("  size: {size_mb} MiB GPT with one ESP partition");
    Ok(())
}

/// Build a GPT disk image from a rootfs: a FAT ESP partition plus an ext4
/// root partition populated directly from the rootfs directory.
pub fn img(rootfs: Option<&str>, out: Option<&str>, size: Option<&str>) -> Result<()> {
    let rootfs = rootfs
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("`image img` needs a `--rootfs` directory"))?;
    if !rootfs.is_dir() {
        bail!("{} is not a directory", rootfs.display());
    }
    let size_mb = parse_mib(size, 1024)?;
    let out = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("leon.img"));

    if !util::tool_available("mkfs.ext4") {
        bail!("`image img` needs `mkfs.ext4` (e2fsprogs) to create the root filesystem");
    }

    // Layout: p1 = ESP 64 MiB at 1 MiB, p2 = root to end.
    let esp_mb = 64u64;
    if size_mb < esp_mb + 2 {
        bail!("`--size` must be at least {} MiB", esp_mb + 2);
    }
    let root_mb = size_mb - esp_mb;
    let disk_bytes = size_mb * MIB;
    let last_lba = disk_bytes / SECTOR - 1;
    let esp_sectors = esp_mb * MIB / SECTOR;
    let root_start_lba = ESP_LBA + esp_sectors;
    let root_end_lba = last_lba - 33;
    let esp_bytes = esp_sectors * SECTOR;
    let root_bytes = (root_end_lba - root_start_lba + 1) * SECTOR;

    let tmp = std::env::temp_dir();
    let esp_file = tmp.join(format!("lbt-esp-{}.img", std::process::id()));
    let root_file = tmp.join(format!("lbt-root-{}.img", std::process::id()));
    let _ = std::fs::remove_file(&esp_file);
    let _ = std::fs::remove_file(&root_file);

    // ESP partition: native FAT with the whole rootfs tree.
    {
        let mut f = create_file(&esp_file, esp_bytes)?;
        fat::write_fat(
            &mut f,
            &fat::FatOptions {
                label: "LEON".to_string(),
                size_bytes: esp_bytes,
            },
            Some(&rootfs),
            &[],
        )?;
    }

    // Root partition: mke2fs -d needs no mount and no privileges.
    let root_img = create_file(&root_file, root_bytes)?;
    drop(root_img);
    let status = std::process::Command::new("mkfs.ext4")
        .arg("-F")
        .arg("-q")
        .arg("-d")
        .arg(&rootfs)
        .arg(&root_file)
        .status()
        .with_context(|| "running mkfs.ext4")?;
    if !status.success() {
        bail!("mkfs.ext4 failed to create the root filesystem");
    }

    let mut disk = create_file(&out, disk_bytes)?;
    gpt::write_gpt(
        &mut disk,
        disk_bytes,
        &[
            gpt::Partition {
                start_lba: ESP_LBA,
                end_lba: root_start_lba - 1,
                type_guid: gpt::PART_ESP,
                name: "EFI System Partition".to_string(),
            },
            gpt::Partition {
                start_lba: root_start_lba,
                end_lba: root_end_lba,
                type_guid: gpt::PART_LINUX,
                name: "Leon Root".to_string(),
            },
        ],
    )?;
    splice_volume(&mut disk, &esp_file, ESP_LBA * SECTOR)?;
    splice_volume(&mut disk, &root_file, root_start_lba * SECTOR)?;
    let _ = std::fs::remove_file(&esp_file);
    let _ = std::fs::remove_file(&root_file);

    println!("disk image written: {}", out.display());
    println!("  partition 1: EFI System ({esp_mb} MiB, FAT)");
    println!("  partition 2: Linux root ({root_mb} MiB, ext4)");
    Ok(())
}

/// Build a bootable ISO from a rootfs directory.
pub fn iso(rootfs: Option<&str>, out: Option<&str>, label: Option<&str>) -> Result<()> {
    let rootfs = rootfs
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("`image iso` needs a `--rootfs` directory"))?;
    if !rootfs.is_dir() {
        bail!("{} is not a directory", rootfs.display());
    }
    let out = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("leon.iso"));
    let label = label.unwrap_or("LEON");

    // The UEFI El Torito boot image: a small FAT volume carrying the bootloader.
    let efi_img = std::env::temp_dir().join(format!("lbt-efi-{}.img", std::process::id()));
    let _ = std::fs::remove_file(&efi_img);
    let boot = rootfs.join("EFI").join("BOOT");
    let boot_file = if boot.join("BOOTX64.EFI").is_file() {
        "BOOTX64.EFI"
    } else if boot.join("BOOTAA64.EFI").is_file() {
        "BOOTAA64.EFI"
    } else {
        ""
    };

    let bootable = !boot_file.is_empty();
    // Stage just the bootloader under a tiny EFI/BOOT tree, then format it.
    if bootable {
        let stage = std::env::temp_dir().join(format!("lbt-iso-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&stage);
        std::fs::create_dir_all(stage.join("EFI").join("BOOT"))?;
        std::fs::copy(boot.join(boot_file), stage.join("EFI").join("BOOT").join(boot_file))?;
        let vol_bytes = 16 * MIB;
        let mut f = create_file(&efi_img, vol_bytes)?;
        fat::write_fat(
            &mut f,
            &fat::FatOptions {
                label: label.to_string(),
                size_bytes: vol_bytes,
            },
            Some(&stage),
            &[],
        )?;
        let _ = std::fs::remove_dir_all(&stage);
    } else {
        eprintln!(
            "note: no BOOTX64.EFI/BOOTAA64.EFI in {}/EFI/BOOT — building a data-only ISO",
            rootfs.display()
        );
    }

    if util::tool_available("xorriso") {
        let mut args: Vec<String> = vec![
            "-as".into(),
            "mkisofs".into(),
            "-V".into(),
            label.into(),
            "-o".into(),
            out.display().to_string(),
            "-J".into(),
            "-R".into(),
        ];
        if bootable {
            args.extend([
                "-e".into(),
                efi_img.display().to_string(),
                "-no-emul-boot".into(),
                "-isohybrid-gpt-basdat".into(),
            ]);
            if let Some(mbr) = isohybris_mbr() {
                args.extend(["-isohybrid-mbr".into(), mbr]);
            }
        }
        args.push(rootfs.display().to_string());
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        util::run("xorriso", &argv)?;
    } else if util::tool_available("genisoimage") {
        eprintln!("note: xorriso not found, falling back to genisoimage");
        let mut args: Vec<String> = vec![
            "-R".into(),
            "-J".into(),
            "-V".into(),
            label.into(),
            "-o".into(),
            out.display().to_string(),
        ];
        if bootable {
            args.extend([
                "-e".into(),
                efi_img.display().to_string(),
                "-no-emul-boot".into(),
            ]);
        }
        args.push(rootfs.display().to_string());
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        util::run("genisoimage", &argv)?;
    } else {
        bail!(
            "no ISO tool found (need `xorriso` or `genisoimage`); install one to build ISOs"
        );
    }
    let _ = std::fs::remove_file(&efi_img);
    println!("ISO image written: {}", out.display());
    Ok(())
}

/// A suitable `isohybrid-mbr` template if one is installed (used to make the
/// ISO bootable from USB in addition to El Torito).
fn isohybris_mbr() -> Option<String> {
    const CANDIDATES: &[&str] = &[
        "/usr/lib/ISOLINUX/isohdpfx.bin",
        "/usr/lib/syslinux/mbr/isohdpfx.bin",
        "/usr/lib/PXELINUX/isohdpfx.bin",
        "/usr/share/syslinux/isohdpfx.bin",
    ];
    CANDIDATES
        .iter()
        .find(|p| Path::new(p).is_file())
        .map(|p| p.to_string())
}
