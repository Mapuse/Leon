//! UI primitives for a unified boot menu renderer usable from UEFI `lbl`.
//!
//! This module is a scaffold for the Rust-side TUI: a GOP framebuffer-backed
//! renderer and minimal widget primitives. It is intentionally small and will
//! be expanded to match the host ratatui menu layout.

pub mod gop;
pub use gop::GopRenderer;
