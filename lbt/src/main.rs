use anyhow::Result;

mod boot_config;
mod cli;
mod commands;
mod discovery;
mod geometry;

#[cfg(feature = "python")]
mod python;

fn main() -> Result<()> {
    cli::run()
}
