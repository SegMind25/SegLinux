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

/// Packages the Lite edition leaves out of its image and Setup downloads
/// into the new system: everything else the full edition ships. One name
/// per line; written by the ISO build (iso/scripts/online-packages.sh).
pub const ONLINE_PACKAGES: &str = "/usr/share/wincraft/online-packages";

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

/// A display controller on the PCI bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Gpu {
    pub vendor: u16,
    pub device: u16,
}

impl Gpu {
    pub const NVIDIA: u16 = 0x10de;
    pub const AMD: u16 = 0x1002;
    pub const INTEL: u16 = 0x8086;

    /// GeForce GTX 16, RTX 20 (Turing, device IDs from 0x1E00) and newer
    /// run NVIDIA's open kernel modules; older generations do not.
    pub fn nvidia_open_capable(&self) -> bool {
        self.vendor == Self::NVIDIA && self.device >= 0x1e00
    }
}

impl std::fmt::Display for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let vendor = match self.vendor {
            Self::NVIDIA => "NVIDIA",
            Self::AMD => "AMD",
            Self::INTEL => "Intel",
            _ => "GPU",
        };
        write!(f, "{vendor} {:04x}:{:04x}", self.vendor, self.device)
    }
}

/// Display controllers (PCI class 0x03) below `/sys/bus/pci/devices`.
pub fn list_gpus(pci_devices: &Path) -> Vec<Gpu> {
    let read_hex = |dir: &Path, name: &str| -> Option<u32> {
        let text = fs::read_to_string(dir.join(name)).ok()?;
        u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
    };
    let Ok(entries) = fs::read_dir(pci_devices) else {
        return Vec::new();
    };
    let mut gpus: Vec<(PathBuf, Gpu)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter_map(|dir| {
            let class = read_hex(&dir, "class")?;
            (class >> 16 == 0x03).then_some(())?;
            let gpu = Gpu {
                vendor: read_hex(&dir, "vendor")? as u16,
                device: read_hex(&dir, "device")? as u16,
            };
            Some((dir, gpu))
        })
        .collect();
    gpus.sort_by(|a, b| a.0.cmp(&b.0));
    gpus.into_iter().map(|(_, g)| g).collect()
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
    /// Packages to download during Setup (Lite edition, see [`ONLINE_PACKAGES`]).
    pub online_packages: Vec<String>,
    /// Whether the live system has a default network route.
    pub network: bool,
    pub memory: u64,
    pub gpus: Vec<Gpu>,
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
            online_packages: fs::read_to_string(ONLINE_PACKAGES)
                .map(|t| parse_package_list(&t))
                .unwrap_or_default(),
            network: has_default_route(),
            memory: memory_bytes().unwrap_or(0),
            gpus: list_gpus(Path::new("/sys/bus/pci/devices")),
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
/// booted with `copytoram`): the usual locations first, then a shallow
/// search.
pub fn find_image(dir: &Path) -> Option<PathBuf> {
    for known in [
        "bootmnt/LiveOS/rootfs.img",
        "copytoram/rootfs.img",
        "bootmnt/artix/x86_64/rootfs.sfs",
    ] {
        let p = dir.join(known);
        if p.is_file() {
            return Some(p);
        }
    }
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

/// Package names, one per line; `#` starts a comment. Anything that is not
/// a valid pacman package name is dropped.
pub fn parse_package_list(text: &str) -> Vec<String> {
    let valid = |name: &str| {
        !name.starts_with(['-', '.'])
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "@._+-".contains(c))
    };
    let mut names: Vec<String> = Vec::new();
    for line in text.lines() {
        let name = line.split('#').next().unwrap_or_default().trim();
        if !name.is_empty() && valid(name) && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// Whether this computer has a default network route right now.
pub fn has_default_route() -> bool {
    let v4 = fs::read_to_string("/proc/net/route").unwrap_or_default();
    let v6 = fs::read_to_string("/proc/net/ipv6_route").unwrap_or_default();
    parse_default_route(&v4, &v6)
}

/// Whether `/proc/net/route` or `/proc/net/ipv6_route` has a default route
/// (destination 0/0) on an interface other than loopback.
pub fn parse_default_route(route: &str, ipv6_route: &str) -> bool {
    let v4 = route.lines().skip(1).any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        f.len() > 7 && f[0] != "lo" && f[1] == "00000000" && f[7] == "00000000"
    });
    let v6 = ipv6_route.lines().any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        f.len() == 10 && f[9] != "lo" && f[0].bytes().all(|b| b == b'0') && f[1] == "00"
    });
    v4 || v6
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
        if removable
            && has_fs
            && unmounted
            && d["type"] != "rom"
            && let Some(p) = d["path"].as_str()
        {
            out.push(p.to_string());
        }
        collect_removable(&d["children"], removable, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_lists() {
        let text = "# generated\nwine\nsteam # games\n\n  lib32-mesa  \nwine\nBad Name\n-rf\nqt6-base\nlibc++\n";
        assert_eq!(
            parse_package_list(text),
            vec!["wine", "steam", "lib32-mesa", "qt6-base", "libc++"]
        );
        assert!(parse_package_list("").is_empty());
    }

    #[test]
    fn default_routes() {
        let header =
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n";
        let wired = format!(
            "{header}enp1s0\t00000000\t0102A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\nenp1s0\t0002A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n"
        );
        let local_only =
            format!("{header}enp1s0\t0002A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n");
        let zero = "00000000000000000000000000000000";
        let v6_default = format!(
            "{zero} 00 {zero} 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003 wlan0\n"
        );
        let v6_lo = format!("{zero} 00 {zero} 00 {zero} ffffffff 00000001 00000000 00200200 lo\n");
        assert!(parse_default_route(&wired, ""));
        assert!(!parse_default_route(&local_only, ""));
        assert!(!parse_default_route(header, &v6_lo));
        assert!(parse_default_route(&local_only, &v6_default));
        assert!(!parse_default_route("", ""));
    }

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
    fn gpus_from_sysfs() {
        let dir = tempfile::tempdir().unwrap();
        let dev = |slot: &str, class: &str, vendor: &str, device: &str| {
            let d = dir.path().join(slot);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("class"), format!("{class}\n")).unwrap();
            fs::write(d.join("vendor"), format!("{vendor}\n")).unwrap();
            fs::write(d.join("device"), format!("{device}\n")).unwrap();
        };
        dev("0000:00:02.0", "0x030000", "0x8086", "0x9a49"); // Intel iGPU
        dev("0000:01:00.0", "0x030000", "0x10de", "0x2684"); // RTX 4090
        dev("0000:01:00.1", "0x040300", "0x10de", "0x22ba"); // its HDMI audio
        let gpus = list_gpus(dir.path());
        assert_eq!(gpus.len(), 2);
        assert!(!gpus[0].nvidia_open_capable());
        assert!(gpus[1].nvidia_open_capable());
        assert_eq!(gpus[1].to_string(), "NVIDIA 10de:2684");
        // GTX 1080 (Pascal) needs the legacy driver.
        assert!(
            !Gpu {
                vendor: Gpu::NVIDIA,
                device: 0x1b80
            }
            .nvidia_open_capable()
        );
        assert!(list_gpus(&dir.path().join("missing")).is_empty());
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
