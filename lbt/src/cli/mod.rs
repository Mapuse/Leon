//! Command-line surface of `lbt` and the dispatch into [`crate::commands`].
//!
//! The argument parser and the command tree live in [`args`] and [`tree`]; this
//! module only wires them to [`crate::commands`] and prints help.

pub mod args;
pub mod tree;

use anyhow::Result;

use self::args::{Action, empty, parse};
use self::tree::{ROOT, help_for};
use crate::commands;

/// Runs the command line, printing the root help for an empty invocation.
pub fn run() -> Result<()> {
    if version_requested() {
        println!("lbt {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if empty() {
        print!("{}", help_for(&ROOT));
        return Ok(());
    }
    match parse()? {
        Action::Help(node) => print!("{}", help_for(node)),
        Action::Run(node, parsed) => commands::dispatch(node, parsed)?,
    }
    Ok(())
}

/// Whether the invocation is a bare `-v` / `--version` (the root declares no
/// such flag, so it must be intercepted before parsing).
fn version_requested() -> bool {
    std::env::args()
        .skip(1)
        .any(|a| a == "-v" || a == "--version" || a == "-V")
}
