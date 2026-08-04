//! Sanity-checking the memory map the kernel now owns.
//!
//! The stub has no console, so "consuming" the map means proving it is
//! structurally sound: a descriptor size too small for a `MemoryDescriptor`,
//! or a total byte range that would wrap, would crash a real kernel the same
//! way it crashes this one.

use core::mem::size_of;

use uefi::mem::memory_map::{MemoryDescriptor, MemoryMap};

/// Sanity-checks the memory map the stub now owns.
pub fn validate_map(map: &impl MemoryMap) -> bool {
    let desc_size = map.meta().desc_size;
    let count = map.meta().entry_count();
    let ok = desc_size >= size_of::<MemoryDescriptor>()
        && count.checked_mul(desc_size).is_some()
        && !map.buffer().is_empty();
    // Keep the values alive so the map cannot be optimized away.
    core::hint::black_box((map.buffer(), desc_size, count));
    ok
}
