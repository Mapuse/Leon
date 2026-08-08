//! The `lbt` library surface.
//!
//! The boot-manager, boot-config, and boot-control commands moved to the `lbc`
//! binary (Leon Boot Configuration). What remains here — and what `lbc`
//! depends on — is the host-side discovery, geometry, and tool helpers, plus
//! `lbt`'s own CLI, filesystem helpers, and the native image builders.

pub mod cli;
pub mod commands;
pub mod discovery;
pub mod geometry;
pub mod img;
pub mod misc;
