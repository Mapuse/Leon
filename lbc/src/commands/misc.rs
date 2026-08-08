//! `lbc` informational helpers: version, keymap, help, JSON output, boot
//! entries, ESPs, geometry and status.

use anyhow::Result;

use crate::cli::tree::{Node, ROOT, help_for};
use crate::boot_config;
use lbt::discovery;
use lbt::geometry::Geometry;

/// The boot-manager keymap (mirrors `lbc tui` and the on-screen help).
pub fn keymap_show() -> Result<()> {
    println!("Leon boot-manager keymap");
    println!("  ↑ / ↓ / j / k   move selection");
    println!("  Enter            boot the selected entry");
    println!("  Esc              cancel / quit");
    println!("  r                refresh entries");
    println!("  ?                toggle help");
    println!("  s                system info");
    Ok(())
}

pub fn version() -> Result<()> {
    println!("lbc {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

fn leaf_nodes(node: &Node, out: &mut Vec<&'static Node>) {
    for child in node.children() {
        if child.handler().is_some() {
            out.push(child);
        }
        leaf_nodes(child, out);
    }
}

/// Print the full command reference: every leaf with its flags.
pub fn help_all() -> Result<()> {
    let mut leaves = Vec::new();
    leaf_nodes(&ROOT, &mut leaves);
    println!("lbc {} — Leon Boot Configuration", env!("CARGO_PKG_VERSION"));
    println!();
    let mut rows: Vec<(&str, Vec<String>)> = Vec::new();
    for leaf in leaves {
        let flags: Vec<String> = leaf
            .flags
            .iter()
            .map(|f| format!("-{}/--{}", f.short, f.long))
            .collect();
        rows.push((leaf.name, flags));
    }
    rows.sort();
    let width = rows.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    for (name, flags) in rows {
        if flags.is_empty() {
            println!("  {name}");
        } else {
            println!("  {name:<width$}  {}", flags.join(" "));
        }
    }
    Ok(())
}

/// Print the command tree (every canonical path).
pub fn help_tree() -> Result<()> {
    let mut out = Vec::new();
    let mut prefix = Vec::new();
    crate::cli::tree::walk(&ROOT, &mut prefix, &mut out);
    for line in out {
        println!("{line}");
    }
    Ok(())
}

/// Print help for one command (matched by canonical name or alias).
pub fn help_command(name: &str) -> Result<()> {
    let node = find_node(&ROOT, name)
        .ok_or_else(|| anyhow::anyhow!("no command matches `{name}`"))?;
    print!("{}", help_for(node));
    Ok(())
}

/// Depth-first search for a node by canonical name or alias.
fn find_node<'a>(node: &'a Node, name: &str) -> Option<&'a Node> {
    if node.matches(name) {
        return Some(node);
    }
    for child in node.children() {
        if let Some(found) = find_node(child, name) {
            return Some(found);
        }
    }
    None
}

/// The boot config as JSON.
pub fn json_config() -> Result<()> {
    let cfg = boot_config::boot_config()?;
    println!("{}", serde_json::to_string_pretty(&cfg)?);
    Ok(())
}

/// Geometry as JSON (same fields the bootloader's `bootinfo.json` carries).
pub fn json_info() -> Result<()> {
    let g = Geometry::load(None)?;
    println!(
        "{}",
        serde_json::json!({
            "width": g.width,
            "height": g.height,
            "stride": g.stride,
            "format": g.format,
            "logo": g.logo_text(),
        })
    );
    Ok(())
}

/// Discovery result as JSON.
pub fn json_discover() -> Result<()> {
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    println!(
        "{}",
        serde_json::json!({
            "esps": esps.iter().map(|e| serde_json::json!({
                "path": e.path,
                "mountpoint": e.mountpoint.as_ref().map(|m| m.display().to_string()),
                "label": e.label,
                "uuid": e.uuid,
            })).collect::<Vec<_>>(),
            "boot_entries": entries.iter().map(|e| serde_json::json!({
                "label": e.label,
                "path": e.path,
            })).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

/// Everything (config + geometry + discovery) as one JSON document.
pub fn json_all() -> Result<()> {
    let g = Geometry::load(None)?;
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    let cfg = boot_config::boot_config()?;
    println!(
        "{}",
        serde_json::json!({
            "lbc": env!("CARGO_PKG_VERSION"),
            "boot_config": serde_json::to_value(&cfg)?,
            "framebuffer": {
                "width": g.width,
                "height": g.height,
                "stride": g.stride,
                "format": g.format,
            },
            "logo": g.logo_text(),
            "esps": esps.iter().map(|e| serde_json::json!({
                "path": e.path,
                "mountpoint": e.mountpoint.as_ref().map(|m| m.display().to_string()),
                "label": e.label,
                "uuid": e.uuid,
            })).collect::<Vec<_>>(),
            "boot_entries": entries.iter().map(|e| serde_json::json!({
                "label": e.label,
                "path": e.path,
            })).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

/// List every discovered boot entry (label + path).
pub fn entries_list() -> Result<()> {
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    if entries.is_empty() {
        println!("no boot entries discovered");
        return Ok(());
    }
    for e in &entries {
        println!("{}  {}", e.label, e.path);
    }
    Ok(())
}

/// Resolve one boot entry by label, printing its on-ESP path.
pub fn entries_get(name: &str) -> Result<()> {
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    match entries.iter().find(|e| e.label == name) {
        Some(e) => println!("{} -> {}", e.label, e.path),
        None => anyhow::bail!("no boot entry labelled `{name}`"),
    }
    Ok(())
}

/// Count discovered boot entries.
pub fn entries_count() -> Result<()> {
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    println!("{}", entries.len());
    Ok(())
}

/// List every discovered boot entry label.
pub fn names_list() -> Result<()> {
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    if entries.is_empty() {
        println!("no boot entries discovered");
        return Ok(());
    }
    for e in &entries {
        println!("{}", e.label);
    }
    Ok(())
}

/// Resolve one boot entry by label (like [`entries_get`]).
pub fn names_get(name: &str) -> Result<()> {
    entries_get(name)
}

/// Count discovered boot entries (like [`entries_count`]).
pub fn names_count() -> Result<()> {
    entries_count()
}

/// List every ESP volume.
pub fn esps() -> Result<()> {
    let esps = discovery::discover_esp_volumes()?;
    if esps.is_empty() {
        println!("no ESP volumes detected");
        return Ok(());
    }
    for e in &esps {
        println!("{}", e.describe());
    }
    Ok(())
}

/// Geometry (text form).
pub fn geometry_show() -> Result<()> {
    println!("{}", Geometry::load(None)?.report());
    Ok(())
}

/// BGRT logo geometry only.
pub fn geometry_bgr() -> Result<()> {
    println!("{}", Geometry::load(None)?.logo_text());
    Ok(())
}

/// Query front-end: geometry (like `lbc i`).
pub fn query_info() -> Result<()> {
    geometry_show()
}

/// Query front-end: every ESP volume.
pub fn query_esps() -> Result<()> {
    esps()
}

/// Boot configuration status: path, saved profiles, and the keys the
/// bootloader will read.
pub fn status() -> Result<()> {
    println!("Leon boot configuration status");
    println!("  config: {}", boot_config::boot_config_path().display());
    let cfg = boot_config::boot_config()?;
    for key in boot_config::CONFIG_KEYS {
        println!(
            "  {} = {}",
            key,
            cfg.field(key).unwrap_or_else(|| "(unset)".to_string())
        );
    }
    let dir = boot_config::boot_config_path()
        .parent()
        .expect("boot config path has a parent")
        .join("profiles");
    let profiles = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().ends_with(".toml"))
                .count()
        })
        .unwrap_or(0);
    println!("  profiles: {profiles} saved");
    let esps = discovery::discover_esp_volumes().unwrap_or_default();
    println!("  ESPs: {}", esps.len());
    Ok(())
}
