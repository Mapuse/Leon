//! `lbt discover`: auto-detect every ESP and its boot entries.

use anyhow::Result;

use crate::discovery;

pub fn run(json: bool) -> Result<()> {
    let devs = discovery::discover_block_devices()?;
    let esps = discovery::discover_esp_volumes()?;
    let entries = discovery::discover_boot_entries(&esps)?;
    if json {
        let devs: Vec<serde_json::Value> = devs
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "path": d.path,
                    "type": d.kind,
                    "fstype": d.fstype,
                    "parttype": d.parttype,
                    "label": d.label,
                    "uuid": d.uuid,
                    "mountpoint": d.mountpoint,
                    "esp": d.is_esp(),
                })
            })
            .collect();
        let esps: Vec<serde_json::Value> = esps
            .iter()
            .map(|e| {
                serde_json::json!({
                    "path": e.path,
                    "mountpoint": e.mountpoint.as_ref().map(|m| m.display().to_string()),
                    "label": e.label,
                    "uuid": e.uuid,
                })
            })
            .collect();
        let entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| serde_json::json!({ "label": e.label, "path": e.path }))
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "devices": devs,
                "esps": esps,
                "boot_entries": entries
            })
        );
    } else {
        println!("Block devices:");
        if devs.is_empty() {
            println!("  none");
        } else {
            for d in &devs {
                let esp = if d.is_esp() { " [ESP]" } else { "" };
                println!(
                    "  {} ({}) fstype={} mount={}{}",
                    d.name,
                    d.kind,
                    d.fstype.as_deref().unwrap_or("none"),
                    d.mountpoint.as_deref().unwrap_or("none"),
                    esp
                );
            }
        }
        println!("EFI System Partitions:");
        if esps.is_empty() {
            println!("  none");
        } else {
            for e in &esps {
                println!("  {}", e.describe());
            }
        }
        println!("Boot entries:");
        if entries.is_empty() {
            println!("  none");
        } else {
            for e in &entries {
                println!("  {} -> {}", e.label, e.path);
            }
        }
    }
    Ok(())
}
