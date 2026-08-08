//! Command-line surface of `lbt` and the dispatch into [`crate::commands`].
//!
//! Everything here is declarative clap derive. `lbt` is a pure-Rust host tool:
//! geometry, discovery, the ratatui boot-manager menu, and boot config.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands;

#[derive(Parser)]
#[command(
    name = "lbt",
    version,
    about = "Leon Build Tool — discover boot entries, run the boot-manager TUI, manage boot config"
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
    /// Run the keyboard-driven Leon boot-manager menu (a ratatui TUI).
    Tui,
    Config {
        #[command(subcommand)]
        cmd: ConfigCommand,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Print a boot config key (or all keys).
    Get { key: Option<String> },
    /// Set a boot config key (timeout, default_entry, theme, splash).
    Set { key: String, value: String },
}

/// Runs the parsed command.
pub fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Info { dump } => commands::info::run(dump),
        Command::Discover { json } => commands::discover::run(json),
        Command::Tui => commands::tui::run(),
        Command::Config { cmd } => commands::config::run(cmd),
    }
}
