//! `lbt tui`: list, run, and persist Python TUIs (Python-gated).
//!
//! A bare `lbt tui` (or `lbt tui run`) runs the configured `[python] tui`; when
//! none is configured the shipped Leon boot-manager menu (`leon_menu.py`, which
//! is embedded in the binary and materialized to `~/.config/leon/tuis/` on first
//! use) runs instead. Live geometry, BGRT logo placement, boot entries and boot
//! config are exported to the `LEON_*` environment variables the TUI reads.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::cli::TuiCommand;
use crate::discovery;
use crate::geometry::Geometry;
use crate::python::{config_dir, python_config, python_init, write_python_config};

/// The shipped Leon boot-manager menu, embedded so `lbt tui` works from any
/// directory (and after install) without shipping a side file.
const DEFAULT_TUI_SOURCE: &str = include_str!("../../tuis/leon_menu.py");

pub fn run(cmd: Option<TuiCommand>) -> Result<()> {
    python_init();
    match cmd {
        None => run_default(),
        Some(TuiCommand::List) => {
            let tuis = cps::tui::TuiEngine::list();
            if tuis.is_empty() {
                println!("No Python TUIs registered.");
            } else {
                for t in &tuis {
                    let desc = if t.description.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", t.description)
                    };
                    println!("  {} ({}){}", t.name, t.path, desc);
                }
            }
            Ok(())
        }
        Some(TuiCommand::Run { name }) => run_named(name.as_deref()),
        Some(TuiCommand::Apply { name }) => apply(&name),
    }
}

/// `lbt tui` with no subcommand: the configured default TUI, or the shipped
/// Leon menu when none is configured (or the configured one is missing).
fn run_default() -> Result<()> {
    let cfg = python_config();
    let path = if !cfg.tui.is_empty() && Path::new(&cfg.tui).exists() {
        cfg.tui
    } else {
        default_tui_path()?
    };
    inject_context();
    run_path(&path)
}

/// `lbt tui run [name]`: run a registered TUI by name, or the default when no
/// name is given.
fn run_named(name: Option<&str>) -> Result<()> {
    let path = match name {
        Some(name) => {
            let entry = cps::tui::TuiEngine::by_name(name)
                .ok_or_else(|| anyhow!("tui '{name}' not found"))?;
            resolve_tui_path(&entry)?
        }
        None => {
            let cfg = python_config();
            if !cfg.tui.is_empty() && Path::new(&cfg.tui).exists() {
                cfg.tui
            } else {
                default_tui_path()?
            }
        }
    };
    inject_context();
    run_path(&path)
}

/// Runs a TUI's `run()` in this process through the cps engine.
fn run_path(path: &str) -> Result<()> {
    let mut python = python_config();
    python.enabled = true;
    python.tui = path.to_string();
    let engine = cps::PythonEngine::new(&python);
    let tui = engine
        .tui
        .ok_or_else(|| anyhow!("tui '{path}' failed to load"))?;
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    if !tui.has_run() {
        bail!("tui '{stem}' does not define run()");
    }
    if !tui.run() {
        bail!("tui '{stem}' run() exited with an error");
    }
    Ok(())
}

/// `lbt tui apply <name>`: persist a TUI as the default for `lbt tui`. The
/// path is written absolute (config-relative and `~` forms are normalized),
/// and `tui_mode` is pinned to `false`: applying a TUI makes it the *default
/// when invoked*, it never flips the host into auto-launching TUI mode.
fn apply(name: &str) -> Result<()> {
    let entry = cps::tui::TuiEngine::by_name(name)
        .ok_or_else(|| anyhow!("tui '{name}' not found"))?;
    let path = resolve_tui_path(&entry)?;
    let mut python = python_config();
    python.enabled = true;
    python.tui = path.clone();
    python.tui_mode = false;
    write_python_config(&python)?;
    println!("tui = {path}");
    Ok(())
}

/// Resolves a registered TUI to an absolute, existing file. The shipped Leon
/// menu is materialized on first use.
fn resolve_tui_path(entry: &cps::tui::TuiEntry) -> Result<String> {
    let def = default_tui_path()?;
    let path = absolutize_cwd(&entry.path);
    if Path::new(&path) == Path::new(&def) {
        return Ok(def);
    }
    if !Path::new(&path).exists() {
        bail!("tui file not found: {path}");
    }
    Ok(path)
}

/// Expands `~` and makes a still-relative path absolute against the working
/// directory (a registered TUI is named from the shell, so CWD is the base).
fn absolutize_cwd(path: &str) -> String {
    let expanded = cps::expand_tilde(path);
    let p = PathBuf::from(&expanded);
    if p.is_absolute() {
        expanded
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&p)
            .to_string_lossy()
            .to_string()
    }
}

/// Path of the shipped Leon boot-manager menu. Writes it to
/// `~/.config/leon/tuis/leon_menu.py` (creating the directory) when missing,
/// so the embedded source becomes an editable, CWD-independent file.
fn default_tui_path() -> Result<String> {
    let path = config_dir().join("tuis").join("leon_menu.py");
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, DEFAULT_TUI_SOURCE)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(path.to_string_lossy().to_string())
}

/// Exports live framebuffer + BGRT geometry, discovered boot entries and boot
/// config to the `LEON_*` environment variables `leon_menu.py` reads. Every
/// step is best-effort: geometry may be missing (no framebuffer), there may be
/// no ESPs, and the boot config may be unreadable — the TUI shows a friendly
/// empty/placeholder state either way.
fn inject_context() {
    let mut ctx: HashMap<String, String> = match Geometry::load(None) {
        Ok(g) => g.context(),
        Err(e) => {
            eprintln!("note: {e:#}");
            HashMap::new()
        }
    };

    if let Ok(esps) = discovery::discover_esp_volumes()
        && let Ok(entries) = discovery::discover_boot_entries(&esps)
    {
        let lines: Vec<String> = entries
            .iter()
            .map(|e| format!("{}\t{}", e.label, e.path))
            .collect();
        ctx.insert("boot_entries".to_string(), lines.join("\n"));
    }

    if let Ok(cfg) = crate::boot_config::boot_config() {
        for (key, env) in [
            ("timeout", "LEON_BOOT_TIMEOUT"),
            ("default_entry", "LEON_BOOT_DEFAULT"),
            ("splash", "LEON_BOOT_SPLASH"),
        ] {
            if let Some(value) = cfg.field(key) {
                ctx.insert(env.to_string(), value);
            }
        }
    }

    let mapping = [
        ("leon_fb_width", "LEON_FB_WIDTH"),
        ("leon_fb_height", "LEON_FB_HEIGHT"),
        ("leon_fb_stride", "LEON_FB_STRIDE"),
        ("leon_fb_format", "LEON_FB_FORMAT"),
        ("leon_bgrt_rect", "LEON_BGRT_RECT"),
        ("leon_logo_center_x", "LEON_LOGO_CENTER_X"),
        ("leon_logo_center_y", "LEON_LOGO_CENTER_Y"),
        ("leon_bgrt_status", "LEON_BGRT_STATUS"),
        ("leon_bgrt_type", "LEON_BGRT_TYPE"),
        ("boot_entries", "LEON_BOOT_ENTRIES"),
        ("LEON_BOOT_TIMEOUT", "LEON_BOOT_TIMEOUT"),
        ("LEON_BOOT_DEFAULT", "LEON_BOOT_DEFAULT"),
        ("LEON_BOOT_SPLASH", "LEON_BOOT_SPLASH"),
    ];
    for (key, env) in mapping {
        if let Some(value) = ctx.get(key) {
            // Safety: this runs once, single-threaded, before the Python
            // interpreter boots; the values are owned and never re-mutated,
            // so no other thread can observe a partially-updated env block.
            unsafe { env::set_var(env, value) };
        }
    }
}
