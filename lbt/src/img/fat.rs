//! Native FAT12/16/32 filesystem writer.
//!
//! Formats a volume image and populates it from a directory tree, all in Rust
//! (no mtools). Used by the ESP, raw, IMG, VHD, VMDK and DMG builders.
//!
//! Layout follows the Microsoft FAT spec: boot sector (plus FSInfo and backup
//! boot for FAT32), FAT table(s), the root directory (a plain cluster chain
//! for FAT32, a fixed region for FAT12/16), then the data region. Files get
//! short (8.3) names plus VFAT long-name entries when needed.

use std::collections::HashSet;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub const BYTES_PER_SECTOR: u64 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatKind {
    Fat12,
    Fat16,
    Fat32,
}

impl FatKind {
    pub fn fs_label(self) -> &'static str {
        match self {
            FatKind::Fat12 => "FAT12   ",
            FatKind::Fat16 => "FAT16   ",
            FatKind::Fat32 => "FAT32   ",
        }
    }
    fn bits(self) -> u64 {
        match self {
            FatKind::Fat12 => 12,
            FatKind::Fat16 => 16,
            FatKind::Fat32 => 32,
        }
    }
    fn eoc(self) -> u32 {
        match self {
            FatKind::Fat12 => 0xFFF,
            FatKind::Fat16 => 0xFFFF,
            FatKind::Fat32 => 0x0FFF_FFFF,
        }
    }
}

/// Computed layout for a volume of `total_sectors` sectors.
struct Geometry {
    kind: FatKind,
    spc: u32,
    reserved: u32,
    fat_sectors: u32,
    num_fats: u32,
    root_entries: u32,
    root_sectors: u64,
    total_sectors: u64,
    data_start: u64,
    total_clusters: u32,
    root_cluster: u32,
}

impl Geometry {
    fn cluster_bytes(&self) -> u64 {
        u64::from(self.spc) * BYTES_PER_SECTOR
    }

    /// Byte offset of a data cluster.
    fn cluster_offset(&self, cluster: u32) -> u64 {
        self.data_start + u64::from(cluster - 2) * self.cluster_bytes()
    }

    /// Byte offset of FAT `n`.
    fn fat_offset(&self, n: u32) -> u64 {
        (u64::from(self.reserved) + u64::from(self.fat_sectors) * u64::from(n))
            * BYTES_PER_SECTOR
    }

    /// Byte offset of the fixed root directory (FAT12/16 only).
    fn root_offset(&self) -> u64 {
        self.data_start - self.root_sectors * BYTES_PER_SECTOR
    }
}

/// Picks `(kind, spc)` so the cluster count lands in the type's valid range,
/// matching the defaults of the standard formatting tools.
fn geometry(total_sectors: u64) -> Result<Geometry> {
    if total_sectors < 32 + 4 {
        bail!("volume too small: {total_sectors} sectors");
    }
    for spc in [1u32, 2, 4, 8, 16, 32, 64, 128] {
        for kind in [FatKind::Fat12, FatKind::Fat16, FatKind::Fat32] {
            let reserved = if kind == FatKind::Fat32 { 32 } else { 1 };
            let root_entries = if kind == FatKind::Fat32 { 0 } else { 512 };
            let root_sectors = (u64::from(root_entries) * 32).div_ceil(BYTES_PER_SECTOR);

            // Iterate the FAT size to a fixed point so the data region is
            // computed against the real overhead.
            let mut fat_sectors = 1u32;
            loop {
                let overhead =
                    u64::from(reserved) + u64::from(fat_sectors) * 2 + root_sectors;
                if overhead >= total_sectors {
                    break;
                }
                let clusters = (total_sectors - overhead) / u64::from(spc);
                let bytes = clusters * kind.bits() / 8 + BYTES_PER_SECTOR;
                let need = bytes.div_ceil(BYTES_PER_SECTOR) as u32;
                if need == fat_sectors {
                    break;
                }
                fat_sectors = need.max(1);
            }

            let overhead = u64::from(reserved) + u64::from(fat_sectors) * 2 + root_sectors;
            if overhead >= total_sectors {
                continue;
            }
            let clusters = (total_sectors - overhead) / u64::from(spc);
            let fits = match kind {
                FatKind::Fat12 => (1..=4084).contains(&clusters),
                FatKind::Fat16 => (4085..=65_524).contains(&clusters),
                FatKind::Fat32 => (65_525..=0x0FFF_FFF4).contains(&clusters),
            };
            if fits {
                return Ok(Geometry {
                    kind,
                    spc,
                    reserved,
                    fat_sectors,
                    num_fats: 2,
                    root_entries,
                    root_sectors,
                    total_sectors,
                    data_start: overhead * BYTES_PER_SECTOR,
                    total_clusters: clusters as u32,
                    root_cluster: 2,
                });
            }
        }
    }
    bail!("no FAT geometry fits {total_sectors} sectors")
}

/// Options for formatting a volume.
pub struct FatOptions {
    pub label: String,
    pub size_bytes: u64,
}

/// A node of the tree copied into the volume.
struct Node {
    name: String,
    parent: usize,
    is_dir: bool,
    size: u64,
    src: Option<PathBuf>,
    children: Vec<usize>,
    first_cluster: u32,
    chain_len: u32,
    short: [u8; 11],
    needs_lfn: bool,
}

impl Node {
    /// The cluster the `..` entry must point to: the parent's first cluster,
    /// or 0 when the parent is the root directory (which has no cluster).
    fn parent_first_cluster(&self, nodes: &[Node]) -> u32 {
        if self.parent == 0 {
            0
        } else {
            nodes[self.parent].first_cluster
        }
    }
}

/// Formats `src` (or only its top-level entries named in `only`) into a FAT
/// volume written into `f`. Returns the number of clusters.
pub fn write_fat(
    f: &mut File,
    opts: &FatOptions,
    src: Option<&Path>,
    only: &[&str],
) -> Result<u32> {
    let total_sectors = opts.size_bytes / BYTES_PER_SECTOR;
    let geo = geometry(total_sectors)?;
    let cluster_bytes = geo.cluster_bytes();

    // ── 1. Build the tree ──────────────────────────────────────────────
    let mut nodes: Vec<Node> = vec![Node {
        name: String::new(),
        parent: 0,
        is_dir: true,
        size: 0,
        src: None,
        children: Vec::new(),
        first_cluster: geo.root_cluster,
        chain_len: 0,
        short: *b"           ",
        needs_lfn: false,
    }];
    if let Some(root) = src {
        let mut used = HashSet::new();
        build_tree(root, &mut nodes, 0, only, &mut used)?;
    }

    // ── 2. Allocate clusters ───────────────────────────────────────────
    // Directory entry sizes depend only on child names, so every chain length
    // is known before any cluster is handed out. Directories are allocated in
    // post-order (children first), files after.
    let mut fat: Vec<u32> = vec![0; geo.total_clusters as usize + 2];
    fat[0] = match geo.kind {
        FatKind::Fat12 => 0xFF8,
        FatKind::Fat16 => 0xFFF8,
        FatKind::Fat32 => 0x0FFF_FFF8,
    };
    fat[1] = geo.kind.eoc();

    for i in 0..nodes.len() {
        if nodes[i].is_dir {
            let bytes = dir_bytes(&nodes, i, &opts.label);
            nodes[i].chain_len = (bytes.len() as u64).div_ceil(cluster_bytes) as u32;
        }
    }

    // The FAT32 root directory is a fixed cluster chain that MUST start at
    // cluster 2 (the boot sector's BPB_RootClus points there). Allocate it in
    // place; the remaining clusters (3..) are handed out in post-order so
    // children land before their parents. FAT12/16 keep a fixed root region,
    // so their root node takes no clusters at all.
    let mut next = geo.root_cluster + 1;
    if geo.kind == FatKind::Fat32 {
        let root_chain = nodes[0].chain_len;
        for c in 0..root_chain {
            let idx = (geo.root_cluster + c) as usize;
            fat[idx] = if c + 1 < root_chain {
                geo.root_cluster + c + 1
            } else {
                geo.kind.eoc()
            };
        }
        next = geo.root_cluster + root_chain;
    }
    let mut order = Vec::new();
    post_order(&nodes, 0, &mut order);
    for i in order {
        if i == 0 {
            continue;
        }
        let need = nodes[i].chain_len as u64;
        if need == 0 {
            nodes[i].first_cluster = 0;
            continue;
        }
        let first = next;
        for c in 0..need {
            let idx = (first + c as u32) as usize;
            fat[idx] = if c + 1 < need {
                first + c as u32 + 1
            } else {
                geo.kind.eoc()
            };
        }
        nodes[i].first_cluster = first;
        next += need as u32;
    }
    for node in &mut nodes {
        if node.is_dir {
            continue;
        }
        let need = node.size.div_ceil(cluster_bytes);
        node.chain_len = need as u32;
        if need == 0 {
            node.first_cluster = 0;
            continue;
        }
        let first = next;
        for c in 0..need {
            let idx = (first + c as u32) as usize;
            fat[idx] = if c + 1 < need {
                first + c as u32 + 1
            } else {
                geo.kind.eoc()
            };
        }
        node.first_cluster = first;
        next += need as u32;
    }

    // ── 3. Boot sector, FSInfo, backup boot ────────────────────────────
    let mut boot = [0u8; 512];
    write_boot_sector(&mut boot, &geo, &opts.label)?;
    f.write_all_at(&boot, 0).with_context(|| "writing the boot sector")?;
    if geo.kind == FatKind::Fat32 {
        let mut fsinfo = [0u8; 512];
        fsinfo[0..4].copy_from_slice(b"RRaA");
        fsinfo[484..488].copy_from_slice(b"rrAa");
        // Real free-cluster count and next-free hint: clusters 2..next-1 are
        // all allocated by the time this runs (directory and file chains).
        let free = u64::from(geo.total_clusters)
            .saturating_sub(u64::from(next.saturating_sub(geo.root_cluster)));
        fsinfo[488..492].copy_from_slice(&(free.min(u32::MAX as u64) as u32).to_le_bytes());
        fsinfo[492..496].copy_from_slice(&next.to_le_bytes());
        fsinfo[510..512].copy_from_slice(&[0x55, 0xAA]);
        f.write_all_at(&fsinfo, BYTES_PER_SECTOR).with_context(|| "writing FSInfo")?;
        f.write_all_at(&boot, 6 * BYTES_PER_SECTOR)
            .with_context(|| "writing the backup boot sector")?;
    }

    // ── 4. Directories ─────────────────────────────────────────────────
    if geo.kind == FatKind::Fat32 {
        for i in 0..nodes.len() {
            if nodes[i].is_dir {
                let bytes = dir_bytes(&nodes, i, &opts.label);
                write_clusters(f, &geo, &nodes[i], &bytes)?;
            }
        }
    } else {
        // Root is a fixed region; only its contents are written there.
        let bytes = dir_bytes(&nodes, 0, &opts.label);
        let cap = geo.root_sectors * BYTES_PER_SECTOR;
        if bytes.len() as u64 > cap {
            bail!("root directory does not fit the fixed root region");
        }
        let mut buf = vec![0u8; cap as usize];
        buf[..bytes.len()].copy_from_slice(&bytes);
        f.write_all_at(&buf, geo.root_offset())
            .with_context(|| "writing the root directory")?;
        for i in 1..nodes.len() {
            if nodes[i].is_dir {
                let bytes = dir_bytes(&nodes, i, &opts.label);
                write_clusters(f, &geo, &nodes[i], &bytes)?;
            }
        }
    }

    // ── 5. File data (streamed cluster by cluster) ─────────────────────
    for node in &nodes {
        if node.is_dir {
            continue;
        }
        let Some(path) = &node.src else {
            continue;
        };
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let chain = cluster_chain(node);
        if chain.is_empty() {
            continue;
        }
        let mut left = node.size;
        for (k, cl) in chain.iter().enumerate() {
            let offset = geo.cluster_offset(*cl);
            let read = cluster_bytes.min(left) as usize;
            let mut buf = vec![0u8; cluster_bytes as usize];
            file.read_exact_at(&mut buf[..read], k as u64 * cluster_bytes)
                .with_context(|| format!("reading {}", path.display()))?;
            f.write_all_at(&buf, offset).with_context(|| "writing file data")?;
            left = left.saturating_sub(cluster_bytes);
        }
    }

    // ── 6. FAT table(s) ────────────────────────────────────────────────
    let fat_bytes = pack_fat(&fat, geo.kind);
    for n in 0..geo.num_fats {
        f.write_all_at(&fat_bytes, geo.fat_offset(n))
            .with_context(|| "writing the FAT")?;
    }

    Ok(geo.total_clusters)
}

fn cluster_chain(node: &Node) -> Vec<u32> {
    if node.chain_len == 0 {
        return Vec::new();
    }
    (node.first_cluster..node.first_cluster + node.chain_len).collect()
}

fn post_order(nodes: &[Node], i: usize, out: &mut Vec<usize>) {
    for &c in &nodes[i].children {
        post_order(nodes, c, out);
    }
    out.push(i);
}

/// Writes a buffer across a node's cluster chain, zero-filling the tail.
fn write_clusters(f: &mut File, geo: &Geometry, node: &Node, bytes: &[u8]) -> Result<()> {
    let cluster_bytes = geo.cluster_bytes();
    let chain = cluster_chain(node);
    let mut pos = 0usize;
    for (k, cl) in chain.iter().enumerate() {
        let offset = geo.cluster_offset(*cl);
        let this = if k + 1 < chain.len() {
            cluster_bytes as usize
        } else {
            bytes.len().saturating_sub(pos)
        };
        let mut buf = vec![0u8; cluster_bytes as usize];
        if this > 0 {
            buf[..this].copy_from_slice(&bytes[pos..pos + this]);
        }
        f.write_all_at(&buf, offset).with_context(|| "writing a directory")?;
        pos += this;
    }
    Ok(())
}

/// Serializes a directory's entry stream (including "." / ".." and the volume
/// label for the root).
fn dir_bytes(nodes: &[Node], idx: usize, label: &str) -> Vec<u8> {
    let mut out = Vec::new();
    if idx == 0 {
        let mut rec = [0u8; 32];
        let mut l = [b' '; 11];
        let lb = label.as_bytes();
        l[..lb.len().min(11)].copy_from_slice(&lb[..lb.len().min(11)]);
        rec[0..11].copy_from_slice(&l);
        rec[11] = 0x08; // volume label
        out.extend_from_slice(&rec);
    }
    if idx != 0 {
        // "." and ".." in every subdirectory (FAT12/16 and FAT32). The root
        // directory has no dot entries in either layout: FAT12/16 keeps a
        // fixed region, and the FAT32 root is a plain cluster chain.
        out.extend_from_slice(&dot_entry(nodes[idx].first_cluster, true));
        let parent = nodes[idx].parent_first_cluster(nodes);
        out.extend_from_slice(&dot_entry(parent, false));
    }
    for &c in &nodes[idx].children {
        if nodes[c].needs_lfn {
            out.extend_from_slice(&lfn_entries(&nodes[c].name, &nodes[c].short));
        }
        let mut rec = [0u8; 32];
        rec[0..11].copy_from_slice(&nodes[c].short);
        rec[11] = if nodes[c].is_dir { 0x10 } else { 0x20 };
        rec[20..22].copy_from_slice(&((nodes[c].first_cluster >> 16) as u16).to_le_bytes());
        rec[26..28].copy_from_slice(&((nodes[c].first_cluster & 0xFFFF) as u16).to_le_bytes());
        rec[28..32].copy_from_slice(&(nodes[c].size as u32).to_le_bytes());
        out.extend_from_slice(&rec);
    }
    out.push(0);
    out.resize((out.len() + 31) & !31, 0);
    out
}

fn dot_entry(cluster: u32, is_dot: bool) -> [u8; 32] {
    let mut rec = [0u8; 32];
    // The name field is "." + 10 spaces, or ".." + 9 spaces (never NUL).
    rec[..11].fill(b' ');
    rec[0] = b'.';
    if !is_dot {
        rec[1] = b'.';
    }
    rec[11] = 0x10;
    rec[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    rec[26..28].copy_from_slice(&((cluster & 0xFFFF) as u16).to_le_bytes());
    rec
}

/// Serializes the VFAT long-name entries for `name`, preceding its 8.3 entry.
///
/// Per the FAT spec the entries are stored in reverse order: the LAST 13-char
/// slot (which carries the 0x40 `LAST_LONG_ENTRY` flag) comes first in the
/// directory, and the FIRST slot (order byte 0x01) sits immediately before the
/// short 8.3 entry.
fn lfn_entries(name: &str, short: &[u8; 11]) -> Vec<u8> {
    let utf16: Vec<u16> = name.encode_utf16().collect();
    let checksum = lfn_checksum(short);
    let n = utf16.len().div_ceil(13);
    let mut out = Vec::new();
    for idx in (0..n).rev() {
        let chunk = &utf16[idx * 13..(idx * 13 + 13).min(utf16.len())];
        let last = idx + 1 == n;
        let mut rec = [0u8; 32];
        rec[0] = (if last { 0x40 } else { 0 }) | (idx + 1) as u8;
        rec[11] = 0x0F;
        rec[13] = checksum;
        let slots: [(usize, usize); 3] = [(1, 5), (14, 6), (28, 2)];
        let mut c = 0usize;
        for (base, count) in slots {
            for slot in 0..count {
                if let Some(ch) = chunk.get(c) {
                    rec[base + slot * 2..base + slot * 2 + 2]
                        .copy_from_slice(&ch.to_le_bytes());
                }
                c += 1;
            }
        }
        out.extend_from_slice(&rec);
    }
    out
}

fn lfn_checksum(short: &[u8; 11]) -> u8 {
    let mut sum = 0u8;
    for &b in short {
        sum = sum.rotate_right(1).wrapping_add(b);
    }
    sum
}

/// Walks `dir`, appending nodes under `parent` (filtered by `only`).
fn build_tree(
    dir: &Path,
    nodes: &mut Vec<Node>,
    parent: usize,
    only: &[&str],
    used: &mut HashSet<String>,
) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !only.is_empty() && !only.contains(&name.as_str()) {
            continue;
        }
        let md = std::fs::metadata(&path).with_context(|| format!("stating {}", path.display()))?;
        let is_dir = md.is_dir();
        let short = make_short_name(&name, is_dir, used);
        let needs_lfn = needs_lfn(&name, &short);
        let idx = nodes.len();
        nodes.push(Node {
            name: name.clone(),
            parent,
            is_dir,
            size: if is_dir { 0 } else { md.len() },
            src: if is_dir { None } else { Some(path.clone()) },
            children: Vec::new(),
            first_cluster: 0,
            chain_len: 0,
            short,
            needs_lfn,
        });
        nodes[parent].children.push(idx);
        if is_dir {
            build_tree(&path, nodes, idx, &[], used)?;
        }
    }
    Ok(())
}

/// Whether a name needs VFAT long-name entries.
///
/// An 8.3 short name can only represent upper-case ASCII within its
/// 8+3 limits, using the legal 8.3 character set. Anything else (lowercase,
/// non-ASCII, overlong stem or extension) needs long-name entries, as does any
/// name whose derived short name no longer round-trips (e.g. after a `~n`
/// collision suffix).
fn needs_lfn(name: &str, short: &[u8; 11]) -> bool {
    if name == "." || name == ".." {
        return false;
    }
    let legal = |c: char| {
        c.is_ascii_alphanumeric() || matches!(c, '$' | '%' | '\'' | '-' | '_' | '~' | '!' | '#' | '@')
    };
    if name
        .chars()
        .any(|c| !c.is_ascii() || (c.is_lowercase() && c.is_ascii_alphabetic()))
    {
        return true;
    }
    let upper = name.to_uppercase();
    let (stem, ext) = match upper.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => (s, e),
        _ => (upper.as_str(), ""),
    };
    let ok_stem = !stem.is_empty() && stem.len() <= 8 && stem.chars().all(legal);
    let ok_ext = ext.is_empty() || (ext.len() <= 3 && ext.chars().all(legal));
    if ok_stem && ok_ext {
        // Round-trip: the short name must match the uppercased 8.3 form.
        let mut expect = [b' '; 11];
        for (i, ch) in stem.chars().enumerate().take(8) {
            expect[i] = ch as u8;
        }
        for (i, ch) in ext.chars().enumerate().take(3) {
            expect[8 + i] = ch as u8;
        }
        return expect != *short;
    }
    true
}

/// Builds an 11-byte 8.3 short name, deduplicating against `used`.
fn make_short_name(name: &str, is_dir: bool, used: &mut HashSet<String>) -> [u8; 11] {
    let clean = |s: &str, max: usize| -> String {
        s.to_uppercase()
            .chars()
            .filter(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(*c, '$' | '%' | '\'' | '-' | '_' | '~' | '!' | '#' | '@')
            })
            .take(max)
            .collect()
    };
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => (s, Some(e)),
        _ => (name, None),
    };
    let stem_base = clean(stem, 8);
    let ext_base = if is_dir {
        String::new()
    } else {
        clean(ext.unwrap_or(""), 3)
    };

    let mut n = 0u32;
    loop {
        let stem = if n == 0 {
            stem_base.clone()
        } else {
            let suffix = format!("~{}", n.min(99_999));
            let keep = 8usize.saturating_sub(suffix.len());
            let mut s = stem_base.chars().take(keep).collect::<String>();
            s.push_str(&suffix);
            s
        };
        let mut cand = String::new();
        cand.push_str(&stem);
        cand.push_str(&ext_base);
        if !used.contains(&cand) {
            used.insert(cand.clone());
            let mut f = [b' '; 11];
            for (i, ch) in stem.chars().enumerate().take(8) {
                f[i] = ch as u8;
            }
            for (i, ch) in ext_base.chars().enumerate().take(3) {
                f[8 + i] = ch as u8;
            }
            return f;
        }
        n += 1;
        if n > 99_999 {
            // Exhausted: emit a deterministic last-resort name.
            let cand = format!("LEON~{}", name.len());
            let mut f = [b' '; 11];
            for (i, ch) in cand.chars().enumerate().take(11) {
                f[i] = ch as u8;
            }
            return f;
        }
    }
}

fn write_boot_sector(boot: &mut [u8; 512], geo: &Geometry, label: &str) -> Result<()> {
    use std::io::Write;
    let mut w = std::io::Cursor::new(boot.as_mut_slice());
    let put = |w: &mut std::io::Cursor<&mut [u8]>, data: &[u8]| -> std::io::Result<()> {
        w.write_all(data)
    };
    put(&mut w, &[0xEB, 0x58, 0x90])?;
    put(&mut w, b"MSDOS5.0")?;
    w.write_all(&512u16.to_le_bytes())?;
    w.write_all(&[geo.spc as u8])?;
    w.write_all(&(geo.reserved as u16).to_le_bytes())?;
    w.write_all(&[geo.num_fats as u8])?;

    if geo.kind == FatKind::Fat32 {
        w.write_all(&0u16.to_le_bytes())?; // root entries
        w.write_all(&0u16.to_le_bytes())?; // total sectors 16
        w.write_all(&[0xF8])?; // media
        w.write_all(&0u16.to_le_bytes())?; // FAT size 16
        w.write_all(&32u16.to_le_bytes())?; // spt
        w.write_all(&64u16.to_le_bytes())?; // heads
        w.write_all(&0u32.to_le_bytes())?; // hidden
        w.write_all(&(geo.total_sectors as u32).to_le_bytes())?;
        w.write_all(&geo.fat_sectors.to_le_bytes())?;
        w.write_all(&0u16.to_le_bytes())?; // ext flags
        w.write_all(&0u16.to_le_bytes())?; // fs version
        w.write_all(&geo.root_cluster.to_le_bytes())?;
        w.write_all(&1u16.to_le_bytes())?; // FSInfo sector
        w.write_all(&6u16.to_le_bytes())?; // backup boot
        w.write_all(&[0u8; 12])?;
        w.write_all(&[0x80])?;
        w.write_all(&[0])?;
        w.write_all(&[0x29])?;
        w.write_all(&0x0BAD_F00Du32.to_le_bytes())?;
        let mut l = [b' '; 11];
        l[..label.len().min(11)].copy_from_slice(&label.as_bytes()[..label.len().min(11)]);
        w.write_all(&l)?;
        w.write_all(b"FAT32   ")?;
        w.write_all(&vec![0u8; 510 - 90])?;
        w.write_all(&[0x55, 0xAA])?;
    } else {
        w.write_all(&(geo.root_entries as u16).to_le_bytes())?;
        if geo.total_sectors < 65_536 {
            w.write_all(&(geo.total_sectors as u16).to_le_bytes())?;
        } else {
            w.write_all(&0u16.to_le_bytes())?;
        }
        w.write_all(&[0xF8])?;
        w.write_all(&(geo.fat_sectors as u16).to_le_bytes())?;
        w.write_all(&32u16.to_le_bytes())?;
        w.write_all(&64u16.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?; // hidden
        if geo.total_sectors >= 65_536 {
            w.write_all(&(geo.total_sectors as u32).to_le_bytes())?;
        } else {
            w.write_all(&0u32.to_le_bytes())?;
        }
        w.write_all(&[0x80])?;
        w.write_all(&[0])?;
        w.write_all(&[0x29])?;
        w.write_all(&0x0BAD_F00Du32.to_le_bytes())?;
        let mut l = [b' '; 11];
        l[..label.len().min(11)].copy_from_slice(&label.as_bytes()[..label.len().min(11)]);
        w.write_all(&l)?;
        w.write_all(geo.kind.fs_label().as_bytes())?;
        w.write_all(&vec![0u8; 510 - 62])?;
        w.write_all(&[0x55, 0xAA])?;
    }
    Ok(())
}

/// Packs the in-memory FAT (cluster -> next) into its on-disk byte layout.
fn pack_fat(fat: &[u32], kind: FatKind) -> Vec<u8> {
    match kind {
        FatKind::Fat12 => {
            let mut out = vec![0u8; fat.len() * 3 / 2 + 1];
            for (i, &v) in fat.iter().enumerate() {
                let v = v & 0xFFF;
                let base = i * 3 / 2;
                if i % 2 == 0 {
                    out[base] |= (v & 0xFF) as u8;
                    out[base + 1] |= ((v >> 8) & 0x0F) as u8;
                } else {
                    out[base] |= ((v & 0x0F) as u8) << 4;
                    out[base + 1] |= ((v >> 4) & 0xFF) as u8;
                }
            }
            out
        }
        FatKind::Fat16 => {
            let mut out = Vec::with_capacity(fat.len() * 2);
            for &v in fat {
                out.extend_from_slice(&(v as u16).to_le_bytes());
            }
            out
        }
        FatKind::Fat32 => {
            let mut out = Vec::with_capacity(fat.len() * 4);
            for &v in fat {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree(root: &Path) {
        std::fs::create_dir_all(root.join("EFI/BOOT")).unwrap();
        std::fs::create_dir_all(root.join("EFI/leon")).unwrap();
        std::fs::create_dir_all(root.join("boot")).unwrap();
        std::fs::write(root.join("EFI/BOOT/BOOTX64.EFI"), b"MZ...PE").unwrap();
        std::fs::write(root.join("EFI/leon/kernel.efi"), b"kernel").unwrap();
        std::fs::write(root.join("README.txt"), b"hello world").unwrap();
        std::fs::write(root.join("boot/empty.txt"), b"").unwrap();
    }

    #[test]
    fn geometry_picks_correct_fat_types() {
        let g32 = geometry(64 * 1024 * 1024 / 512).unwrap();
        assert_eq!(g32.kind, FatKind::Fat32);
        assert_eq!(g32.spc, 1);
        let g16 = geometry(16 * 1024 * 1024 / 512).unwrap();
        assert_eq!(g16.kind, FatKind::Fat16);
        let g12 = geometry(2 * 1024 * 1024 / 512).unwrap();
        assert_eq!(g12.kind, FatKind::Fat12);
    }

    #[test]
    fn fat32_volume_roundtrip() {
        let dir = std::env::temp_dir().join("lbt_fat32_src");
        let _ = std::fs::remove_dir_all(&dir);
        sample_tree(&dir);
        let img = std::env::temp_dir().join("lbt_fat32.img");
        let _ = std::fs::remove_file(&img);
        let size = 64 * 1024 * 1024u64;
        let mut f = File::options()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            .open(&img)
            .unwrap();
        f.set_len(size).unwrap();
        write_fat(
            &mut f,
            &FatOptions {
                label: "LEON".to_string(),
                size_bytes: size,
            },
            Some(&dir),
            &[],
        )
        .unwrap();
        drop(f);

        let buf = std::fs::read(&img).unwrap();
        assert_eq!(&buf[510..512], &[0x55, 0xAA]);
        assert_eq!(&buf[0..3], &[0xEB, 0x58, 0x90]);
        assert_eq!(buf[13], 1, "64 MiB FAT32 uses 1 sector/cluster");
        assert_eq!(&buf[82..90], b"FAT32   ");
        let root_cluster = u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]]);
        assert_eq!(root_cluster, 2);
        let fat_sectors = u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]);
        assert!(fat_sectors > 0);
        // FSInfo present.
        assert_eq!(&buf[0x200..0x204], b"RRaA");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&img);
    }

    #[test]
    fn fat32_layout_meets_the_spec() {
        // Regression test for the three standards fixes validated against
        // fsck.fat: the root directory really lives at cluster 2, it carries
        // the volume label and no dot entries, and every subdirectory's "."
        // / ".." entries use space-padded names with correct clusters.
        let dir = std::env::temp_dir().join("lbt_fat_spec_src");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("EFI/BOOT")).unwrap();
        std::fs::write(dir.join("EFI/BOOT/BOOTX64.EFI"), b"MZ").unwrap();
        let img = std::env::temp_dir().join("lbt_fat_spec.img");
        let _ = std::fs::remove_file(&img);
        let size = 64 * 1024 * 1024u64;
        let mut f = File::options()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            .open(&img)
            .unwrap();
        f.set_len(size).unwrap();
        write_fat(
            &mut f,
            &FatOptions {
                label: "LEON".to_string(),
                size_bytes: size,
            },
            Some(&dir),
            &[],
        )
        .unwrap();
        drop(f);

        let buf = std::fs::read(&img).unwrap();
        let bps = u16::from_le_bytes([buf[11], buf[12]]) as usize;
        let spc = buf[13] as usize;
        let reserved = u16::from_le_bytes([buf[14], buf[15]]) as usize;
        let fats = buf[16] as usize;
        let fat_sectors = u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]) as usize;
        let root = u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]]) as usize;
        assert_eq!(root, 2);
        let data_start = (reserved + fats * fat_sectors) * bps;
        let cluster = |c: usize| data_start + (c - 2) * spc * bps;
        let fat_at = |c: usize| -> u32 {
            let off = reserved * bps + c * 4;
            u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
                & 0x0FFF_FFFF
        };

        let read_dir = |c: usize| -> Vec<[u8; 32]> {
            let mut out = Vec::new();
            let mut cl = c;
            loop {
                let off = cluster(cl);
                for i in 0..spc * bps / 32 {
                    let mut e = [0u8; 32];
                    e.copy_from_slice(&buf[off + i * 32..off + i * 32 + 32]);
                    if e[0] == 0 || e[0] == 0xE5 {
                        continue;
                    }
                    out.push(e);
                }
                let next = fat_at(cl);
                if next >= 0x0FFF_FFF8 {
                    break;
                }
                cl = next as usize;
            }
            out
        };

        let root_entries = read_dir(root);
        // First entry is the volume label (attr 0x08), space-padded, with no
        // dot entries in the root.
        assert_eq!(root_entries[0][11], 0x08, "volume label first in root");
        assert_eq!(&root_entries[0][0..11], b"LEON       ");
        assert!(
            root_entries.iter().all(|e| e[0] != b'.' || e[1] != b' '),
            "root directory must not contain '.' entries: {root_entries:?}"
        );

        // The EFI subdirectory has space-padded "." and ".." entries whose
        // clusters point at itself and at the root (0 for a root child).
        let efi = root_entries
            .iter()
            .find(|e| e[11] == 0x10 && &e[0..11] == b"EFI        ")
            .unwrap();
        let efi_cluster = (u16::from_le_bytes([efi[20], efi[21]]) as u32) << 16
            | u16::from_le_bytes([efi[26], efi[27]]) as u32;
        let efi_entries = read_dir(efi_cluster as usize);
        assert_eq!(&efi_entries[0][0..11], b".          ", "dot padded with spaces");
        assert_eq!(&efi_entries[1][0..11], b"..         ", "dotdot padded with spaces");
        assert_eq!(efi_entries[0][11], 0x10);
        assert_eq!(efi_entries[1][11], 0x10);
        let first = |e: &[u8; 32]| {
            (u16::from_le_bytes([e[20], e[21]]) as u32) << 16
                | u16::from_le_bytes([e[26], e[27]]) as u32
        };
        assert_eq!(first(&efi_entries[0]), efi_cluster, "'.' points to itself");
        assert_eq!(first(&efi_entries[1]), 0, "'..' of a root child points to 0");
        // The bootloader file lives in EFI/BOOT with a valid short entry.
        let boot_dir = efi_entries
            .iter()
            .find(|e| e[11] == 0x10 && &e[0..11] == b"BOOT       ")
            .unwrap();
        let boot_cluster = first(boot_dir);
        let boot_entries = read_dir(boot_cluster as usize);
        let _boot = boot_entries
            .iter()
            .find(|e| e[11] == 0x20 && &e[0..11] == b"BOOTX64 EFI")
            .unwrap();
        assert!(fat_at(boot_cluster as usize) >= 0x0FFF_FFF8, "dir chain ends");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&img);
    }

    #[test]
    fn short_names_collide_safely() {
        let mut used = HashSet::new();
        let a = make_short_name("LongFileName.TXT", false, &mut used);
        let b = make_short_name("LongFileNam2.TXT", false, &mut used);
        assert_ne!(a, b);
        assert_eq!(&a[8..11], b"TXT");
        assert_eq!(&b[8..11], b"TXT");
    }

    #[test]
    fn plain_8dot3_needs_no_lfn() {
        let mut used = HashSet::new();
        let short = make_short_name("README.TXT", false, &mut used);
        assert_eq!(&short[..11], b"README  TXT");
        assert!(!needs_lfn("README.TXT", &short));
        assert!(needs_lfn("long-name.txt", &short));
    }

    #[test]
    fn pack_fat_lengths() {
        let fat = vec![0u32; 1000];
        assert_eq!(pack_fat(&fat, FatKind::Fat12).len(), 1500 + 1);
        assert_eq!(pack_fat(&fat, FatKind::Fat16).len(), 2000);
        assert_eq!(pack_fat(&fat, FatKind::Fat32).len(), 4000);
    }
}
