//! `lbc` — Leon Boot Configuration.
//!
//! The host-side companion that owns boot configuration and boot control:
//! reading and writing the boot config (`~/.config/leon/boot.toml`, mirrored
//! onto every mounted ESP as `\EFI\leon\boot.toml`), staging the ESP tree, the
//! boot-manager menu (TUI), and reports of the boot volume layout. It reuses
//! the shared host-side modules (`discovery`, `geometry`, helpers) from the
//! `lbt` library.

mod boot_config;
mod cli;
mod commands;

fn main() -> anyhow::Result<()> {
    cli::run()
}
