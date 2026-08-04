//! Command-line surface of `lbt` and the dispatch into [`crate::commands`].
//!
//! Everything here is declarative clap derive. The Python-backed subcommands
//! (`theme`, `plugin`, `tui`) are compiled out when the `python` feature is off,
//! so a plain `cargo build` links zero pyo3 code.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands;

#[derive(Parser)]
#[command(
    name = "lbt",
    version,
    about = "Leon Build Tool — author/preview boot themes, run TUIs/plugins, manage boot config"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print framebuffer + BGRT logo geometry (live sysfs, or a BootInfo dump).
    Info { dump: Option<PathBuf> },
    /// Auto-detect every EFI System Partition and its boot entries.
    Discover {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    #[cfg(feature = "python")]
    Theme {
        #[command(subcommand)]
        cmd: ThemeCommand,
    },
    #[cfg(feature = "python")]
    Plugin {
        #[command(subcommand)]
        cmd: PluginCommand,
    },
    #[cfg(feature = "python")]
    Tui {
        /// No subcommand runs the configured default TUI (the shipped Leon
        /// menu when none is configured).
        #[command(subcommand)]
        cmd: Option<TuiCommand>,
    },
    Config {
        #[command(subcommand)]
        cmd: ConfigCommand,
    },
}

#[cfg(feature = "python")]
#[derive(Subcommand)]
pub enum ThemeCommand {
    /// List registered boot themes.
    List,
    /// Render a theme's splash/menu against Leon geometry (live sysfs, or a
    /// BootInfo dump written by the bootloader).
    Preview { name: String, dump: Option<PathBuf> },
    /// Invoke a theme's full-screen run().
    Run { name: String, dump: Option<PathBuf> },
}

#[cfg(feature = "python")]
#[derive(Subcommand)]
pub enum PluginCommand {
    /// List registered plugins.
    List,
    /// Run a plugin (by name or alias) with args.
    Run {
        name: String,
        func: String,
        #[arg(num_args = 0..)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Print a boot config key (or all keys).
    Get { key: Option<String> },
    /// Set a boot config key (timeout, default_entry, theme, splash).
    Set { key: String, value: String },
}

#[cfg(feature = "python")]
#[derive(Subcommand)]
pub enum TuiCommand {
    /// List registered Python TUIs.
    List,
    /// Run a TUI's run() in this process. Without a name, the configured
    /// default TUI runs (the shipped Leon menu when none is configured).
    Run {
        /// Registered TUI name; defaults to the configured TUI.
        name: Option<String>,
    },
    /// Persist a TUI as the active `[python]` tui in `~/.config/leon/python.toml`.
    Apply { name: String },
}

/// Runs the parsed command.
pub fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Info { dump } => commands::info::run(dump),
        Command::Discover { json } => commands::discover::run(json),
        #[cfg(feature = "python")]
        Command::Theme { cmd } => commands::theme::run(cmd),
        #[cfg(feature = "python")]
        Command::Plugin { cmd } => commands::plugin::run(cmd),
        #[cfg(feature = "python")]
        Command::Tui { cmd } => commands::tui::run(cmd),        Command::Config { cmd } => commands::config::run(cmd),
    }
}
