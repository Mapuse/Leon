use anyhow::Result;

mod boot_config;
mod cli;
mod commands;
mod discovery;
mod geometry;

fn main() -> Result<()> {
    cli::run()
}
