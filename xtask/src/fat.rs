//! FAT32 file system writer (per Microsoft's FAT specification, fatgen103).
//!
//! Builds a complete, freshly formatted volume in memory containing a given
//! set of files, with long file names where needed. Files and directories
//! are laid out contiguously; there is no fragmentation to deal with.

use std::collections::BTreeMap;

pub const SECTOR: usize = 512;
const RESERVED_SECTORS: u32 = 32;
const NUM_FATS: u32 = 2;
const ROOT_CLUSTER: u32 = 2;
const FSINFO_SECTOR: u16 = 1;
const BACKUP_BOOT_SECTOR: u16 = 6;
/// FAT32 needs at least this many clusters (fatgen103, "FAT type
/// determination"); stay clear of the boundary as the spec recommends.
const MIN_CLUSTERS: u32 = 65525 + 16;
const END_OF_CHAIN: u32 = 0x0fff_ffff;
const MEDIA: u8 = 0xf8;
const LABEL: &[u8; 11] = b"SEGOS      ";

const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LONG_NAME: u8 = 0x0f;

/// 2025-01-01 00:00:00: fixed, so images are reproducible.
const DATE: u16 = ((2025 - 1980) << 9) | (1 << 5) | 1;
const TIME: u16 = 0;

enum Node {
    File(Vec<u8>),
    Dir(BTreeMap<String, Node>),
}

/// One directory entry as it will be written: the 8.3 name, optional long
/// name and the node it refers to.
struct Entry<'a> {
    short: [u8; 11],
    long: Option<&'a str>,
    node: &'a Node,
}

struct Layout {
    sectors_per_cluster: u32,
    fat_sectors: u32,
    clusters: u32,
}

fn layout(total_sectors: u32) -> Result<Layout, String> {
    // Largest cluster size (up to 4 KiB) that still leaves enough clusters.
    for spc in [8u32, 4, 2, 1] {
        // Solve for the FAT size: every cluster plus the two reserved
        // entries needs 4 bytes in each FAT.
        let mut fat_sectors = 1;
        loop {
            let data = total_sectors - RESERVED_SECTORS - NUM_FATS * fat_sectors;
            let clusters = data / spc;
            if (clusters as u64 + 2) * 4 <= fat_sectors as u64 * SECTOR as u64 {
                if clusters >= MIN_CLUSTERS {
                    return Ok(Layout {
                        sectors_per_cluster: spc,
                        fat_sectors,
                        clusters,
                    });
                }
                break;
            }
            fat_sectors += 1;
        }
    }
    Err(format!(
        "{total_sectors} sectors is too small for FAT32 (need about 34 MiB)"
    ))
}

/// Valid characters of an 8.3 short name (upper case only).
fn short_char_ok(c: u8) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || b"!#$%&'()-@^_`{}~".contains(&c)
}

/// The 11-byte short name if `name` is already a valid upper-case 8.3 name.
fn exact_short_name(name: &str) -> Option<[u8; 11]> {
    let (base, ext) = name.split_once('.').unwrap_or((name, ""));
    if base.is_empty()
        || base.len() > 8
        || ext.len() > 3
        || !base.bytes().chain(ext.bytes()).all(short_char_ok)
    {
        return None;
    }
    let mut s = [b' '; 11];
    s[..base.len()].copy_from_slice(base.as_bytes());
    s[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
    Some(s)
}

/// A unique `BASIS~N.EXT` short name for a long name.
fn generated_short_name(name: &str, used: &[[u8; 11]]) -> [u8; 11] {
    let clean = |s: &str| -> Vec<u8> {
        s.bytes()
            .filter(|&c| c != b' ' && c != b'.')
            .map(|c| {
                let c = c.to_ascii_uppercase();
                if short_char_ok(c) { c } else { b'_' }
            })
            .collect()
    };
    let (base, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (clean(&name[..i]), clean(&name[i + 1..])),
        _ => (clean(name), Vec::new()),
    };
    for n in 1u32.. {
        let tail = format!("~{n}");
        let keep = base.len().min(8 - tail.len());
        let mut s = [b' '; 11];
        s[..keep].copy_from_slice(&base[..keep]);
        s[keep..keep + tail.len()].copy_from_slice(tail.as_bytes());
        let e = ext.len().min(3);
        s[8..8 + e].copy_from_slice(&ext[..e]);
        if !used.contains(&s) {
            return s;
        }
    }
    unreachable!()
}

fn lfn_checksum(short: &[u8; 11]) -> u8 {
    short
        .iter()
        .fold(0u8, |sum, &c| (sum >> 1 | sum << 7).wrapping_add(c))
}

fn dir_entries(children: &BTreeMap<String, Node>) -> Vec<Entry<'_>> {
    let mut used = Vec::new();
    let mut out = Vec::new();
    // Exact 8.3 names first so generated names cannot take them.
    for (name, node) in children {
        if let Some(s) = exact_short_name(name) {
            used.push(s);
            out.push(Entry {
                short: s,
                long: None,
                node,
            });
        }
    }
    for (name, node) in children {
        if exact_short_name(name).is_none() {
            let s = generated_short_name(name, &used);
            used.push(s);
            out.push(Entry {
                short: s,
                long: Some(name),
                node,
            });
        }
    }
    out
}

/// Number of 32-byte slots an entry takes (long name entries + short entry).
fn slots(e: &Entry) -> usize {
    1 + e.long.map_or(0, |l| l.encode_utf16().count().div_ceil(13))
}

struct Writer<'a> {
    img: &'a mut [u8],
    layout: Layout,
    data_start: u32,
    next_cluster: u32,
}

impl Writer<'_> {
    fn cluster_bytes(&self) -> usize {
        self.layout.sectors_per_cluster as usize * SECTOR
    }

    fn set_fat(&mut self, cluster: u32, value: u32) {
        for f in 0..NUM_FATS {
            let off = (RESERVED_SECTORS + f * self.layout.fat_sectors) as usize * SECTOR
                + cluster as usize * 4;
            self.img[off..off + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    /// Allocates a contiguous chain for `len` bytes (at least one cluster)
    /// and writes `data` into it. Returns the first cluster.
    fn store(&mut self, data: &[u8]) -> Result<u32, String> {
        let n = data.len().div_ceil(self.cluster_bytes()).max(1) as u32;
        let first = self.next_cluster;
        if first + n > self.layout.clusters + 2 {
            return Err("files do not fit in the EFI system partition".into());
        }
        for c in first..first + n {
            let next = if c + 1 == first + n { END_OF_CHAIN } else { c + 1 };
            self.set_fat(c, next);
        }
        self.next_cluster += n;
        let off = (self.data_start as usize
            + (first - 2) as usize * self.layout.sectors_per_cluster as usize)
            * SECTOR;
        self.img[off..off + data.len()].copy_from_slice(data);
        Ok(first)
    }

    /// Writes a directory (and, recursively, its contents) whose first
    /// cluster is `own` (already reserved) and whose parent is `parent`
    /// (0 for the root).
    fn write_dir(
        &mut self,
        children: &BTreeMap<String, Node>,
        own: u32,
        parent: u32,
        is_root: bool,
    ) -> Result<(), String> {
        let entries = dir_entries(children);
        // Reserve every child's clusters first: a directory's own clusters
        // must be known before the parent entry pointing to it is written.
        let mut first_clusters = Vec::new();
        for e in &entries {
            first_clusters.push(match e.node {
                Node::File(data) if data.is_empty() => 0,
                Node::File(data) => self.store(data)?,
                Node::Dir(sub) => {
                    let bytes = (dir_entries(sub).iter().map(slots).sum::<usize>() + 2) * 32;
                    self.store(&vec![0; bytes])?
                }
            });
        }
        let mut buf = Vec::new();
        if is_root {
            short_entry(&mut buf, LABEL, ATTR_VOLUME_ID, 0, 0);
        } else {
            short_entry(&mut buf, b".          ", ATTR_DIRECTORY, own, 0);
            short_entry(&mut buf, b"..         ", ATTR_DIRECTORY, parent, 0);
        }
        for (e, &cluster) in entries.iter().zip(&first_clusters) {
            if let Some(long) = e.long {
                long_entries(&mut buf, long, lfn_checksum(&e.short));
            }
            match e.node {
                Node::File(data) => {
                    let size = u32::try_from(data.len()).map_err(|_| "file larger than 4 GiB")?;
                    short_entry(&mut buf, &e.short, ATTR_ARCHIVE, cluster, size);
                }
                Node::Dir(_) => short_entry(&mut buf, &e.short, ATTR_DIRECTORY, cluster, 0),
            }
        }
        let off = (self.data_start as usize
            + (own - 2) as usize * self.layout.sectors_per_cluster as usize)
            * SECTOR;
        self.img[off..off + buf.len()].copy_from_slice(&buf);
        for (e, &cluster) in entries.iter().zip(&first_clusters) {
            if let Node::Dir(sub) = e.node {
                // ".." of a directory in the root is 0, by convention.
                let p = if is_root { 0 } else { own };
                self.write_dir(sub, cluster, p, false)?;
            }
        }
        Ok(())
    }
}

fn short_entry(buf: &mut Vec<u8>, name: &[u8; 11], attr: u8, cluster: u32, size: u32) {
    let mut e = [0u8; 32];
    e[0..11].copy_from_slice(name);
    e[11] = attr;
    if attr != ATTR_VOLUME_ID {
        e[14..16].copy_from_slice(&TIME.to_le_bytes()); // creation time
        e[16..18].copy_from_slice(&DATE.to_le_bytes()); // creation date
        e[18..20].copy_from_slice(&DATE.to_le_bytes()); // last access date
    }
    e[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    e[22..24].copy_from_slice(&TIME.to_le_bytes()); // write time
    e[24..26].copy_from_slice(&DATE.to_le_bytes()); // write date
    e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    e[28..32].copy_from_slice(&size.to_le_bytes());
    buf.extend_from_slice(&e);
}

/// Long-name entries for `name`, last part first, as they precede the
/// short entry on disk.
fn long_entries(buf: &mut Vec<u8>, name: &str, checksum: u8) {
    let mut chars: Vec<u16> = name.encode_utf16().collect();
    let parts = chars.len().div_ceil(13);
    if chars.len() % 13 != 0 {
        chars.push(0);
    }
    chars.resize(parts * 13, 0xffff);
    for part in (0..parts).rev() {
        let c = &chars[part * 13..part * 13 + 13];
        let mut e = [0u8; 32];
        e[0] = (part + 1) as u8 | if part + 1 == parts { 0x40 } else { 0 };
        e[11] = ATTR_LONG_NAME;
        e[13] = checksum;
        let offsets = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        for (ch, off) in c.iter().zip(offsets) {
            e[off..off + 2].copy_from_slice(&ch.to_le_bytes());
        }
        buf.extend_from_slice(&e);
    }
}

/// Formats `img` (the whole partition, a multiple of 512 bytes) as FAT32
/// holding `files` (paths like "EFI/BOOT/BOOTX64.EFI"). `hidden_sectors` is
/// the partition's first LBA on the disk; `volume_id` the serial number.
pub fn format(
    img: &mut [u8],
    hidden_sectors: u32,
    volume_id: u32,
    files: &[(&str, Vec<u8>)],
) -> Result<(), String> {
    let total = u32::try_from(img.len() / SECTOR).map_err(|_| "partition too large")?;
    let layout = layout(total)?;

    let mut root = BTreeMap::new();
    for (path, data) in files {
        let mut dir = &mut root;
        let parts: Vec<&str> = path.split('/').collect();
        for p in &parts[..parts.len() - 1] {
            let n = dir
                .entry(p.to_string())
                .or_insert_with(|| Node::Dir(BTreeMap::new()));
            dir = match n {
                Node::Dir(d) => d,
                Node::File(_) => return Err(format!("{p} is both a file and a directory")),
            };
        }
        dir.insert(parts[parts.len() - 1].to_string(), Node::File(data.clone()));
    }

    // Boot sector (BPB).
    let mut bs = [0u8; SECTOR];
    bs[0..3].copy_from_slice(&[0xeb, 0x58, 0x90]);
    bs[3..11].copy_from_slice(b"SEGOS   ");
    bs[11..13].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    bs[13] = layout.sectors_per_cluster as u8;
    bs[14..16].copy_from_slice(&(RESERVED_SECTORS as u16).to_le_bytes());
    bs[16] = NUM_FATS as u8;
    // Root entry count, 16-bit total sectors and 16-bit FAT size stay 0.
    bs[21] = MEDIA;
    bs[24..26].copy_from_slice(&63u16.to_le_bytes()); // sectors per track
    bs[26..28].copy_from_slice(&255u16.to_le_bytes()); // heads
    bs[28..32].copy_from_slice(&hidden_sectors.to_le_bytes());
    bs[32..36].copy_from_slice(&total.to_le_bytes());
    bs[36..40].copy_from_slice(&layout.fat_sectors.to_le_bytes());
    // Flags 0 (FATs mirrored), version 0.0.
    bs[44..48].copy_from_slice(&ROOT_CLUSTER.to_le_bytes());
    bs[48..50].copy_from_slice(&FSINFO_SECTOR.to_le_bytes());
    bs[50..52].copy_from_slice(&BACKUP_BOOT_SECTOR.to_le_bytes());
    bs[64] = 0x80; // drive number
    bs[66] = 0x29; // extended boot signature
    bs[67..71].copy_from_slice(&volume_id.to_le_bytes());
    bs[71..82].copy_from_slice(LABEL);
    bs[82..90].copy_from_slice(b"FAT32   ");
    // Not bootable as a legacy volume: "int 0x18" (try next boot device).
    bs[90..92].copy_from_slice(&[0xcd, 0x18]);
    bs[510] = 0x55;
    bs[511] = 0xaa;

    let data_start = RESERVED_SECTORS + NUM_FATS * layout.fat_sectors;
    let clusters = layout.clusters;
    let mut w = Writer {
        img,
        layout,
        data_start,
        next_cluster: ROOT_CLUSTER,
    };
    w.set_fat(0, 0x0fff_ff00 | MEDIA as u32);
    w.set_fat(1, END_OF_CHAIN);
    let root_bytes = (dir_entries(&root).iter().map(slots).sum::<usize>() + 1) * 32;
    let root_cluster = w.store(&vec![0; root_bytes])?;
    debug_assert_eq!(root_cluster, ROOT_CLUSTER);
    w.write_dir(&root, root_cluster, 0, true)?;
    let next_free = w.next_cluster;
    let free = clusters + 2 - next_free;

    // FSInfo sector.
    let mut fsinfo = [0u8; SECTOR];
    fsinfo[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes());
    fsinfo[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
    fsinfo[488..492].copy_from_slice(&free.to_le_bytes());
    fsinfo[492..496].copy_from_slice(&next_free.to_le_bytes());
    fsinfo[508..512].copy_from_slice(&0xaa55_0000u32.to_le_bytes());

    // Boot sector, FSInfo and the third boot sector, plus their backups.
    for base in [0, BACKUP_BOOT_SECTOR as usize] {
        let at = |s: usize| (base + s) * SECTOR;
        w.img[at(0)..at(1)].copy_from_slice(&bs);
        w.img[at(1)..at(2)].copy_from_slice(&fsinfo);
        w.img[at(2) + 510] = 0x55;
        w.img[at(2) + 511] = 0xaa;
    }
    Ok(())
}
