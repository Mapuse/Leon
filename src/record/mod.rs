//! Persistence of the real boot geometry for host tooling.
//!
//! Every boot the loader writes the live GOP frame buffer, the BGRT logo
//! metadata, and the resolved boot config as JSON on the boot volume, so host
//! tools (`lbt`) mirror exactly what this machine's firmware provided — never
//! any assumed resolution or logo. See `dump` for the schema.
//!
//! Note: this is a pure *record*. The kernel is an EFI-stub UEFI application
//! and queries GOP and BGRT itself; there is no handoff blob.

mod dump;

pub use dump::write;
