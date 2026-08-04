//! One module per `lbt` subcommand.
//!
//! [`info`], [`discover`], and [`config`] are always compiled; the Python-backed
//! [`theme`], [`plugin`], and [`tui`] modules exist only under the `python`
//! feature.

pub mod config;
pub mod discover;
pub mod info;

#[cfg(feature = "python")]
pub mod plugin;
#[cfg(feature = "python")]
pub mod theme;
#[cfg(feature = "python")]
pub mod tui;
