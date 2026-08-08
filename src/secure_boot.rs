//! Secure Boot state detection.
//!
//! The loader reads the `SecureBoot` / `SetupMode` global variables so it can
//! warn when image verification is active: an unsigned entry is then rejected
//! by the firmware's `LoadImage`, and knowing that up front beats a terse
//! `SECURITY_VIOLATION` at handoff time. This is a read-only query — the
//! loader never tries to change the Secure Boot policy, it just reports it.

use uefi::CStr16;
use uefi::cstr16;
use uefi::runtime::{VariableVendor, get_variable};

/// What the platform reports about Secure Boot enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecureBootState {
    /// `SecureBoot` is set: image verification is active and unsigned entries
    /// will be rejected at `LoadImage` time.
    Enabled,
    /// `SecureBoot` is cleared: no image verification is enforced.
    Disabled,
    /// The state could not be read (variables not exposed by this firmware).
    Unknown,
}

/// Reads the Secure Boot state from the `SecureBoot`/`SetupMode` global
/// variables. Best-effort: any read failure collapses to [`Unknown`], so a
/// missing variable never breaks the boot.
pub fn state() -> SecureBootState {
    match read_u8(cstr16!("SecureBoot")) {
        Some(1) => SecureBootState::Enabled,
        Some(0) => SecureBootState::Disabled,
        _ => SecureBootState::Unknown,
    }
}

/// Reads a one-byte global variable, or `None` when unreadable.
fn read_u8(name: &CStr16) -> Option<u8> {
    let mut buf = [0u8; 1];
    get_variable(name, &VariableVendor::GLOBAL_VARIABLE, &mut buf)
        .ok()
        .map(|(val, _)| val[0])
}

/// A short, static warning string for a Secure Boot state, or `None` when the
/// state needs no warning.
pub fn warning(state: SecureBootState) -> Option<&'static str> {
    match state {
        SecureBootState::Enabled => Some("Secure Boot is ON - unsigned entries will be rejected"),
        SecureBootState::Disabled | SecureBootState::Unknown => None,
    }
}
