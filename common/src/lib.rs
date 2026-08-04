//! Shared types between `lbl` (the bootloader) and `lbl-kernel` (the EFI-stub
//! kernel it chainloads).
//!
//! The loader and the kernel no longer exchange a handoff blob: the kernel is a
//! plain UEFI application (an "EFI stub") that queries the GOP frame buffer and
//! the ACPI BGRT itself after being started with `LoadImage`/`StartImage`.
//! What is shared is the *representation* of those firmware facts — the pixel
//! formats, the frame buffer geometry, the BGRT metadata (see [`geometry`]) —
//! plus the `no_std` parser for `\EFI\leon\boot.toml`.
//!
//! With the `boot-config` feature (on by default) the crate hosts the `no_std`
//! parser shared between `lbl` (which validates the file at every boot) and
//! `lbt` (which writes it).

#![no_std]

#[cfg(feature = "boot-config")]
pub mod boot_config;

mod geometry;

pub use geometry::{Bgrt, Framebuffer, PixelFormat};
