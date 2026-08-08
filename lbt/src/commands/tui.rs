//! `lbt tui`: the keyboard-driven Leon boot-manager menu (ratatui).
//!
//! A pure-Rust, full-screen preview of what Leon renders at boot: the
//! discovered boot entries, the live framebuffer + BGRT geometry, and the boot
//! config. Geometry and entries are read directly from the host (the same
//! `Geometry`/`discovery` modules the other `lbt` commands use); `LEON_BOOT_ENTRIES`
//! (one `label<TAB>path` per line) overrides live discovery so tests and
//! embedded runs stay deterministic.

use anyhow::Result;

use crate::boot_config;
use crate::discovery;
use crate::geometry::Geometry;

/// One boot entry shown in the menu.
#[derive(Debug, Clone)]
pub struct Entry {
    pub label: String,
    pub path: String,
    pub source: String,
}

/// The live state the menu renders: geometry, entries, and boot config.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub entries: Vec<Entry>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: String,
    pub timeout: String,
    pub default: String,
    pub splash: String,
}

impl Context {
    /// Reads geometry, entries, and boot config. Every source is best-effort:
    /// geometry may be missing (no framebuffer), there may be no ESPs, and the
    /// boot config may be unreadable — the menu shows a friendly placeholder
    /// state either way.
    pub fn load() -> Self {
        let mut ctx = Self::default();

        // Geometry: LEON_FB_* wins, then the live sysfs / BootInfo dump.
        if let Some(width) = env_u32("LEON_FB_WIDTH") {
            ctx.width = width;
            ctx.height = env_u32("LEON_FB_HEIGHT").unwrap_or(0);
            ctx.stride = env_u32("LEON_FB_STRIDE").unwrap_or(width);
            ctx.format = std::env::var("LEON_FB_FORMAT").unwrap_or_default();
        } else if let Ok(g) = Geometry::load(None) {
            ctx.width = g.width;
            ctx.height = g.height;
            ctx.stride = g.stride;
            ctx.format = g.format;
        }

        // Entries: LEON_BOOT_ENTRIES wins, else live discovery.
        let src = std::env::var("LEON_BOOT_ENTRIES").unwrap_or_default();
        if src.trim().is_empty() {
            if let Ok(esps) = discovery::discover_esp_volumes()
                && let Ok(entries) = discovery::discover_boot_entries(&esps)
            {
                ctx.entries = entries
                    .into_iter()
                    .map(|e| Entry {
                        label: e.label,
                        path: e.path,
                        source: "discovery".to_string(),
                    })
                    .collect();
            }
        } else {
            ctx.entries = src
                .lines()
                .filter_map(|line| {
                    let mut it = line.splitn(2, '\t');
                    let label = it.next()?.trim();
                    if label.is_empty() {
                        return None;
                    }
                    let path = it.next().unwrap_or("").trim();
                    Some(Entry {
                        label: label.to_string(),
                        path: path.to_string(),
                        source: "backend".to_string(),
                    })
                })
                .collect();
        }

        if let Ok(cfg) = boot_config::boot_config() {
            ctx.timeout = cfg.field("timeout").unwrap_or_default();
            ctx.default = cfg.field("default_entry").unwrap_or_default();
            ctx.splash = cfg.field("splash").unwrap_or_default();
        }

        ctx
    }
}

fn env_u32(key: &str) -> Option<u32> {
    std::env::var(key).ok().and_then(|s| s.trim().parse().ok())
}

/// Runs the boot-manager menu until the user quits.
pub fn run() -> Result<()> {
    let context = Context::load();
    super::tui_app::App::new(context).run()
}
