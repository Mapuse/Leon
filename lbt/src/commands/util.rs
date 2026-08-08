//! Internal helpers shared by the command handlers: locating the repo, running
//! host tools, and the common copy/print routines.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Walks up from the current directory to the Leon repository root (the
/// directory containing `env.mk` and the top-level `Makefile`).
pub fn repo_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let mut dir = cwd.as_path();
    loop {
        if dir.join("env.mk").is_file() && dir.join("Makefile").is_file() {
            return Ok(dir.to_path_buf());
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => bail!(
                "could not locate the Leon repository root (env.mk) from {}",
                cwd.display()
            ),
        }
    }
}

/// The host architecture as `amd64` / `arm64`.
pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => {
            // Unsupported hosts default to amd64 so `lbt` still runs; the
            // boot targets are what actually care.
            eprintln!("warning: unsupported host arch {other}, assuming amd64");
            "amd64"
        }
    }
}

/// The UEFI cargo target for a Leon architecture name.
pub fn uefi_target(arch: &str) -> Result<&'static str> {
    match arch {
        "amd64" => Ok("x86_64-unknown-uefi"),
        "arm64" => Ok("aarch64-unknown-uefi"),
        other => bail!("unsupported architecture: {other} (supported: amd64, arm64)"),
    }
}

/// The UEFI-canonical boot file name for a Leon architecture name.
pub fn boot_file(arch: &str) -> Result<&'static str> {
    match arch {
        "amd64" => Ok("BOOTX64.EFI"),
        "arm64" => Ok("BOOTAA64.EFI"),
        other => bail!("unsupported architecture: {other} (supported: amd64, arm64)"),
    }
}

/// Whether a host tool exists and runs (`cmd --version` succeeds).
pub fn tool_available(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Locate a host tool on PATH.
pub fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(cmd);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Run a tool with inherited stdio; errors on a non-zero exit.
pub fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("running `{cmd}`"))?;
    if !status.success() {
        bail!("`{cmd}` exited with {status}");
    }
    Ok(())
}

/// Run a tool and return its trimmed stdout; errors on a non-zero exit.
pub fn capture(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("running `{cmd}`"))?;
    if !out.status.success() {
        bail!(
            "`{cmd}` exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Recursively copy a directory tree (files and symlinks, best-effort).
pub fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            let target = std::fs::read_link(&from)?;
            std::fs::remove_file(&to).ok();
            std::os::unix::fs::symlink(target, &to)?;
        } else if ty.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// The default location of the boot log inside a staged/rootfs ESP tree.
pub fn default_boot_log() -> PathBuf {
    PathBuf::from("build/esp/var/logs/leon/log.md")
}
