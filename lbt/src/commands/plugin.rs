//! `lbt plugin`: list and run plugins (Python-gated).

use anyhow::{Result, anyhow, bail};

use crate::cli::PluginCommand;
use crate::python::python_init;

pub fn run(cmd: PluginCommand) -> Result<()> {
    python_init();
    match cmd {
        PluginCommand::List => {
            let plugins = cps::plugin::PluginManager::list();
            if plugins.is_empty() {
                println!("No plugins registered.");
            } else {
                for p in &plugins {
                    let alias_str = if p.aliases.is_empty() {
                        String::new()
                    } else {
                        let names: Vec<&str> = p.aliases.keys().map(String::as_str).collect();
                        format!(" [aliases: {}]", names.join(", "))
                    };
                    println!("  {} ({}){}", p.name, p.path, alias_str);
                }
            }
        }
        PluginCommand::Run { name, func, args } => {
            let (entry, func) = match cps::plugin::PluginManager::by_alias(&name) {
                Some(found) => found,
                None => {
                    let entry = cps::plugin::PluginManager::by_name(&name)
                        .ok_or_else(|| anyhow!("plugin '{name}' not found"))?;
                    (entry, func)
                }
            };
            match cps::plugin::PluginManager::run(&entry, &func, &args) {
                Ok(out) => print!("{out}"),
                Err(e) => bail!("plugin '{}' failed: {e}", entry.name),
            }
        }
    }
    Ok(())
}
