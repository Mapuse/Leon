//! `lbt` environment, OS, USB and firmware diagnostics.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::util;
use crate::discovery;

/// Print the host environment and tool availability.
pub fn env_show() -> Result<()> {
    println!("lbt {} on {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS);
    for (k, v) in [
        ("PATH", std::env::var("PATH").unwrap_or_default()),
        ("SHELL", std::env::var("SHELL").unwrap_or_default()),
        ("TERM", std::env::var("TERM").unwrap_or_default()),
        ("USER", std::env::var("USER").unwrap_or_default()),
    ] {
        println!("{k}={v}");
    }
    println!("arch={}", util::host_arch());
    for tool in KEY_TOOLS {
        let present = util::tool_available(tool);
        println!("tool {tool}: {}", if present { "present" } else { "missing" });
    }
    Ok(())
}

const KEY_TOOLS: &[&str] = &[
    "cargo",
    "rustup",
    "dd",
    "mkfs.ext4",
    "mformat",
    "mcopy",
    "xorriso",
    "sgdisk",
    "openssl",
    "sbsign",
    "objcopy",
    "qemu-system-x86_64",
];

/// Verify host prerequisites for building and imaging.
pub fn env_check() -> Result<()> {
    let mut missing = Vec::new();
    for tool in KEY_TOOLS {
        if !util::tool_available(tool) {
            missing.push(tool);
        }
    }
    if missing.is_empty() {
        println!("environment OK");
        Ok(())
    } else {
        println!(
            "missing tools: {}",
            missing
                .iter()
                .map(|t| format!("`{t}`"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!("most builds need: rustup + cargo; imaging needs: dd, mkfs.ext4");
        Ok(())
    }
}

/// Locate a host tool and print its path.
pub fn env_tool(name: &str) -> Result<()> {
    match util::which(name) {
        Some(p) => println!("{}", p.display()),
        None => bail!("`{name}` not found on PATH"),
    }
    Ok(())
}

/// The pretty OS name.
pub fn os_id() -> Result<()> {
    println!("{}", os_pretty_name());
    Ok(())
}

fn os_pretty_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Print the OS release description.
pub fn os_release() -> Result<()> {
    for line in std::fs::read_to_string("/etc/os-release")
        .unwrap_or_default()
        .lines()
    {
        println!("{line}");
    }
    Ok(())
}

/// Print the running kernel version.
pub fn os_kernel() -> Result<()> {
    println!("{}", util::capture("uname", &["-r"]).unwrap_or_else(|_| "unknown".into()));
    Ok(())
}

/// List USB / block devices.
pub fn usb_list() -> Result<()> {
    let devs = discovery::discover_block_devices()?;
    if devs.is_empty() {
        println!("no block devices detected");
        return Ok(());
    }
    for d in &devs {
        let esp = if d.is_esp() { " [ESP]" } else { "" };
        println!(
            "{} ({}) {} mount={}{}",
            d.path,
            d.kind,
            d.fstype.as_deref().unwrap_or("none"),
            d.mountpoint.as_deref().unwrap_or("none"),
            esp
        );
    }
    Ok(())
}

/// Detect removable devices (from the `removable` sysfs attribute).
pub fn usb_detect() -> Result<()> {
    let found = removable_blocks();
    if found.is_empty() {
        println!("no removable block devices detected");
        return Ok(());
    }
    for (dev, size) in found {
        println!("{dev}  {}", crate::misc::fmt_size(size));
    }
    Ok(())
}

fn removable_blocks() -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/block") else {
        return out;
    };
    for entry in dir.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("sd") && !name.starts_with("mmcblk") && !name.starts_with("nvme") {
            continue;
        }
        let removable = std::fs::read_to_string(entry.path().join("removable"))
            .map(|v| v.trim() == "1")
            .unwrap_or(false);
        if removable
            && let Ok(md) = std::fs::metadata(format!("/dev/{name}"))
        {
            out.push((format!("/dev/{name}"), md.len()));
        }
    }
    out
}

/// Flash a disk image onto a block device (needs root).
pub fn usb_flash(device: &str, path: &str) -> Result<()> {
    if device.is_empty() || path.is_empty() {
        bail!("usage: lbt usb flash --device /dev/sdX --path image.img");
    }
    if !Path::new(device).exists() {
        bail!("device {device} does not exist");
    }
    let src = Path::new(path);
    if !src.is_file() {
        bail!("image {} is not a file", src.display());
    }
    if !src.metadata()?.len().is_multiple_of(512) {
        bail!("image is not a whole number of 512-byte sectors");
    }
    require_root()?;
    println!("flashing {} -> {device} (this overwrites the device!)", src.display());
    super::util::run(
        "dd",
        &[
            &format!("if={}", src.display()),
            &format!("of={device}"),
            "bs=4M",
            "status=progress",
        ],
    )?;
    super::util::run("sync", &[])?;
    println!("flashed {}", src.display());
    Ok(())
}

/// Errors out unless we are running as root.
fn require_root() -> Result<()> {
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok())
        .unwrap_or(u32::MAX);
    if uid != 0 {
        bail!("this command needs root (flashing a block device); re-run with sudo");
    }
    Ok(())
}

/// Describe the firmware from sysfs/DMI.
pub fn firmware_info() -> Result<()> {
    let efi = Path::new("/sys/firmware/efi").is_dir();
    println!("firmware: {}", if efi { "UEFI" } else { "BIOS/legacy" });
    let sysvendor = read_first("/sys/class/dmi/id/sys_vendor");
    let product = read_first("/sys/class/dmi/id/product_name");
    println!("vendor: {}", sysvendor.unwrap_or_else(|| "unknown".into()));
    println!("product: {}", product.unwrap_or_else(|| "unknown".into()));
    println!("secure boot: {}", sb_status());
    Ok(())
}

/// Report the Secure Boot state from efivarfs.
pub fn firmware_sb() -> Result<()> {
    println!("secure boot: {}", sb_status());
    Ok(())
}

fn sb_status() -> String {
    let sb = std::env::var("efi_secure_boot").ok();
    if let Some(v) = sb {
        return if v == "1" { "enabled".into() } else { "disabled".into() };
    }
    let var = "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
    match std::fs::read(var) {
        Ok(data) if data.len() >= 5 => {
            if data[4] == 1 { "enabled".into() } else { "disabled".into() }
        }
        _ => "not detectable (no UEFI / no efivarfs)".into(),
    }
}

fn read_first(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// List the ACPI tables the firmware exposed.
pub fn firmware_acpi() -> Result<()> {
    let dir = Path::new("/sys/firmware/acpi/tables");
    if !dir.is_dir() {
        println!("no ACPI tables exposed (legacy BIOS?)");
        return Ok(());
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    if names.is_empty() {
        println!("no ACPI tables found");
    } else {
        for n in names {
            println!("{n}");
        }
    }
    Ok(())
}

/// Check network reachability with a plain TCP connect (no external tool).
pub fn net_check() -> Result<()> {
    for host in ["1.1.1.1", "8.8.8.8"] {
        let addr = format!("{host}:443");
        let res = std::net::TcpStream::connect_timeout(
            &addr.parse().with_context(|| format!("parsing {addr}"))?,
            Duration::from_secs(3),
        );
        println!("{host}: {}", if res.is_ok() { "reachable" } else { "unreachable" });
    }
    Ok(())
}
