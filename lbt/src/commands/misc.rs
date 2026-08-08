//! `lbt` informational helpers: version, help, JSON output, boot-entry names,
//! geometry and query front-ends.

use anyhow::Result;

use crate::cli::tree::{Node, ROOT, help_for};
use crate::discovery;
use crate::geometry::Geometry;

pub fn version() -> Result<()> {
    println!("lbt {}", env!("CARGO_PKG_VERSION"));
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
    println!("lbt {} — Leon Build Tool", env!("CARGO_PKG_VERSION"));
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

/// Print the command tree as an indented outline.
pub fn help_tree() -> Result<()> {
    let mut out = Vec::new();
    crate::cli::tree::walk(&ROOT, &mut Vec::new(), &mut out);
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

/// Everything (geometry + discovery) as one JSON document.
pub fn json_all() -> Result<()> {
    let g = Geometry::load(None)?;
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    println!(
        "{}",
        serde_json::json!({
            "lbt": env!("CARGO_PKG_VERSION"),
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

/// Resolve one boot entry by label, printing its on-ESP path.
pub fn names_get(name: &str) -> Result<()> {
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    match entries.iter().find(|e| e.label == name) {
        Some(e) => println!("{} -> {}", e.label, e.path),
        None => anyhow::bail!("no boot entry labelled `{name}`"),
    }
    Ok(())
}

/// Count discovered boot entries.
pub fn names_count() -> Result<()> {
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    println!("{}", entries.len());
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

/// Query front-end: geometry (like `lbt i`).
pub fn query_info() -> Result<()> {
    geometry_show()
}

/// Query front-end: every ESP volume (like `lbt d`'s ESP section).
pub fn query_esps() -> Result<()> {
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
