//! What Setup needs to know about the machine it runs on.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use wincraft_core::cmd::Cmd;
use wincraft_core::init::InitSystem;

use crate::answer::{ANSWER_FILE_NAME, AnswerSource, path_from_cmdline};
use crate::disk::{self, Disk};

/// Where Artix's live initramfs mounts the install media.
pub const BOOT_MOUNT: &str = "/run/artix/bootmnt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Firmware {
    Uefi,
    Bios,
}

impl std::fmt::Display for Firmware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Firmware::Uefi => "UEFI",
            Firmware::Bios => "BIOS",
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Facts {
    pub firmware: Firmware,
    #[serde(serialize_with = "serialize_init")]
    pub init: InitSystem,
    pub disks: Vec<Disk>,
    /// Disk the live system booted from; never offered as a target.
    pub boot_disk: Option<String>,
    /// The system image (`LiveOS/rootfs.img`).
    pub image: Option<PathBuf>,
    pub oem_dir: Option<PathBuf>,
    pub offline_packages: Vec<PathBuf>,
    pub memory: u64,
    /// Known values for validation and pick lists; `None` = not checked.
    pub timezones: Option<BTreeSet<String>>,
    pub locales: Option<BTreeSet<String>>,
    pub keyboards: Option<BTreeSet<String>>,
}

impl Facts {
    /// Probe the running live system.
    pub fn probe() -> Result<Facts> {
        let boot = Path::new(BOOT_MOUNT);
        let lsblk = Cmd::new("lsblk")
            .args(["-J", "-b", "-d", "-o", "PATH,SIZE,TYPE,MODEL,RM,RO,TRAN"])
            .output()?;
        Ok(Facts {
            firmware: if Path::new("/sys/firmware/efi").exists() {
                Firmware::Uefi
            } else {
                Firmware::Bios
            },
            init: InitSystem::detect(Path::new("/")).unwrap_or(InitSystem::OpenRc),
            disks: disk::parse_lsblk(&lsblk)?,
            boot_disk: boot_disk(),
            image: find_image(Path::new("/run/artix")),
            oem_dir: Some(boot.join("wincraft/oem")).filter(|p| p.is_dir()),
            offline_packages: list_packages(&boot.join("wincraft/packages")),
            memory: memory_bytes().unwrap_or(0),
            timezones: Some(list_timezones(Path::new("/usr/share/zoneinfo")))
                .filter(|s| !s.is_empty()),
            locales: fs::read_to_string("/usr/share/i18n/SUPPORTED")
                .ok()
                .map(|t| parse_supported_locales(&t)),
            keyboards: fs::read_to_string("/usr/share/X11/xkb/rules/base.lst")
                .ok()
                .map(|t| parse_xkb_layouts(&t)),
        })
    }

    /// Disks that can be installed to.
    pub fn candidate_disks(&self) -> Vec<&Disk> {
        self.disks
            .iter()
            .filter(|d| !d.read_only && Some(&d.path) != self.boot_disk.as_ref())
            .collect()
    }

    /// The disk `auto` picks: the largest internal one that is big enough.
    pub fn auto_disk(&self) -> Option<&Disk> {
        self.candidate_disks()
            .into_iter()
            .filter(|d| !d.removable && d.transport != "usb" && d.size >= disk::MIN_DISK_SIZE)
            .max_by_key(|d| d.size)
    }
}

fn serialize_init<S: serde::Serializer>(init: &InitSystem, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(init.name())
}

fn boot_disk() -> Option<String> {
    let source = Cmd::new("findmnt")
        .args(["-n", "-o", "SOURCE", BOOT_MOUNT])
        .output()
        .ok()?;
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    let parent = Cmd::new("lsblk")
        .args(["-n", "-o", "PKNAME", source])
        .output()
        .ok()?;
    let parent = parent.lines().next().unwrap_or("").trim();
    Some(if parent.is_empty() {
        source.to_string()
    } else {
        format!("/dev/{parent}")
    })
}

/// Find the system image below `dir` (the media, or the copy in RAM when
/// booted with `copytoram`).
pub fn find_image(dir: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: u32) -> Option<PathBuf> {
        if depth > 4 {
            return None;
        }
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in &entries {
            let name = p.file_name()?.to_string_lossy();
            if p.is_file() && (name == "rootfs.img" || name == "rootfs.sfs") {
                return Some(p.clone());
            }
        }
        entries
            .iter()
            .filter(|p| p.is_dir() && !p.is_symlink())
            .find_map(|p| walk(p, depth + 1))
    }
    walk(dir, 0)
}

pub fn list_packages(dir: &Path) -> Vec<PathBuf> {
    let mut pkgs: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    name.contains(".pkg.tar.") && !name.ends_with(".sig")
                })
                .collect()
        })
        .unwrap_or_default();
    pkgs.sort();
    pkgs
}

pub fn memory_bytes() -> Option<u64> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo_total(&text)
}

pub fn parse_meminfo_total(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// Lines such as `en_US.UTF-8 UTF-8` from `/usr/share/i18n/SUPPORTED`.
pub fn parse_supported_locales(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|l| !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Layout names from the `! layout` section of xkb's `base.lst`.
pub fn parse_xkb_layouts(text: &str) -> BTreeSet<String> {
    let mut in_layouts = false;
    let mut out = BTreeSet::new();
    for line in text.lines() {
        if let Some(section) = line.strip_prefix('!') {
            in_layouts = section.trim() == "layout";
            continue;
        }
        if in_layouts && let Some(name) = line.split_whitespace().next() {
            out.insert(name.to_string());
        }
    }
    out
}

/// Region/City names below `/usr/share/zoneinfo`.
pub fn list_timezones(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(base: &Path, dir: &Path, out: &mut BTreeSet<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            // Skip the duplicate trees and metadata files.
            if matches!(
                name.as_str(),
                "posix" | "right" | "posixrules" | "localtime" | "Factory"
            ) || name.contains('.')
                || name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            {
                continue;
            }
            if p.is_dir() {
                walk(base, &p, out);
            } else if let Ok(rel) = p.strip_prefix(base) {
                out.insert(rel.to_string_lossy().into_owned());
            }
        }
    }
    walk(dir, dir, &mut out);
    out
}

/// Look for an answer file the way Windows Setup looks for
/// `autounattend.xml`: explicit path, kernel command line, root of the
/// install media, root of other removable drives, then the live image.
pub fn find_answer_file(explicit: Option<&Path>) -> Result<Option<(PathBuf, AnswerSource)>> {
    if let Some(p) = explicit {
        return Ok(Some((p.to_path_buf(), AnswerSource::CommandLine)));
    }
    if let Ok(cmdline) = fs::read_to_string("/proc/cmdline")
        && let Some(p) = path_from_cmdline(&cmdline)
        && p.is_file()
    {
        return Ok(Some((p, AnswerSource::KernelCmdline)));
    }
    let on_media = Path::new(BOOT_MOUNT).join(ANSWER_FILE_NAME);
    if on_media.is_file() {
        return Ok(Some((on_media, AnswerSource::InstallMedia)));
    }
    if let Some(found) = search_removable_drives()? {
        return Ok(Some(found));
    }
    let baked = Path::new("/etc/wincraft").join(ANSWER_FILE_NAME);
    if baked.is_file() {
        return Ok(Some((baked, AnswerSource::LiveImage)));
    }
    Ok(None)
}

/// Mount each removable partition read-only and copy an answer file from
/// its root to /run/wincraft.
fn search_removable_drives() -> Result<Option<(PathBuf, AnswerSource)>> {
    let Ok(out) = Cmd::new("lsblk")
        .args(["-J", "-b", "-o", "PATH,TYPE,RM,TRAN,FSTYPE,MOUNTPOINT"])
        .output()
    else {
        return Ok(None);
    };
    let v: serde_json::Value = serde_json::from_str(&out)?;
    let mut parts = Vec::new();
    collect_removable(&v["blockdevices"], false, &mut parts);
    let probe_dir = Path::new("/run/wincraft/probe");
    for dev in parts {
        fs::create_dir_all(probe_dir)?;
        if Cmd::new("mount")
            .args(["-o", "ro", &dev])
            .arg(probe_dir.to_string_lossy())
            .output()
            .is_err()
        {
            continue;
        }
        let candidate = probe_dir.join(ANSWER_FILE_NAME);
        let copied = Path::new("/run/wincraft").join(ANSWER_FILE_NAME);
        let found = candidate.is_file() && fs::copy(&candidate, &copied).is_ok();
        let _ = Cmd::new("umount").arg(probe_dir.to_string_lossy()).output();
        if found {
            return Ok(Some((copied, AnswerSource::RemovableDrive(dev))));
        }
    }
    Ok(None)
}

fn collect_removable(devices: &serde_json::Value, parent_removable: bool, out: &mut Vec<String>) {
    let Some(list) = devices.as_array() else {
        return;
    };
    for d in list {
        let removable = parent_removable
            || matches!(d["rm"], serde_json::Value::Bool(true))
            || d["rm"] == "1"
            || d["tran"] == "usb";
        let has_fs = d["fstype"]
            .as_str()
            .is_some_and(|f| !f.is_empty() && f != "swap");
        let unmounted = d["mountpoint"].is_null();
        if removable && has_fs && unmounted && d["type"] != "rom" {
            if let Some(p) = d["path"].as_str() {
                out.push(p.to_string());
            }
        }
        collect_removable(&d["children"], removable, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo() {
        assert_eq!(
            parse_meminfo_total("MemTotal:        8040412 kB\nMemFree:  1 kB\n"),
            Some(8040412 * 1024)
        );
        assert_eq!(parse_meminfo_total("nope"), None);
    }

    #[test]
    fn locales_and_layouts() {
        let locales =
            parse_supported_locales("en_US.UTF-8 UTF-8\nen_US ISO-8859-1\nfr_FR.UTF-8 UTF-8\n");
        assert!(locales.contains("fr_FR.UTF-8"));
        assert_eq!(locales.len(), 3);
        let layouts = parse_xkb_layouts(
            "! model\n  pc105  Generic\n\n! layout\n  us   English (US)\n  fr   French\n\n! variant\n  intl  us: English\n",
        );
        assert_eq!(layouts.into_iter().collect::<Vec<_>>(), vec!["fr", "us"]);
    }

    #[test]
    fn finds_image_and_packages() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_image(dir.path()), None);
        fs::create_dir_all(dir.path().join("bootmnt/LiveOS")).unwrap();
        fs::write(dir.path().join("bootmnt/LiveOS/livefs.img"), "").unwrap();
        fs::write(dir.path().join("bootmnt/LiveOS/rootfs.img"), "").unwrap();
        assert_eq!(
            find_image(dir.path()),
            Some(dir.path().join("bootmnt/LiveOS/rootfs.img"))
        );

        let pkgs = dir.path().join("packages");
        fs::create_dir_all(&pkgs).unwrap();
        for f in [
            "b-1-1-x86_64.pkg.tar.zst",
            "a-1-1-any.pkg.tar.xz",
            "a-1-1-any.pkg.tar.xz.sig",
            "readme.txt",
        ] {
            fs::write(pkgs.join(f), "").unwrap();
        }
        let found: Vec<String> = list_packages(&pkgs)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            found,
            vec!["a-1-1-any.pkg.tar.xz", "b-1-1-x86_64.pkg.tar.zst"]
        );
    }

    #[test]
    fn timezones() {
        let dir = tempfile::tempdir().unwrap();
        for f in [
            "UTC",
            "Europe/Paris",
            "America/Argentina/Salta",
            "posix/UTC",
            "zone.tab",
            "right/UTC",
        ] {
            let p = dir.path().join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "").unwrap();
        }
        let tz = list_timezones(dir.path());
        assert_eq!(
            tz.into_iter().collect::<Vec<_>>(),
            vec!["America/Argentina/Salta", "Europe/Paris", "UTC"]
        );
    }

    #[test]
    fn removable_partitions_inherit_from_parent_disk() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"blockdevices":[
              {"path":"/dev/sda","type":"disk","rm":false,"tran":"sata","fstype":null,"mountpoint":null,
               "children":[{"path":"/dev/sda1","type":"part","rm":false,"tran":null,"fstype":"ext4","mountpoint":null}]},
              {"path":"/dev/sdb","type":"disk","rm":true,"tran":"usb","fstype":null,"mountpoint":null,
               "children":[{"path":"/dev/sdb1","type":"part","rm":false,"tran":null,"fstype":"vfat","mountpoint":null},
                           {"path":"/dev/sdb2","type":"part","rm":false,"tran":null,"fstype":"vfat","mountpoint":"/run/artix/bootmnt"}]}
            ]}"#,
        )
        .unwrap();
        let mut out = Vec::new();
        collect_removable(&v["blockdevices"], false, &mut out);
        assert_eq!(out, vec!["/dev/sdb1"]);
    }
}
