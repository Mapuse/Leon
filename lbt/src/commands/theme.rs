//! `lbt theme`: list, preview, and run boot themes (Python-gated).

use anyhow::{Result, anyhow, bail};

use crate::cli::ThemeCommand;
use crate::discovery;
use crate::geometry::Geometry;
use crate::python::python_init;

pub fn run(cmd: ThemeCommand) -> Result<()> {
    python_init();
    match cmd {
        ThemeCommand::List => {
            let themes = cps::theme::ThemeEngine::list();
            if themes.is_empty() {
                println!("No boot themes registered.");
            } else {
                for t in &themes {
                    let desc = if t.description.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", t.description)
                    };
                    println!("  {} ({}){}", t.name, t.path, desc);
                }
            }
        }
        ThemeCommand::Preview { name, dump } => {
            let entry = cps::theme::ThemeEngine::by_name(&name)
                .ok_or_else(|| anyhow!("theme '{name}' not found"))?;
            let g = Geometry::load(dump.as_deref())?;
            let engine = load_theme(&entry.path)?;
            preview(&engine, &g);
        }
        ThemeCommand::Run { name, dump } => {
            let entry = cps::theme::ThemeEngine::by_name(&name)
                .ok_or_else(|| anyhow!("theme '{name}' not found"))?;
            let _g = Geometry::load(dump.as_deref())?;
            let engine = load_theme(&entry.path)?;
            if !engine.has_run() {
                bail!("theme '{}' does not define run()", name);
            }
            if !engine.run() {
                bail!("theme '{}' run() failed", name);
            }
        }
    }
    Ok(())
}

/// Loads a theme by file path with a cps Python engine.
fn load_theme(path: &str) -> Result<cps::ThemeEngine> {
    let cfg = cps::PythonConfig {
        enabled: true,
        theme: path.to_string(),
        ..Default::default()
    };
    let engine = cps::PythonEngine::new(&cfg);
    engine
        .theme
        .ok_or_else(|| anyhow!("theme '{path}' failed to load"))
}

/// Renders a theme's prompt against the given geometry and prints it.
fn preview(engine: &cps::ThemeEngine, g: &Geometry) {
    let mut ctx = g.context();

    // Real boot entries discovered from every mounted ESP; never invented.
    if let Ok(esps) = discovery::discover_esp_volumes()
        && let Ok(entries) = discovery::discover_boot_entries(&esps)
        && let Some(first) = entries.first()
    {
        ctx.insert("boot_entry".to_string(), first.label.clone());
        let labels: Vec<String> = entries.iter().map(|e| e.label.clone()).collect();
        ctx.insert("boot_entries".to_string(), labels.join(","));
    }

    let r = engine.render_prompt(&ctx);
    for line in &r.lines_above {
        println!("{line}");
    }
    if r.right_prompt.is_empty() {
        println!("{}", r.input_prefix);
    } else {
        let pad = g
            .width
            .saturating_sub(r.input_prefix.chars().count() as u32) as usize;
        println!("{}{}{}", r.input_prefix, " ".repeat(pad), r.right_prompt);
    }
    if !r.colors.is_empty() {
        let mut colors: Vec<(&String, &String)> = r.colors.iter().collect();
        colors.sort_by_key(|(k, _)| *k);
        for (k, v) in colors {
            println!("  color {k} = {v}");
        }
    }
    for (k, v) in &r.extra {
        println!("  extra {k} = {v}");
    }
    println!(
        "  framebuffer {}x{} stride {} | {}",
        g.width,
        g.height,
        g.stride,
        g.logo_text()
    );
    if let Some(w) = &g.logo_warning {
        println!("  warning: {w}");
    }
}
