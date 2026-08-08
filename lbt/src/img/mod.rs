//! Native disk-image formats, implemented from scratch in lbt (no wrapper
//! around xorriso / mkisofs / genisoimage / fdisk / sgdisk / mtools):
//!
//! * [`fat`] — FAT12/16/32 filesystem writer (boot sector, FAT, directories).
//! * [`gpt`] — GPT partition table with a protective MBR.
//!
//! Everything is pure Rust over `std`.

pub mod fat;
pub mod gpt;

/// Standard CRC-32 (IEEE 802.3), used by GPT headers and DMG trailers.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }
}
