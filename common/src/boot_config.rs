//! Minimal parser for `\EFI\leon\boot.toml`, the boot configuration written
//! by `lbc config set` (host side) and validated by the bootloader every boot.
//!
//! Only the keys `lbt` manages are understood — `timeout` (integer seconds),
//! `default_entry`, `theme`, `splash`, `entries_file` — and unknown keys are
//! ignored so the bootloader tolerates forward-compatible files. String values
//! accept both TOML basic strings (`"..."`, with escapes) and TOML literal
//! strings (`'...'`, no escapes), matching what serde/toml emits. Everything
//! here is pure and `no_std`, so it is unit-tested on the host.

extern crate alloc;

use alloc::string::String;

/// Boot configuration as parsed from the boot volume.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BootConfig {
    /// Seconds to wait before booting the default entry.
    pub timeout: Option<u32>,
    /// Label of the entry booted when the timeout elapses.
    pub default_entry: Option<String>,
    /// Splash theme file (host-side authoring concern).
    pub theme: Option<String>,
    /// Whether the splash menu is enabled.
    pub splash: Option<bool>,
    /// Where the bootloader writes the auto-detected boot entries (JSONC).
    pub entries_file: Option<String>,
}

/// A parse failure in `boot.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootConfigError(pub &'static str);

impl core::fmt::Display for BootConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

impl core::error::Error for BootConfigError {}

/// Parses a `boot.toml` document. A missing file is not an error: callers
/// treat a failed `read` as an all-default config.
pub fn parse_boot_config(content: &str) -> Result<BootConfig, BootConfigError> {
    let mut cfg = BootConfig::default();
    for raw in content.lines() {
        let line = raw.split('#').next().unwrap_or(raw).trim();
        if line.is_empty() {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or(BootConfigError("expected 'key = value'"))?;
        match k.trim() {
            "timeout" => {
                cfg.timeout = Some(
                    v.trim()
                        .parse()
                        .map_err(|_| BootConfigError("timeout must be a number of seconds"))?,
                )
            }
            "default_entry" => cfg.default_entry = Some(parse_string(v)?),
            "theme" => cfg.theme = Some(parse_string(v)?),
            "splash" => cfg.splash = Some(parse_bool(v)?),
            "entries_file" => cfg.entries_file = Some(parse_string(v)?),
            _ => {} // tolerate unknown keys
        }
    }
    Ok(cfg)
}

fn parse_string(v: &str) -> Result<String, BootConfigError> {
    let v = v.trim();
    // TOML basic string: double-quoted, with the escapes serde/toml emits.
    if let Some(inner) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                _ => return Err(BootConfigError("invalid escape in string value")),
            }
        }
        return Ok(out);
    }
    // TOML literal string: single-quoted, no escapes (what `toml::to_string`
    // emits for backslash-heavy values like `\EFI\leon\entries.jsonc`).
    if let Some(inner) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Ok(String::from(inner));
    }
    Err(BootConfigError(
        "string values must be double- or single-quoted",
    ))
}

fn parse_bool(v: &str) -> Result<bool, BootConfigError> {
    match v.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(BootConfigError("splash must be true or false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_keys() {
        let cfg = parse_boot_config(
            "timeout = 5\ndefault_entry = \"Cudane Linux\"\ntheme = \"splash.py\"\nsplash = true\nentries_file = \"\\\\EFI\\\\leon\\\\entries.jsonc\"\n",
        )
        .unwrap();
        assert_eq!(cfg.timeout, Some(5));
        assert_eq!(cfg.default_entry.as_deref(), Some("Cudane Linux"));
        assert_eq!(cfg.theme.as_deref(), Some("splash.py"));
        assert_eq!(cfg.splash, Some(true));
        assert_eq!(
            cfg.entries_file.as_deref(),
            Some(r"\EFI\leon\entries.jsonc")
        );
    }

    #[test]
    fn missing_keys_default() {
        let cfg = parse_boot_config("").unwrap();
        assert_eq!(cfg, BootConfig::default());
    }

    #[test]
    fn ignores_comments_and_unknown_keys() {
        let cfg =
            parse_boot_config("# comment\ntimeout = 3   # inline\nfuture_key = \"x\"\n").unwrap();
        assert_eq!(cfg.timeout, Some(3));
        assert_eq!(cfg.default_entry, None);
        assert_eq!(cfg.splash, None);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(parse_boot_config("timeout = soon").is_err());
        assert!(parse_boot_config("splash = yes").is_err());
        assert!(parse_boot_config("theme = splash.py").is_err());
        assert!(parse_boot_config("default_entry = \"unclosed").is_err());
        assert!(parse_boot_config("garbage").is_err());
    }

    #[test]
    fn unescapes_strings() {
        let cfg = parse_boot_config("default_entry = \"a\\\"b\"\ntheme = \"a\\\\b\"\n").unwrap();
        assert_eq!(cfg.default_entry.as_deref(), Some("a\"b"));
        assert_eq!(cfg.theme.as_deref(), Some("a\\b"));
    }

    #[test]
    fn parses_literal_strings() {
        // `toml::to_string` emits literal strings for backslash-heavy values.
        let cfg = parse_boot_config("entries_file = '\\EFI\\leon\\entries.jsonc'\n").unwrap();
        assert_eq!(
            cfg.entries_file.as_deref(),
            Some(r"\EFI\leon\entries.jsonc")
        );
    }
}
