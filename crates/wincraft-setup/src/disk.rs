//! Disks, the partition layout and `/etc/fstab`.
//!
//! Windows Setup's default UEFI layout is ESP + MSR + Windows + Recovery.
//! Wincraft uses the Linux counterpart, on GPT, bootable from both UEFI
//! and legacy BIOS:
//!
//! | # | Partition            | Windows counterpart          |
//! |---|----------------------|------------------------------|
//! | 1 | EFI system partition | EFI system partition         |
//! | 2 | BIOS boot (1 MiB)    | System Reserved (BIOS/MBR)   |
//! | 3 | swap (optional)      | pagefile.sys                 |
//! | 4 | root                 | Windows (C:)                 |
//!
//! The install media itself plays the role of the Recovery partition.

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::answer::{Filesystem, MIB};

/// GPT partition type GUIDs.
pub const GUID_ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
pub const GUID_BIOS_BOOT: &str = "21686148-6449-6E6F-744E-656564454649";
pub const GUID_SWAP: &str = "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F";
/// "Root partition (x86-64)" from the Discoverable Partitions Specification.
pub const GUID_ROOT_X86_64: &str = "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709";

/// Smallest disk Setup accepts.
pub const MIN_DISK_SIZE: u64 = 20 * 1024 * MIB;
/// Smallest root partition left after ESP and swap.
pub const MIN_ROOT_SIZE: u64 = 12 * 1024 * MIB;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Disk {
    pub path: String,
    pub size: u64,
    pub model: String,
    pub removable: bool,
    pub read_only: bool,
    pub transport: String,
}

impl Disk {
    pub fn label(&self) -> String {
        let model = if self.model.is_empty() {
            "Disk"
        } else {
            self.model.as_str()
        };
        let mut s = format!("{}  {}  {}", self.path, human_size(self.size), model);
        if self.removable || self.transport == "usb" {
            s.push_str("  (removable)");
        }
        s
    }
}

/// Parse `lsblk -J -b -d -o PATH,SIZE,TYPE,MODEL,RM,RO,TRAN`. Older
/// util-linux versions print numbers and booleans as strings; both forms
/// are accepted.
pub fn parse_lsblk(json: &str) -> Result<Vec<Disk>> {
    let v: Value = serde_json::from_str(json).context("lsblk printed invalid JSON")?;
    let Some(devices) = v.get("blockdevices").and_then(Value::as_array) else {
        bail!("lsblk output has no blockdevices");
    };
    let text = |d: &Value, k: &str| {
        d.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let flag = |d: &Value, k: &str| match d.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "1" || s == "true",
        Some(Value::Number(n)) => n.as_u64() == Some(1),
        _ => false,
    };
    let mut disks = Vec::new();
    for d in devices {
        let path = text(d, "path");
        if text(d, "type") != "disk" || path.is_empty() {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or("");
        if name.starts_with("zram") || name.starts_with("loop") || name.starts_with("ram") {
            continue;
        }
        let size = match d.get("size") {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
            Some(Value::String(s)) => s.parse().unwrap_or(0),
            _ => 0,
        };
        disks.push(Disk {
            path,
            size,
            model: text(d, "model"),
            removable: flag(d, "rm"),
            read_only: flag(d, "ro"),
            transport: text(d, "tran"),
        });
    }
    Ok(disks)
}

/// Device node of partition `n`: `/dev/sda` -> `/dev/sda1`,
/// `/dev/nvme0n1` -> `/dev/nvme0n1p1`, `/dev/mmcblk0` -> `/dev/mmcblk0p1`.
pub fn partition_path(disk: &str, n: u32) -> String {
    if disk.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{disk}p{n}")
    } else {
        format!("{disk}{n}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Esp,
    BiosBoot,
    Swap,
    Root,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub number: u32,
    pub role: Role,
    pub path: String,
    /// `None` = rest of the disk.
    pub size: Option<u64>,
    /// Filesystem UUID chosen by Setup (FAT volume ID for the ESP), so
    /// fstab can be written before the filesystems exist.
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub disk: String,
    pub filesystem: Filesystem,
    pub partitions: Vec<Partition>,
}

impl Layout {
    pub fn new(
        disk: &str,
        filesystem: Filesystem,
        esp_size: u64,
        swap: Option<u64>,
        ids: &mut dyn crate::ids::IdSource,
    ) -> Layout {
        let mut partitions = vec![
            Partition {
                number: 1,
                role: Role::Esp,
                path: partition_path(disk, 1),
                size: Some(esp_size),
                uuid: Some(ids.fat_volume_id()),
            },
            Partition {
                number: 2,
                role: Role::BiosBoot,
                path: partition_path(disk, 2),
                size: Some(MIB),
                uuid: None,
            },
        ];
        if let Some(size) = swap {
            partitions.push(Partition {
                number: 3,
                role: Role::Swap,
                path: partition_path(disk, 3),
                size: Some(size),
                uuid: Some(ids.uuid()),
            });
        }
        let n = partitions.len() as u32 + 1;
        partitions.push(Partition {
            number: n,
            role: Role::Root,
            path: partition_path(disk, n),
            size: None,
            uuid: Some(ids.uuid()),
        });
        Layout {
            disk: disk.to_string(),
            filesystem,
            partitions,
        }
    }

    pub fn get(&self, role: Role) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.role == role)
    }

    pub fn root(&self) -> &Partition {
        self.get(Role::Root)
            .expect("layout always has a root partition")
    }

    pub fn esp(&self) -> &Partition {
        self.get(Role::Esp).expect("layout always has an ESP")
    }

    /// Bytes used by fixed-size partitions (plus 2 MiB alignment slack).
    pub fn fixed_size(&self) -> u64 {
        self.partitions.iter().filter_map(|p| p.size).sum::<u64>() + 2 * MIB
    }

    /// Input for `sfdisk`.
    pub fn sfdisk_script(&self) -> String {
        let mut s = String::from("label: gpt\n");
        for p in &self.partitions {
            let (guid, name) = match p.role {
                Role::Esp => (GUID_ESP, "EFI system partition"),
                Role::BiosBoot => (GUID_BIOS_BOOT, "BIOS boot partition"),
                Role::Swap => (GUID_SWAP, "Swap"),
                Role::Root => (GUID_ROOT_X86_64, "Wincraft"),
            };
            if let Some(size) = p.size {
                s.push_str(&format!("size={}MiB, ", size.div_ceil(MIB)));
            }
            s.push_str(&format!("type={guid}, name=\"{name}\"\n"));
        }
        s
    }

    /// Mount options for the root filesystem.
    pub fn root_options(&self) -> &'static str {
        match self.filesystem {
            Filesystem::Btrfs => "rw,noatime,compress=zstd:1,subvol=/@",
            _ => "rw,relatime",
        }
    }

    pub fn fstab(&self) -> String {
        let mut out = String::from(
            "# /etc/fstab: static file system information, written by Wincraft Setup.\n\
             # <file system>  <dir>  <type>  <options>  <dump>  <pass>\n",
        );
        let root = self.root();
        let root_uuid = root.uuid.as_deref().unwrap_or_default();
        let fs = self.filesystem.to_string();
        let root_pass = if self.filesystem == Filesystem::Ext4 {
            1
        } else {
            0
        };
        out.push_str(&format!(
            "UUID={root_uuid}  /  {fs}  {}  0 {root_pass}\n",
            self.root_options()
        ));
        if self.filesystem == Filesystem::Btrfs {
            out.push_str(&format!(
                "UUID={root_uuid}  /home  btrfs  rw,noatime,compress=zstd:1,subvol=/@home  0 0\n"
            ));
        }
        let esp = self.esp();
        out.push_str(&format!(
            "UUID={}  /boot/efi  vfat  rw,relatime,fmask=0077,dmask=0077  0 2\n",
            fat_uuid_display(esp.uuid.as_deref().unwrap_or_default())
        ));
        if let Some(swap) = self.get(Role::Swap) {
            out.push_str(&format!(
                "UUID={}  none  swap  defaults  0 0\n",
                swap.uuid.as_deref().unwrap_or_default()
            ));
        }
        out
    }
}

/// `mkfs.fat -i 1A2B3C4D` shows up as `UUID=1A2B-3C4D`.
pub fn fat_uuid_display(volume_id: &str) -> String {
    let id = volume_id.to_ascii_uppercase();
    if id.len() == 8 {
        format!("{}-{}", &id[..4], &id[4..])
    } else {
        id
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::GIB;
    use crate::ids::Sequential;

    #[test]
    fn partition_names() {
        assert_eq!(partition_path("/dev/sda", 1), "/dev/sda1");
        assert_eq!(partition_path("/dev/vdb", 4), "/dev/vdb4");
        assert_eq!(partition_path("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
        assert_eq!(partition_path("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
        assert_eq!(partition_path("/dev/loop7", 1), "/dev/loop7p1");
    }

    #[test]
    fn layout_with_swap() {
        let l = Layout::new(
            "/dev/nvme0n1",
            Filesystem::Ext4,
            512 * MIB,
            Some(4 * GIB),
            &mut Sequential::default(),
        );
        let roles: Vec<(u32, Role, &str)> = l
            .partitions
            .iter()
            .map(|p| (p.number, p.role, p.path.as_str()))
            .collect();
        assert_eq!(
            roles,
            vec![
                (1, Role::Esp, "/dev/nvme0n1p1"),
                (2, Role::BiosBoot, "/dev/nvme0n1p2"),
                (3, Role::Swap, "/dev/nvme0n1p3"),
                (4, Role::Root, "/dev/nvme0n1p4"),
            ]
        );
        assert_eq!(
            l.sfdisk_script(),
            "label: gpt\n\
             size=512MiB, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B, name=\"EFI system partition\"\n\
             size=1MiB, type=21686148-6449-6E6F-744E-656564454649, name=\"BIOS boot partition\"\n\
             size=4096MiB, type=0657FD6D-A4AB-43C4-84E5-0933C84B4F4F, name=\"Swap\"\n\
             type=4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709, name=\"Wincraft\"\n"
        );
        assert_eq!(l.fixed_size(), 512 * MIB + MIB + 4 * GIB + 2 * MIB);
    }

    #[test]
    fn fstab_ext4_without_swap() {
        let l = Layout::new(
            "/dev/sda",
            Filesystem::Ext4,
            512 * MIB,
            None,
            &mut Sequential::default(),
        );
        assert_eq!(l.root().number, 3);
        let fstab = l.fstab();
        let lines: Vec<&str> = fstab.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            lines,
            vec![
                "UUID=00000000-0000-4000-8000-000000000002  /  ext4  rw,relatime  0 1",
                "UUID=0000-0001  /boot/efi  vfat  rw,relatime,fmask=0077,dmask=0077  0 2",
            ]
        );
    }

    #[test]
    fn fstab_btrfs_with_subvolumes_and_swap() {
        let l = Layout::new(
            "/dev/sda",
            Filesystem::Btrfs,
            512 * MIB,
            Some(2 * GIB),
            &mut Sequential::default(),
        );
        let fstab = l.fstab();
        assert!(fstab.contains("/  btrfs  rw,noatime,compress=zstd:1,subvol=/@  0 0"));
        assert!(fstab.contains("/home  btrfs  rw,noatime,compress=zstd:1,subvol=/@home  0 0"));
        assert!(fstab.contains("none  swap  defaults  0 0"));
    }

    #[test]
    fn lsblk_modern_and_legacy_output() {
        let modern = r#"{"blockdevices":[
            {"path":"/dev/sda","size":64424509440,"type":"disk","model":"Samsung SSD 870","rm":false,"ro":false,"tran":"sata"},
            {"path":"/dev/sdb","size":15376318464,"type":"disk","model":"SanDisk","rm":true,"ro":false,"tran":"usb"},
            {"path":"/dev/sr0","size":1073741312,"type":"rom","model":"QEMU DVD","rm":true,"ro":false,"tran":"sata"},
            {"path":"/dev/zram0","size":4294967296,"type":"disk","model":null,"rm":false,"ro":false,"tran":null}
        ]}"#;
        let disks = parse_lsblk(modern).unwrap();
        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].path, "/dev/sda");
        assert_eq!(disks[0].size, 64424509440);
        assert!(disks[1].removable);
        assert!(disks[1].label().ends_with("(removable)"));

        let legacy = r#"{"blockdevices":[{"path":"/dev/vda","size":"21474836480","type":"disk","model":null,"rm":"0","ro":"0","tran":null}]}"#;
        let disks = parse_lsblk(legacy).unwrap();
        assert_eq!(disks[0].size, 21474836480);
        assert!(!disks[0].removable);
        assert_eq!(disks[0].label(), "/dev/vda  20.0 GiB  Disk");
    }

    #[test]
    fn sizes_and_fat_ids() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536 * MIB), "1.5 GiB");
        assert_eq!(fat_uuid_display("1a2b3c4d"), "1A2B-3C4D");
    }
}
