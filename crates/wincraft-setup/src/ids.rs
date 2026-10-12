//! Identifiers Setup chooses up front: filesystem UUIDs and the computer
//! name. Choosing them before formatting lets the whole plan, including
//! `/etc/fstab`, be computed and shown before anything is written.

use std::fs::File;
use std::io::Read;

pub trait IdSource {
    /// Random (version 4) UUID.
    fn uuid(&mut self) -> String;
    /// 32-bit FAT volume ID as 8 hex digits (`mkfs.fat -i`).
    fn fat_volume_id(&mut self) -> String;
    /// Windows-style computer name: `DESKTOP-` plus 7 characters.
    fn computer_name(&mut self) -> String;
}

/// Randomness from the kernel.
pub struct Random;

impl Random {
    fn bytes<const N: usize>() -> [u8; N] {
        let mut buf = [0u8; N];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut buf))
            .expect("/dev/urandom is readable");
        buf
    }
}

pub fn format_uuid(mut b: [u8; 16]) -> String {
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

const NAME_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

impl IdSource for Random {
    fn uuid(&mut self) -> String {
        format_uuid(Self::bytes::<16>())
    }

    fn fat_volume_id(&mut self) -> String {
        Self::bytes::<4>()
            .iter()
            .map(|x| format!("{x:02X}"))
            .collect()
    }

    fn computer_name(&mut self) -> String {
        let suffix: String = Self::bytes::<7>()
            .iter()
            .map(|b| NAME_CHARS[usize::from(*b) % NAME_CHARS.len()] as char)
            .collect();
        format!("DESKTOP-{suffix}")
    }
}

/// Predictable identifiers for tests and reproducible dry runs.
#[derive(Default)]
pub struct Sequential {
    next: u32,
}

impl Sequential {
    fn bump(&mut self) -> u32 {
        self.next += 1;
        self.next
    }
}

impl IdSource for Sequential {
    fn uuid(&mut self) -> String {
        format!("00000000-0000-4000-8000-{:012x}", self.bump())
    }

    fn fat_volume_id(&mut self) -> String {
        format!("{:08X}", self.bump())
    }

    fn computer_name(&mut self) -> String {
        format!("DESKTOP-{:07}", self.bump())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_shape() {
        let u = Random.uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(Random.uuid(), u);
        assert_eq!(format_uuid([0; 16]), "00000000-0000-4000-8000-000000000000");
    }

    #[test]
    fn computer_names() {
        let name = Random.computer_name();
        assert_eq!(name.len(), 15);
        assert!(crate::answer::valid_hostname(&name));
        assert_eq!(Random.fat_volume_id().len(), 8);
    }
}
