//! Host-side boot config: `~/.config/leon/boot.toml`, mirrored onto every
//! mounted ESP as `\EFI\leon\boot.toml` for the bootloader.
//!
//! The keys lbt manages (`timeout`, `default_entry`, `theme`, `splash`,
//! `entries_file`) are exactly the keys `leon_common::boot_config` parses, so
//! what lbt writes is always what the bootloader reads.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

use crate::discovery;

pub const CONFIG_KEYS: [&str; 5] = [
    "timeout",
    "default_entry",
    "theme",
    "splash",
    "entries_file",
];

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct BootConfig {
    #[serde(default)]
    timeout: Option<u32>,
    #[serde(default)]
    default_entry: Option<String>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    splash: Option<bool>,
    #[serde(default)]
    entries_file: Option<String>,
}

impl BootConfig {
    pub fn field(&self, key: &str) -> Option<String> {
        match key {
            "timeout" => self.timeout.map(|v| v.to_string()),
            "default_entry" => self.default_entry.clone(),
            "theme" => self.theme.clone(),
            "splash" => self.splash.map(|v| v.to_string()),
            "entries_file" => self.entries_file.clone(),
            _ => None,
        }
    }

    pub fn set_field(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "timeout" => {
                let n: u32 = value
                    .parse()
                    .map_err(|_| anyhow!("timeout must be a number of seconds"))?;
                self.timeout = Some(n);
            }
            "default_entry" => self.default_entry = Some(value.to_string()),
            "theme" => self.theme = Some(value.to_string()),
            "splash" => {
                // Must match what the bootloader's parser accepts
                // (`leon_common::boot_config`), hence real booleans only.
                let b = match value {
                    "true" => true,
                    "false" => false,
                    _ => bail!("splash must be true or false"),
                };
                self.splash = Some(b);
            }
            "entries_file" => self.entries_file = Some(value.to_string()),
            _ => bail!(
                "no such boot config key: {key} (expected one of {})",
                CONFIG_KEYS.join(", ")
            ),
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let path = boot_config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let content = toml::to_string(self).context("serializing boot config")?;
        std::fs::write(&path, &content).with_context(|| format!("writing {}", path.display()))?;
        sync_config_to_esps(&content);
        Ok(())
    }
}

/// Mirrors the boot config onto every mounted EFI System Partition so the
/// bootloader (which reads `\EFI\leon\boot.toml` from the boot volume) picks
/// it up. Best-effort: an unwritable or unmounted ESP only gets a note.
fn sync_config_to_esps(content: &str) {
    let esps = match discovery::discover_esp_volumes() {
        Ok(esps) => esps,
        Err(_) => {
            eprintln!(
                "note: could not scan for EFI System Partitions; boot config kept in ~/.config/leon/ only"
            );
            return;
        }
    };
    let mut synced = 0;
    for esp in &esps {
        let Some(mount) = &esp.mountpoint else {
            continue;
        };
        let target = mount.join("EFI").join("leon").join("boot.toml");
        if std::fs::create_dir_all(target.parent().expect("has a parent"))
            .and_then(|()| std::fs::write(&target, content))
            .is_ok()
        {
            synced += 1;
        }
    }
    if synced == 0 {
        eprintln!(
            "note: no mounted EFI System Partition could be written; the bootloader only reads \
             \\EFI\\leon\\boot.toml from the boot volume — mount an ESP and re-run `lbt config set`, \
             or copy ~/.config/leon/boot.toml there manually"
        );
    }
}

pub fn boot_config_path() -> PathBuf {
    // Fall back to the current directory rather than a literal "~" when HOME
    // is unset (a literal "~" would create a stray directory in the CWD).
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    home.join(".config").join("leon").join("boot.toml")
}

pub fn boot_config() -> Result<BootConfig> {
    let path = boot_config_path();
    if path.exists() {
        match std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))
            .and_then(|c| toml::from_str(&c).with_context(|| format!("parsing {}", path.display())))
        {
            Ok(cfg) => return Ok(cfg),
            Err(e) => {
                // A corrupt host copy must not block `config set` from fixing
                // it — same "bad config -> defaults" policy as the bootloader.
                eprintln!("warning: ignoring broken boot config ({e:#})");
            }
        }
    }
    // No usable host copy: fall back to the config on a mounted ESP, which is
    // what the bootloader actually reads.
    if let Ok(esps) = discovery::discover_esp_volumes() {
        for esp in &esps {
            if let Some(mount) = &esp.mountpoint {
                let target = mount.join("EFI").join("leon").join("boot.toml");
                if let Ok(content) = std::fs::read_to_string(&target) {
                    return toml::from_str(&content)
                        .with_context(|| format!("parsing {}", target.display()));
                }
            }
        }
    }
    Ok(BootConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let cfg = BootConfig {
            timeout: Some(5),
            theme: Some("splash.py".to_string()),
            entries_file: Some(r"\EFI\leon\entries.jsonc".to_string()),
            ..Default::default()
        };
        let s = toml::to_string(&cfg).unwrap();
        let back: BootConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.timeout, Some(5));
        assert_eq!(back.theme.as_deref(), Some("splash.py"));
        assert_eq!(back.field("timeout").as_deref(), Some("5"));
        assert_eq!(
            back.entries_file.as_deref(),
            Some(r"\EFI\leon\entries.jsonc")
        );
    }

    #[test]
    fn serialized_config_is_parseable_by_the_bootloader() {
        // lbt writes `boot.toml`; the bootloader reads it with the shared
        // `leon_common::boot_config` parser. The two must always agree, so
        // every key lbt writes must survive that parser.
        let cfg = BootConfig {
            timeout: Some(5),
            default_entry: Some("Cudane Linux".to_string()),
            theme: Some("splash.py".to_string()),
            splash: Some(true),
            entries_file: Some(r"\EFI\leon\entries.jsonc".to_string()),
        };
        let s = toml::to_string(&cfg).unwrap();
        let parsed = leon_common::boot_config::parse_boot_config(&s).unwrap();
        assert_eq!(parsed.timeout, Some(5));
        assert_eq!(parsed.default_entry.as_deref(), Some("Cudane Linux"));
        assert_eq!(parsed.theme.as_deref(), Some("splash.py"));
        assert_eq!(parsed.splash, Some(true));
        assert_eq!(
            parsed.entries_file.as_deref(),
            Some(r"\EFI\leon\entries.jsonc")
        );

        // And the parser must reject a string splash, so the bootloader never
        // silently accepts the old string form.
        assert!(leon_common::boot_config::parse_boot_config("splash = \"true\"").is_err());
    }
}
