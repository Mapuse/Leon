//! `lbc config`: read, write and reset the boot config, plus the default-entry
//! shortcut and saved config profiles.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::boot_config::{BootConfig, boot_config, boot_config_path};

/// Print one key (or the whole config, like `boot config list`).
pub fn get(key: Option<&str>) -> Result<()> {
    let cfg = boot_config()?;
    match key {
        Some(k) => match cfg.field(k) {
            Some(v) => println!("{v}"),
            None => bail!("no such boot config key: {k}"),
        },
        None => println!("{}", toml::to_string(&cfg).unwrap_or_default().trim_end()),
    }
    Ok(())
}

/// Set one key and persist it to the host copy (mirrored to mounted ESPs).
pub fn set(key: &str, value: &str) -> Result<()> {
    let mut cfg = boot_config()?;
    cfg.set_field(key, value)?;
    cfg.save()?;
    println!("{key} = {value}");
    Ok(())
}

/// Print the whole boot config.
pub fn list() -> Result<()> {
    get(None)
}

/// Reset the boot config to defaults.
pub fn reset() -> Result<()> {
    BootConfig::default().save()?;
    println!(
        "boot config reset to defaults ({})",
        boot_config_path().display()
    );
    Ok(())
}

/// Print the resolved boot config path.
pub fn path() -> Result<()> {
    println!("{}", boot_config_path().display());
    Ok(())
}

/// Mirror the boot config file onto a mounted ESP (used by `boot stage` after
/// copying the staged tree). Best-effort: the path comes from the config
/// module's own sync, so a missing ESP just means a note.
pub fn sync_to_esp(mount: &std::path::Path) -> Result<()> {
    let cfg = boot_config()?;
    let content = toml::to_string(&cfg).context("serializing boot config")?;
    let target = mount.join("EFI").join("leon").join("boot.toml");
    std::fs::create_dir_all(target.parent().expect("has a parent"))
        .and_then(|()| std::fs::write(&target, content))
        .with_context(|| format!("writing {}", target.display()))?;
    Ok(())
}

/// Print the current default boot entry.
pub fn default_get() -> Result<()> {
    let cfg = boot_config()?;
    match cfg.field("default_entry") {
        Some(v) => println!("{v}"),
        None => println!("(unset)"),
    }
    Ok(())
}

/// Set the default boot entry (a `config set default_entry` convenience).
pub fn default_set(name: &str) -> Result<()> {
    set("default_entry", name)
}

/// The directory holding saved config profiles, next to the live boot config.
fn profiles_dir() -> Result<PathBuf> {
    let dir = boot_config_path()
        .parent()
        .expect("boot config path has a parent")
        .join("profiles");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

fn profile_path(name: &str) -> PathBuf {
    profiles_dir()
        .map(|d| d.join(format!("{name}.toml")))
        .unwrap_or_else(|_| boot_config_path().with_file_name(format!("{name}.toml")))
}

/// Save the current boot config as a named profile.
pub fn profile_save(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("profile name required");
    }
    let cfg = boot_config()?;
    let content = toml::to_string(&cfg).context("serializing boot config")?;
    let path = profile_path(name.trim());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, &content).with_context(|| format!("writing {}", path.display()))?;
    println!("saved profile `{name}` -> {}", path.display());
    Ok(())
}

/// List every saved config profile.
pub fn profile_list() -> Result<()> {
    let dir = profiles_dir()?;
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".toml").map(|n| n.to_string())
        })
        .collect();
    names.sort();
    if names.is_empty() {
        println!("no config profiles saved");
        return Ok(());
    }
    for name in names {
        println!("{name}");
    }
    Ok(())
}

/// Restore the boot config from a saved profile.
pub fn profile_load(name: &str) -> Result<()> {
    let path = profile_path(name);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading profile {}", path.display()))?;
    let cfg: BootConfig =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    cfg.save()?;
    println!("loaded profile `{name}` ({})", boot_config_path().display());
    Ok(())
}

/// Delete a saved config profile.
pub fn profile_delete(name: &str) -> Result<()> {
    let path = profile_path(name);
    std::fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
    println!("deleted profile `{name}`");
    Ok(())
}
