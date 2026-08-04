//! Host-side Python config (`[python]` section) for the cps runtime.
//!
//! Only compiled under the `python` feature.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// One-time cps configuration, called only from the Python-backed subcommands.
/// Keeps pyo3 uninitialized for everything else (`info`, `discover`, `config`).
pub fn python_init() {
    cps::configure(cps::Options::new("leon"));
}

/// The Leon user config directory (`$HOME/.config/leon`), falling back to the
/// current directory when `HOME` is unset.
pub fn config_dir() -> PathBuf {
    python_config_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Path of the host-side Python config (`[python]` section) for this component.
///
/// This is *not* the boot config: `boot.toml` stays the keys the bootloader
/// reads. The `[python]` section follows the cps convention shared by every
/// Cudane component and carries the interpreter settings that only matter on
/// the host (`enabled`, `tui`, `theme`, `venv_path`).
pub fn python_config_path() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    home.join(".config").join("leon").join("python.toml")
}

/// Serde mirror of the `[python]` table in `python.toml`.
#[derive(serde::Deserialize)]
struct PythonSection {
    #[serde(default)]
    python: cps::PythonConfig,
}

/// Reads the host Python config, defaulting everything on any failure. A
/// missing or broken file must never stop a TUI/theme from running — same
/// "bad config -> defaults" policy as the bootloader.
pub fn python_config() -> cps::PythonConfig {
    let path = python_config_path();
    let mut cfg = if path.exists() {
        match std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))
            .and_then(|c| {
                let section: PythonSection =
                    toml::from_str(&c).with_context(|| format!("parsing {}", path.display()))?;
                Ok(section.python)
            }) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("warning: ignoring broken python config ({e:#})");
                cps::PythonConfig::default()
            }
        }
    } else {
        cps::PythonConfig::default()
    };
    // `[python]` paths are relative to the config file, never to the working
    // directory: a config in `~/.config/leon` must behave the same from
    // anywhere. This also keeps the cps `sys.path` insert from ever receiving
    // an empty string when a TUI sits directly under the CWD.
    let base = config_dir();
    cfg.tui = absolutize(&cfg.tui, &base);
    cfg.venv_path = absolutize(&cfg.venv_path, &base);
    cfg
}

/// Expands a leading `~` and, when still relative, resolves `path` against
/// `base`. An empty path is left empty (it means "not configured"). `.` path
/// segments are dropped so `./venv` resolves cleanly to `<base>/venv`.
pub fn absolutize(path: &str, base: &Path) -> String {
    let expanded = cps::expand_tilde(path);
    if path.is_empty() {
        return expanded;
    }
    let p = PathBuf::from(&expanded);
    let clean = |pb: PathBuf| {
        let mut out = PathBuf::new();
        for c in pb.components() {
            match c {
                std::path::Component::CurDir => {}
                _ => out.push(c.as_os_str()),
            }
        }
        out.to_string_lossy().to_string()
    };
    if p.is_absolute() {
        clean(p)
    } else {
        clean(base.join(p))
    }
}

/// Writes the host Python config, wrapping it in a `[python]` table so it
/// round-trips through [`python_config`].
pub fn write_python_config(cfg: &cps::PythonConfig) -> Result<()> {
    let path = python_config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let body = toml::to_string(cfg).context("serializing python config")?;
    let content = format!("[python]\n{body}");
    std::fs::write(&path, &content).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolutize_resolves_relative_against_base() {
        let base = Path::new("/home/user/.config/leon");
        assert_eq!(
            absolutize("./venv_tui.py", base),
            "/home/user/.config/leon/venv_tui.py"
        );
        assert_eq!(
            absolutize("venv", base),
            "/home/user/.config/leon/venv"
        );
        // Absolute paths pass through untouched; `~` expands against $HOME.
        assert_eq!(absolutize("/x/y.py", base), "/x/y.py");
        assert_eq!(absolutize("", base), "");
    }

    #[test]
    fn absolutize_drops_dot_segments() {
        let base = Path::new("/home/user/.config/leon");
        assert_eq!(
            absolutize("./venv", base),
            "/home/user/.config/leon/venv"
        );
        assert_eq!(
            absolutize("././tuis/./leon_menu.py", base),
            "/home/user/.config/leon/tuis/leon_menu.py"
        );
        assert_eq!(absolutize("/a/./b", base), "/a/b");
    }

    #[test]
    fn python_config_roundtrip() {
        // `write_python_config` produces a `[python]` table that
        // `python_config` must read back exactly.
        let dir = std::env::temp_dir().join("lbt_python_cfg_test");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = cps::PythonConfig {
            enabled: true,
            tui: "~/.config/leon/tuis/setup.py".to_string(),
            venv_path: "~/.config/leon/venv".to_string(),
            ..Default::default()
        };
        let body = toml::to_string(&cfg).unwrap();
        let content = format!("[python]\n{body}");
        std::fs::write(dir.join("python.toml"), &content).unwrap();
        let back: PythonSection = toml::from_str(&content).unwrap();
        assert_eq!(back.python, cfg);
        assert_eq!(back.python.tui, "~/.config/leon/tuis/setup.py");
        assert_eq!(back.python.venv_path, "~/.config/leon/venv");
        assert!(back.python.enabled);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
