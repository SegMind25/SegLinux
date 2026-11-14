//! Just enough ACPI to find the CPUs: RSDP → XSDT (or RSDT) → MADT.

use crate::mm::phys_to_virt;
use alloc::vec::Vec;

fn read<T: Copy>(phys: u64) -> T {
    unsafe { phys_to_virt(phys).cast::<T>().read_unaligned() }
}

fn checksum_ok(phys: u64, len: usize) -> bool {
    let bytes = unsafe { core::slice::from_raw_parts(phys_to_virt(phys), len) };
    bytes.iter().fold(0u8, |a, &b| a.wrapping_add(b)) == 0
}

#[derive(Clone, Copy)]
#[repr(C, packed)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

/// Physical addresses of every table the root table lists.
fn tables(rsdp: u64) -> Vec<u64> {
    let mut out = Vec::new();
    if rsdp == 0 || read::<[u8; 8]>(rsdp) != *b"RSD PTR " || !checksum_ok(rsdp, 20) {
        crate::warn!("acpi: no valid RSDP");
        return out;
    }
    let revision: u8 = read(rsdp + 15);
    let (root, entry_size) = if revision >= 2 && read::<u64>(rsdp + 24) != 0 {
        (read::<u64>(rsdp + 24), 8)
    } else {
        (read::<u32>(rsdp + 16) as u64, 4)
    };
    let header: SdtHeader = read(root);
    if !checksum_ok(root, header.length as usize) {
        crate::warn!("acpi: root table checksum is wrong");
        return out;
    }
    let count = (header.length as usize - size_of::<SdtHeader>()) / entry_size;
    for i in 0..count {
        let at = root + size_of::<SdtHeader>() as u64 + (i * entry_size) as u64;
        out.push(if entry_size == 8 {
            read::<u64>(at)
        } else {
            read::<u32>(at) as u64
        });
    }
    out
}

fn find(rsdp: u64, signature: &[u8; 4]) -> Option<u64> {
    tables(rsdp).into_iter().find(|&t| {
        let h: SdtHeader = read(t);
        h.signature == *signature && checksum_ok(t, h.length as usize)
    })
}

/// Local APIC IDs of the usable processors, from the MADT.
pub fn processor_apic_ids(rsdp: u64) -> Vec<u32> {
    let mut ids = Vec::new();
    let Some(madt) = find(rsdp, b"APIC") else {
        crate::warn!("acpi: no MADT");
        return ids;
    };
    let len = read::<SdtHeader>(madt).length as u64;
    let mut at = madt + size_of::<SdtHeader>() as u64 + 8; // local APIC address, flags
    while at + 2 <= madt + len {
        let (kind, entry_len): (u8, u8) = (read(at), read(at + 1));
        if entry_len < 2 {
            break;
        }
        match kind {
            // Processor local APIC: enabled, or can be brought online.
            0 if read::<u32>(at + 4) & 0b11 != 0 => ids.push(read::<u8>(at + 3) as u32),
            // Processor local x2APIC.
            9 if read::<u32>(at + 8) & 0b11 != 0 => ids.push(read::<u32>(at + 4)),
            _ => {}
        }
        at += entry_len as u64;
    }
    ids.dedup();
    ids
}
