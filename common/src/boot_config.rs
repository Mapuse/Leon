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

use alloc::string::{String, ToString};

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

/// Serializes a [`BootConfig`] back to TOML, omitting unset keys — the exact
/// inverse of [`parse_boot_config`], and in the same shape the host tools
/// (`lbc`/`lbm`) write. Used by the bootloader when the on-device menuconfig
/// edits the config and saves it to `\EFI\leon\boot.toml`.
pub fn serialize_boot_config(cfg: &BootConfig) -> String {
    let mut out = String::new();
    if let Some(t) = cfg.timeout {
        out.push_str("timeout = ");
        out.push_str(&t.to_string());
        out.push('\n');
    }
    if let Some(s) = cfg.splash {
        out.push_str("splash = ");
        out.push_str(if s { "true" } else { "false" });
        out.push('\n');
    }
    if let Some(v) = &cfg.theme {
        out.push_str("theme = ");
        push_string(&mut out, v);
        out.push('\n');
    }
    if let Some(v) = &cfg.default_entry {
        out.push_str("default_entry = ");
        push_string(&mut out, v);
        out.push('\n');
    }
    if let Some(v) = &cfg.entries_file {
        out.push_str("entries_file = ");
        push_string(&mut out, v);
        out.push('\n');
    }
    out
}

/// Appends a TOML string value. Values without a single quote use a TOML
/// literal string (`'...'`, no escapes) — what the host tools emit for
/// backslash-heavy paths like `\EFI\leon\entries.jsonc`. Values containing a
/// quote fall back to a double-quoted basic string with the escapes the
/// parser above knows. Both forms are re-parseable by [`parse_boot_config`].
fn push_string(out: &mut String, v: &str) {
    if !v.contains('\'') {
        out.push('\'');
        out.push_str(v);
        out.push('\'');
    } else {
        out.push('"');
        for c in v.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push('"');
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

    #[test]
    fn serialization_roundtrips_through_parse() {
        let cfg = BootConfig {
            timeout: Some(5),
            default_entry: Some("Cudane Linux".to_string()),
            theme: Some("splash.py".to_string()),
            splash: Some(true),
            entries_file: Some(r"\EFI\leon\entries.jsonc".to_string()),
        };
        let s = serialize_boot_config(&cfg);
        assert_eq!(parse_boot_config(&s).unwrap(), cfg);
    }

    #[test]
    fn serialization_omits_unset_keys() {
        // Same contract as `lbc config set` / `lbm`: unset keys are omitted.
        assert!(serialize_boot_config(&BootConfig::default()).trim().is_empty());
    }

    #[test]
    fn serialization_uses_literal_strings_for_backslash_paths() {
        let cfg = BootConfig {
            entries_file: Some(r"\EFI\leon\entries.jsonc".to_string()),
            ..BootConfig::default()
        };
        let s = serialize_boot_config(&cfg);
        assert!(s.contains(r"entries_file = '\EFI\leon\entries.jsonc'"));
        assert_eq!(parse_boot_config(&s).unwrap(), cfg);
    }

    #[test]
    fn serialization_escapes_embedded_quotes() {
        let cfg = BootConfig {
            default_entry: Some("It's Leon".to_string()),
            theme: Some(r"a\b".to_string()),
            ..BootConfig::default()
        };
        let s = serialize_boot_config(&cfg);
        assert_eq!(parse_boot_config(&s).unwrap(), cfg);
    }
}
