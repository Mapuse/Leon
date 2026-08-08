//! `lbt build`: build the bootloader, EFI-stub kernel and the lbt host tool,
//! mirroring the Makefile targets in pure Rust. `lbt xfer` copies a staged
//! tree onto a mounted ESP or into a directory.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use super::util;

/// Build everything: bootloader, kernel and lbt.
pub fn all(arch: Option<&str>) -> Result<()> {
    boot(arch)?;
    kernel(arch)?;
    lbt()?;
    Ok(())
}

/// Build the UEFI bootloader (`lbl`) for the given (or host) architecture.
pub fn boot(arch: Option<&str>) -> Result<()> {
    let repo = util::repo_root()?;
    let arch = arch.map(str::to_string).unwrap_or_else(|| util::host_arch().to_string());
    let target = util::uefi_target(&arch)?;
    println!("building bootloader for {arch} ({target})");
    let status = std::process::Command::new("cargo")
        .current_dir(&repo)
        .env("CARGO_TARGET_DIR", repo.join("target"))
        .args(["build", "--locked", "--target", target, "-p", "leon", "--release"])
        .status()
        .with_context(|| "running cargo build (bootloader)")?;
    if !status.success() {
        bail!("bootloader build failed");
    }
    let bl = repo.join("target").join(target).join("release").join("lbl.efi");
    if !bl.is_file() {
        bail!("bootloader build did not produce {}", bl.display());
    }
    println!("built {}", bl.display());
    Ok(())
}

/// Build the EFI-stub kernel for the given architecture.
pub fn kernel(arch: Option<&str>) -> Result<()> {
    let repo = util::repo_root()?;
    let arch = arch.map(str::to_string).unwrap_or_else(|| util::host_arch().to_string());
    let target = util::uefi_target(&arch)?;
    println!("building EFI-stub kernel for {arch} ({target})");
    let status = std::process::Command::new("make")
        .current_dir(repo.join("kernel"))
        .args(["build"])
        .env("TARGET", target)
        .env("PROFILE", "release")
        .status()
        .with_context(|| "running `make -C kernel build`")?;
    if !status.success() {
        bail!("kernel build failed");
    }
    let k = repo.join("kernel").join("target").join(target).join("release").join("lbl-kernel.efi");
    if !k.is_file() {
        bail!("kernel build did not produce {}", k.display());
    }
    println!("built {}", k.display());
    Ok(())
}

/// Build the lbt host tool (native target).
pub fn lbt() -> Result<()> {
    let repo = util::repo_root()?;
    println!("building lbt host tool");
    let status = std::process::Command::new("cargo")
        .current_dir(&repo)
        .env("CARGO_TARGET_DIR", repo.join("target"))
        .args(["build", "--locked", "-p", "lbt", "--release"])
        .status()
        .with_context(|| "running cargo build (lbt)")?;
    if !status.success() {
        bail!("lbt build failed");
    }
    println!("built {}", repo.join("target").join("release").join("lbt").display());
    Ok(())
}

/// Copy a staged ESP tree onto a mounted ESP volume (`--dest` = mount point).
pub fn xfer_esp(source: Option<&str>, dest: Option<&str>) -> Result<()> {
    let src = PathBuf::from(source.unwrap_or("build/esp"));
    let dest = PathBuf::from(dest.unwrap_or("/mnt/esp"));
    if !src.is_dir() {
        bail!("{} is not a directory; run `lbc boot stage` first", src.display());
    }
    if !dest.is_dir() {
        bail!("{} is not a mounted ESP", dest.display());
    }
    println!("copying {} -> {}", src.display(), dest.display());
    util::copy_tree(&src, &dest)?;
    println!("staged ESP installed on {}", dest.display());
    Ok(())
}

/// Copy a staged tree into a plain directory.
pub fn xfer_stage(source: Option<&str>, dest: Option<&str>) -> Result<()> {
    let src = PathBuf::from(source.unwrap_or("build/esp"));
    let dest = PathBuf::from(dest.unwrap_or("esp"));
    if !src.is_dir() {
        bail!("{} is not a directory", src.display());
    }
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    println!("copying {} -> {}", src.display(), dest.display());
    util::copy_tree(&src, &dest)?;
    println!("copied to {}", dest.display());
    Ok(())
}
