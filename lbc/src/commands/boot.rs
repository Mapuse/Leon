//! `lbc boot`: stage the ESP tree, report the boot volume layout, one-shot
//! boot, and ESP uploads.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::commands::config;
use crate::commands::util;
use lbt::discovery;

/// Stage a ready-to-boot ESP tree (bootloader at `\EFI\BOOT`, EFI-stub kernel
/// at `\EFI\leon\kernel.efi`, boot config mirrored) under `--dest` or
/// `build/esp`, mirroring the Makefile `stage` target in pure Rust.
pub fn stage(dest: Option<&str>, arch: Option<&str>) -> Result<()> {
    let repo = util::repo_root()?;
    let arch = arch
        .map(str::to_string)
        .unwrap_or_else(|| util::host_arch().to_string());
    let target = util::uefi_target(&arch)?;
    let boot_file = util::boot_file(&arch)?;
    let profile = "release";

    let bl_efi = repo
        .join("target")
        .join(target)
        .join(profile)
        .join("lbl.efi");
    let kernel_efi = repo
        .join("kernel")
        .join("target")
        .join(target)
        .join(profile)
        .join("lbl-kernel.efi");
    if !bl_efi.is_file() || !kernel_efi.is_file() {
        bail!(
            "build outputs missing ({} / {}); run `lbt build all` first",
            bl_efi.display(),
            kernel_efi.display()
        );
    }

    let dest_dir = dest
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("build").join("esp"));
    let boot_dir = dest_dir.join("EFI").join("BOOT");
    let leon_dir = dest_dir.join("EFI").join("leon");
    std::fs::create_dir_all(&boot_dir).with_context(|| format!("creating {}", boot_dir.display()))?;
    std::fs::create_dir_all(&leon_dir)
        .with_context(|| format!("creating {}", leon_dir.display()))?;

    std::fs::copy(&bl_efi, boot_dir.join(boot_file))
        .with_context(|| format!("copying {}", bl_efi.display()))?;
    std::fs::copy(&kernel_efi, leon_dir.join("kernel.efi"))
        .with_context(|| format!("copying {}", kernel_efi.display()))?;
    if let Err(e) = config::sync_to_esp(&dest_dir) {
        eprintln!("note: could not mirror boot config onto the staged ESP ({e:#})");
    }

    println!("staged ESP tree under {}", dest_dir.display());
    println!("  {}/EFI/BOOT/{boot_file}", dest_dir.display());
    println!("  {}/EFI/leon/kernel.efi", dest_dir.display());
    println!("  {}/EFI/leon/boot.toml", dest_dir.display());
    Ok(())
}

/// Report the boot volume layout: host config, detected ESPs, and the paths
/// the bootloader expects on the boot volume.
pub fn info() -> Result<()> {
    println!("Leon boot volume layout");
    println!(
        "  host config: {}",
        crate::boot_config::boot_config_path().display()
    );
    let esps = discovery::discover_esp_volumes().unwrap_or_default();
    if esps.is_empty() {
        println!("  mounted ESP: none detected");
    } else {
        for e in &esps {
            let mount = e
                .mountpoint
                .as_ref()
                .map(|m| m.display().to_string())
                .unwrap_or_else(|| "not mounted".to_string());
            println!("  ESP: {} ({mount})", e.path);
        }
    }
    println!("  on-ESP config: \\EFI\\leon\\boot.toml");
    println!("  bootloader: \\EFI\\BOOT\\BOOTX64.EFI (amd64) / BOOTAA64.EFI (arm64)");
    println!("  kernel: \\EFI\\leon\\kernel.efi");
    println!("  boot log: \\var\\logs\\leon\\log.md");
    Ok(())
}

/// One-shot boot of a single entry: validate the label and report the intended
/// boot. The bootloader itself performs the one-shot selection interactively
/// in the boot-manager menu (pause at boot, highlight the entry, press Enter);
/// the boot config deliberately carries no `boot_once_entry` key that the
/// bootloader would have to interpret.
pub fn once(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("an entry label is required");
    }
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    let Some(entry) = entries.iter().find(|e| e.label == name) else {
        bail!("no boot entry labelled `{name}`");
    };
    println!("one-shot boot: {} ({})", entry.label, entry.path);
    println!("to boot it now, pause in the boot-manager menu (Esc at boot),");
    println!("highlight `{}` and press Enter; the next boot reverts to the default.", entry.label);
    Ok(())
}

/// Mirror the boot config onto a mounted ESP (its root given by `--vol`, or
/// every detected ESP if omitted).
pub fn esp_sync(vol: &str) -> Result<()> {
    let vol = vol.trim();
    if !vol.is_empty() {
        config::sync_to_esp(std::path::Path::new(vol))?;
        println!("boot config mirrored onto {}", vol);
        return Ok(());
    }
    let esps = discovery::discover_esp_volumes()?;
    if esps.is_empty() {
        bail!("no ESP volumes detected to mirror onto");
    }
    let mut synced = 0;
    for esp in &esps {
        if let Some(mount) = &esp.mountpoint {
            config::sync_to_esp(mount)?;
            println!("boot config mirrored onto {}", mount.display());
            synced += 1;
        }
    }
    if synced == 0 {
        bail!("no mounted ESP could be written");
    }
    Ok(())
}
