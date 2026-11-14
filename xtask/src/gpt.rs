//! GUID partition table writer (UEFI specification, chapter 5): protective
//! MBR, primary and backup GPT headers and partition entry arrays.

pub const SECTOR: u64 = 512;
const ENTRIES: u32 = 128;
const ENTRY_SIZE: u32 = 128;
/// Sectors taken by the partition entry array (128 * 128 / 512).
const ARRAY_SECTORS: u64 = (ENTRIES * ENTRY_SIZE) as u64 / SECTOR;

/// EFI System Partition type GUID C12A7328-F81F-11D2-BA4B-00A0C93EC93B.
pub const ESP_TYPE: [u8; 16] = guid(0xc12a7328, 0xf81f, 0x11d2, [0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b]);

/// A GUID in its on-disk (mixed-endian) byte order.
const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

/// A random (version 4, RFC 4122 variant) GUID from 16 random bytes.
pub fn random_guid(mut r: [u8; 16]) -> [u8; 16] {
    r[7] = (r[7] & 0x0f) | 0x40; // version in the high nibble of field c
    r[8] = (r[8] & 0x3f) | 0x80; // variant
    r
}

/// CRC-32 (IEEE 802.3, reflected polynomial 0xEDB88320), as GPT uses.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

pub struct Partition<'a> {
    pub type_guid: [u8; 16],
    pub guid: [u8; 16],
    pub first_lba: u64,
    /// Inclusive.
    pub last_lba: u64,
    pub name: &'a str,
}

/// First LBA available to partitions.
pub const fn first_usable() -> u64 {
    2 + ARRAY_SECTORS
}

/// Last LBA available to partitions on a disk of `sectors` sectors.
pub const fn last_usable(sectors: u64) -> u64 {
    sectors - 2 - ARRAY_SECTORS
}

/// Writes the protective MBR and both GPTs into `disk` (the whole image).
pub fn write(disk: &mut [u8], disk_guid: [u8; 16], parts: &[Partition]) -> Result<(), String> {
    let sectors = disk.len() as u64 / SECTOR;
    let last = sectors - 1;
    if parts.len() > ENTRIES as usize {
        return Err("too many partitions".into());
    }
    for p in parts {
        if p.first_lba < first_usable() || p.last_lba > last_usable(sectors) || p.last_lba < p.first_lba {
            return Err(format!("partition {} outside the usable area", p.name));
        }
    }

    // Protective MBR: one partition of type 0xEE covering the disk.
    let mbr = &mut disk[..SECTOR as usize];
    let e = &mut mbr[446..462];
    e[0] = 0x00; // not active
    e[1..4].copy_from_slice(&[0x00, 0x02, 0x00]); // CHS of LBA 1
    e[4] = 0xee;
    e[5..8].copy_from_slice(&[0xff, 0xff, 0xff]); // CHS: beyond addressable
    e[8..12].copy_from_slice(&1u32.to_le_bytes());
    e[12..16].copy_from_slice(&((sectors - 1).min(u32::MAX as u64) as u32).to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xaa;

    // Partition entry array (identical copies before and after the data).
    let mut array = vec![0u8; (ENTRIES * ENTRY_SIZE) as usize];
    for (i, p) in parts.iter().enumerate() {
        let e = &mut array[i * ENTRY_SIZE as usize..(i + 1) * ENTRY_SIZE as usize];
        e[0..16].copy_from_slice(&p.type_guid);
        e[16..32].copy_from_slice(&p.guid);
        e[32..40].copy_from_slice(&p.first_lba.to_le_bytes());
        e[40..48].copy_from_slice(&p.last_lba.to_le_bytes());
        // Attributes (48..56) stay 0.
        for (j, c) in p.name.encode_utf16().take(36).enumerate() {
            e[56 + j * 2..58 + j * 2].copy_from_slice(&c.to_le_bytes());
        }
    }
    let array_crc = crc32(&array);

    let header = |my_lba: u64, alternate: u64, array_lba: u64| {
        let mut h = [0u8; 92];
        h[0..8].copy_from_slice(b"EFI PART");
        h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes()); // revision 1.0
        h[12..16].copy_from_slice(&92u32.to_le_bytes());
        // CRC (16..20) computed below with the field zeroed; 20..24 reserved.
        h[24..32].copy_from_slice(&my_lba.to_le_bytes());
        h[32..40].copy_from_slice(&alternate.to_le_bytes());
        h[40..48].copy_from_slice(&first_usable().to_le_bytes());
        h[48..56].copy_from_slice(&last_usable(sectors).to_le_bytes());
        h[56..72].copy_from_slice(&disk_guid);
        h[72..80].copy_from_slice(&array_lba.to_le_bytes());
        h[80..84].copy_from_slice(&ENTRIES.to_le_bytes());
        h[84..88].copy_from_slice(&ENTRY_SIZE.to_le_bytes());
        h[88..92].copy_from_slice(&array_crc.to_le_bytes());
        let crc = crc32(&h);
        h[16..20].copy_from_slice(&crc.to_le_bytes());
        h
    };
    let at = |lba: u64| (lba * SECTOR) as usize;
    let backup_array = last - ARRAY_SECTORS;
    disk[at(1)..at(1) + 92].copy_from_slice(&header(1, last, 2));
    disk[at(2)..at(2) + array.len()].copy_from_slice(&array);
    disk[at(backup_array)..at(backup_array) + array.len()].copy_from_slice(&array);
    disk[at(last)..at(last) + 92].copy_from_slice(&header(last, 1, backup_array));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}
