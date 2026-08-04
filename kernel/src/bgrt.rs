//! ACPI BGRT discovery for the EFI-stub kernel.
//!
//! Mirrors `lbl`'s finder: walks from the RSDP in the configuration table to
//! the XSDT, scans for the BGRT signature, and validates the logo BMP header —
//! all read-only, never touching the logo pixels themselves.

use core::ptr;

use uefi::system::with_config_table;
use uefi::table::cfg::ConfigTableEntry;

use leon_common::Bgrt;

/// Locates and validates the ACPI BGRT, mirroring `lbl`'s finder.
pub fn find_bgrt() -> Option<Bgrt> {
    let rsdp = with_config_table(|entries| {
        entries
            .iter()
            .find(|e| e.guid == ConfigTableEntry::ACPI2_GUID)
            .map(|e| e.address as u64)
    })?;
    if rsdp == 0 || unsafe { ptr::read_volatile(rsdp as *const u8) } != b'R' {
        return None;
    }
    // SAFETY: `rsdp` came from the configuration table and points at a valid
    // ACPI 2.0 RSDP (XSDT address lives at offset 24).
    let revision = unsafe { ptr::read_volatile((rsdp + 15) as *const u8) };
    if revision < 2 {
        return None;
    }
    let xsdt = unsafe { ptr::read_unaligned((rsdp + 24) as *const u64) };
    if xsdt == 0 {
        return None;
    }
    let bgrt = find_table(xsdt, *b"BGRT")?;
    parse_bgrt(bgrt).filter(|b| b.image_address != 0)
}

/// Scans an XSDT (array of 64-bit table pointers) for a table signature.
fn find_table(xsdt: u64, sig: [u8; 4]) -> Option<u64> {
    // SAFETY: `xsdt` is a valid ACPI table (validated by its own checksum).
    let length = unsafe { ptr::read_unaligned((xsdt + 4) as *const u32) } as usize;
    if length < 36 {
        return None;
    }
    let count = (length - 36) / 8;
    for i in 0..count {
        // SAFETY: Table pointer array is within the XSDT length.
        let entry = unsafe { ptr::read_unaligned((xsdt + (36 + i * 8) as u64) as *const u64) };
        if entry == 0 {
            continue;
        }
        // SAFETY: `entry` is a valid table pointer from the XSDT.
        let header = entry as *const u8;
        let ok = unsafe {
            ptr::read_volatile(header) == sig[0]
                && ptr::read_volatile(header.add(1)) == sig[1]
                && ptr::read_volatile(header.add(2)) == sig[2]
                && ptr::read_volatile(header.add(3)) == sig[3]
        };
        if ok {
            return Some(entry);
        }
    }
    None
}

/// BGRT layout, see ACPI 6.x spec "Boot Graphics Resource Table".
#[repr(C)]
struct BgrtTable {
    _header: [u8; 36],
    _version: u16,
    status: u8,
    image_type: u8,
    image_address: u64,
    image_offset_x: u32,
    image_offset_y: u32,
}

fn parse_bgrt(addr: u64) -> Option<Bgrt> {
    // SAFETY: `addr` points at a valid BGRT located by signature scan.
    let table = unsafe { &*(addr as *const BgrtTable) };
    let image_address = table.image_address;
    let (width, height) = parse_bmp_dims(image_address)?;
    Some(Bgrt {
        image_address,
        offset_x: table.image_offset_x as i32,
        offset_y: table.image_offset_y as i32,
        image_width: width,
        image_height: height,
        status: table.status,
        image_type: table.image_type,
    })
}

/// Reads the pixel dimensions out of the logo BMP, without touching its data.
fn parse_bmp_dims(addr: u64) -> Option<(u32, u32)> {
    if addr == 0 || addr > u64::from(u32::MAX) {
        return None;
    }
    let p = addr as *const u8;
    // SAFETY: The image lives in firmware-reserved RAM, identity-mapped by UEFI.
    unsafe {
        if ptr::read_volatile(p) != b'B' || ptr::read_volatile(p.add(1)) != b'M' {
            return None;
        }
        let width = ptr::read_unaligned(p.add(18) as *const i32);
        let height = ptr::read_unaligned(p.add(22) as *const i32);
        let bpp = ptr::read_unaligned(p.add(28) as *const u16);
        if width <= 0 || height == 0 || bpp == 0 {
            return None;
        }
        if width > 16384 || height.abs() > 16384 {
            return None;
        }
        Some((width as u32, height.unsigned_abs()))
    }
}
