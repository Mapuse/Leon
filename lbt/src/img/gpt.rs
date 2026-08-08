//! Native GPT partition table writer, with a protective MBR.
//!
//! Writes the primary GPT (header at LBA 1, 128 entries from LBA 2), the
//! protective MBR (LBA 0) and the backup GPT at the end of the disk, with
//! correct CRC32 checksums. No `fdisk`/`sgdisk` involved.

use std::fs::File;
use std::os::unix::fs::FileExt;

use anyhow::{Result, bail};

use super::crc32;

/// GPT partition-type GUID for an EFI System Partition.
pub const PART_ESP: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

/// GPT partition-type GUID for a Linux filesystem partition.
pub const PART_LINUX: [u8; 16] = [
    0xaf, 0x3d, 0xc6, 0x0f, 0x84, 0x83, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4,
];

const SECTOR: u64 = 512;
const PART_ENTRY_SIZE: u32 = 128;
const PART_ENTRIES: u32 = 128;

/// One partition in the table.
pub struct Partition {
    pub start_lba: u64,
    pub end_lba: u64,
    pub type_guid: [u8; 16],
    pub name: String,
}

fn disk_guid() -> [u8; 16] {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64 ^ (std::process::id() as u64) << 32)
        .unwrap_or(0x9E3779B97F4A7C15);
    let mut g = [0u8; 16];
    let mut s = seed;
    for b in g.iter_mut() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s >> 32) as u8 ^ (seed & 0xFF) as u8;
    }
    g
}

/// Writes a GPT (plus protective MBR) to `f`, a file of `disk_size` bytes.
pub fn write_gpt(f: &mut File, disk_size: u64, parts: &[Partition]) -> Result<()> {
    if disk_size < 34 * SECTOR + SECTOR {
        bail!("disk too small for a GPT: {disk_size} bytes");
    }
    let last_lba = disk_size / SECTOR - 1;
    let first_usable = 34u64;
    let last_usable = last_lba.saturating_sub(33);
    for p in parts {
        if p.start_lba < first_usable || p.end_lba > last_usable {
            bail!(
                "partition {} out of the usable region ({first_usable}..{last_usable})",
                p.name
            );
        }
    }

    // Protective MBR.
    let mut mbr = [0u8; 512];
    mbr[0x1BE] = 0x00; // status: inactive
    mbr[0x1BE + 1] = 0x00; // CHS start
    mbr[0x1BE + 2] = 0x02;
    mbr[0x1BE + 3] = 0x00;
    mbr[0x1BE + 4] = 0xEE; // GPT protective
    mbr[0x1BE + 5] = 0xFF;
    mbr[0x1BE + 6] = 0xFF;
    mbr[0x1BE + 7] = 0xFF;
    mbr[0x1BE + 8..0x1BE + 12].copy_from_slice(&1u32.to_le_bytes()); // start LBA
    // Size in sectors: capped at 0xFFFFFFFF per the GPT spec when the disk is
    // too large to represent (disks over ~2 TiB).
    let sectors = disk_size / SECTOR - 1;
    let sectors = if sectors > u64::from(u32::MAX) {
        u32::MAX
    } else {
        sectors as u32
    };
    mbr[0x1BE + 12..0x1BE + 16].copy_from_slice(&sectors.to_le_bytes());
    mbr[0x1FE] = 0x55;
    mbr[0x1FF] = 0xAA;
    f.write_all_at(&mbr, 0)?;

    // Primary GPT header (LBA 1).
    let mut entries = Vec::with_capacity((PART_ENTRIES * PART_ENTRY_SIZE) as usize);
    for p in parts {
        let mut e = [0u8; 128];
        e[0..16].copy_from_slice(&p.type_guid);
        e[16..32].copy_from_slice(&disk_guid());
        e[32..40].copy_from_slice(&p.start_lba.to_le_bytes());
        e[40..48].copy_from_slice(&p.end_lba.to_le_bytes());
        e[48..56].copy_from_slice(&0u64.to_le_bytes()); // attributes
        e[56..128].fill(0);
        let name_bytes = p.name.encode_utf16().collect::<Vec<_>>();
        for (i, ch) in name_bytes.iter().enumerate().take(36) {
            e[56 + i * 2] = (ch & 0xFF) as u8;
            e[56 + i * 2 + 1] = (ch >> 8) as u8;
        }
        entries.extend_from_slice(&e);
    }
    // Zero the remaining partition entries.
    entries.resize((PART_ENTRIES * PART_ENTRY_SIZE) as usize, 0);

    let entries_crc = crc32(&entries);
    let guid = disk_guid();

    let mut hdr = [0u8; 92];
    hdr[0..8].copy_from_slice(b"EFI PART");
    hdr[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    hdr[12..16].copy_from_slice(&92u32.to_le_bytes());
    // header CRC at 16..20, filled after.
    hdr[24..32].copy_from_slice(&1u64.to_le_bytes()); // current LBA
    hdr[32..40].copy_from_slice(&last_lba.to_le_bytes()); // backup LBA
    hdr[40..48].copy_from_slice(&first_usable.to_le_bytes());
    hdr[48..56].copy_from_slice(&last_usable.to_le_bytes());
    hdr[56..72].copy_from_slice(&guid);
    hdr[72..80].copy_from_slice(&2u64.to_le_bytes()); // partition entry LBA
    hdr[80..84].copy_from_slice(&PART_ENTRIES.to_le_bytes());
    hdr[84..88].copy_from_slice(&PART_ENTRY_SIZE.to_le_bytes());
    hdr[88..92].copy_from_slice(&entries_crc.to_le_bytes());
    let hdr_crc = crc32(&hdr);
    hdr[16..20].copy_from_slice(&hdr_crc.to_le_bytes());

    f.write_all_at(&hdr, SECTOR)?;
    f.write_all_at(&entries, 2 * SECTOR)?;

    // Backup GPT at the end.
    let backup_entries_lba = last_lba - 32;
    let mut bhdr = [0u8; 92];
    bhdr[0..8].copy_from_slice(b"EFI PART");
    bhdr[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    bhdr[12..16].copy_from_slice(&92u32.to_le_bytes());
    bhdr[24..32].copy_from_slice(&last_lba.to_le_bytes());
    bhdr[32..40].copy_from_slice(&1u64.to_le_bytes());
    bhdr[40..48].copy_from_slice(&first_usable.to_le_bytes());
    bhdr[48..56].copy_from_slice(&last_usable.to_le_bytes());
    bhdr[56..72].copy_from_slice(&guid);
    bhdr[72..80].copy_from_slice(&backup_entries_lba.to_le_bytes());
    bhdr[80..84].copy_from_slice(&PART_ENTRIES.to_le_bytes());
    bhdr[84..88].copy_from_slice(&PART_ENTRY_SIZE.to_le_bytes());
    bhdr[88..92].copy_from_slice(&entries_crc.to_le_bytes());
    let bhdr_crc = crc32(&bhdr);
    bhdr[16..20].copy_from_slice(&bhdr_crc.to_le_bytes());

    f.write_all_at(&entries, backup_entries_lba * SECTOR)?;
    f.write_all_at(&bhdr, last_lba * SECTOR)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpt_layout_has_valid_checksums() {
        let tmp = std::env::temp_dir().join("lbt_gpt.img");
        let _ = std::fs::remove_file(&tmp);
        let size = 128 * 1024 * 1024u64;
        let mut f = File::options()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            .open(&tmp)
            .unwrap();
        f.set_len(size).unwrap();
        write_gpt(
            &mut f,
            size,
            &[
                Partition {
                    start_lba: 2048,
                    end_lba: 2048 + 131_071,
                    type_guid: PART_ESP,
                    name: "EFI System Partition".to_string(),
                },
                Partition {
                    start_lba: 2048 + 131_072,
                    end_lba: size / SECTOR - 34,
                    type_guid: PART_LINUX,
                    name: "Leon Root".to_string(),
                },
            ],
        )
        .unwrap();
        drop(f);

        let buf = std::fs::read(&tmp).unwrap();
        assert_eq!(&buf[0x1FE..0x200], &[0x55, 0xAA], "MBR signature");
        assert_eq!(&buf[0x1BE + 4..0x1BE + 5], &[0xEE], "protective type");
        assert_eq!(&buf[512..520], b"EFI PART", "GPT signature");
        let hdr_crc = u32::from_le_bytes([buf[528], buf[529], buf[530], buf[531]]);
        let mut hdr = buf[512..512 + 92].to_vec();
        hdr[16..20].fill(0);
        assert_eq!(crc32(&hdr), hdr_crc, "header checksum matches");
        let last_lba = size / SECTOR - 1;
        let back = (last_lba * SECTOR) as usize;
        assert_eq!(&buf[back..back + 8], b"EFI PART", "backup GPT present");
        let _ = std::fs::remove_file(&tmp);
    }
}
