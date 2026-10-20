//! What the computer is: its CPU (and whether one of its cache complexes is
//! the big V-Cache one of an X3D part) and its graphics cards, read from
//! /proc and /sys below a root directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::sys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    Amd,
    Intel,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpu {
    pub vendor: CpuVendor,
    pub model: String,
}

/// `vendor_id` and `model name` of the first processor in /proc/cpuinfo.
pub fn parse_cpuinfo(text: &str) -> Cpu {
    let field = |key: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    };
    let vendor = match field("vendor_id").as_deref() {
        Some("AuthenticAMD") => CpuVendor::Amd,
        Some("GenuineIntel") => CpuVendor::Intel,
        _ => CpuVendor::Other,
    };
    Cpu {
        vendor,
        model: field("model name").unwrap_or_default(),
    }
}

pub fn cpu(root: &Path) -> Cpu {
    parse_cpuinfo(&sys::read_trimmed(root.join("proc/cpuinfo")).unwrap_or_default())
}

/// `0-7,16-23` -> the CPU numbers. Anything malformed gives None.
pub fn parse_cpulist(text: &str) -> Option<Vec<usize>> {
    let mut cpus = Vec::new();
    for part in text.trim().split(',') {
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b): (usize, usize) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
                if b < a || b - a > 4096 {
                    return None;
                }
                cpus.extend(a..=b);
            }
            None => cpus.push(part.trim().parse().ok()?),
        }
    }
    cpus.sort_unstable();
    cpus.dedup();
    (!cpus.is_empty()).then_some(cpus)
}

/// `[0,1,2,3,8]` -> `0-3,8`.
pub fn format_cpulist(cpus: &[usize]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&start) = cpus.get(i) {
        let mut end = start;
        while cpus.get(i + 1) == Some(&(end + 1)) {
            end += 1;
            i += 1;
        }
        out.push(if end == start {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        i += 1;
    }
    out.join(",")
}

/// `98304K` / `32M` -> KiB.
pub fn parse_cache_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let (digits, factor) = if let Some(d) = text.strip_suffix('K') {
        (d, 1)
    } else if let Some(d) = text.strip_suffix('M') {
        (d, 1024)
    } else {
        (text, 1)
    };
    digits.parse::<u64>().ok()?.checked_mul(factor)
}

/// The CPUs that share one last-level cache, and its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheGroup {
    pub cpus: Vec<usize>,
    pub size_kib: u64,
}

/// The L3 caches of the machine, one per group of CPUs sharing it, found as
/// `cpu*/cache/index*/` entries of level 3.
pub fn l3_groups(root: &Path) -> Vec<CacheGroup> {
    let Ok(entries) = fs::read_dir(root.join("sys/devices/system/cpu")) else {
        return Vec::new();
    };
    let mut groups: BTreeMap<Vec<usize>, u64> = BTreeMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name
            .strip_prefix("cpu")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let Ok(indexes) = fs::read_dir(entry.path().join("cache")) else {
            continue;
        };
        for index in indexes.flatten() {
            let dir = index.path();
            if sys::read_trimmed(dir.join("level")).as_deref() != Some("3") {
                continue;
            }
            let size = sys::read_trimmed(dir.join("size")).and_then(|s| parse_cache_size(&s));
            let shared =
                sys::read_trimmed(dir.join("shared_cpu_list")).and_then(|s| parse_cpulist(&s));
            if let (Some(size), Some(shared)) = (size, shared) {
                groups.insert(shared, size);
            }
        }
    }
    groups
        .into_iter()
        .map(|(cpus, size_kib)| CacheGroup { cpus, size_kib })
        .collect()
}

/// The V-Cache complex of an X3D part with two complexes: the one whose L3
/// is bigger than another's. None when all complexes are alike (a single
/// complex, or ordinary CPUs): there is nothing to choose then.
pub fn vcache_group(groups: &[CacheGroup]) -> Option<&CacheGroup> {
    let biggest = groups.iter().max_by_key(|g| g.size_kib)?;
    let smallest = groups.iter().min_by_key(|g| g.size_kib)?;
    (biggest.size_kib > smallest.size_kib).then_some(biggest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    Amd,
    Nvidia,
    Intel,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    /// `card0`
    pub card: String,
    pub vendor: GpuVendor,
    pub device_id: u16,
    /// The kernel driver bound to it (`amdgpu`, `i915`, `xe`, `nvidia`,
    /// `nouveau`), if one is.
    pub driver: Option<String>,
    pub slot: Option<String>,
    pub boot_vga: bool,
}

impl Gpu {
    /// Turing (GeForce GTX 16 / RTX 20) and newer: the generations NVIDIA's
    /// open kernel modules support (the same rule SegLinux Setup uses).
    pub fn nvidia_open_capable(&self) -> bool {
        self.vendor == GpuVendor::Nvidia && self.device_id >= 0x1e00
    }
}

fn parse_hex(text: &str) -> Option<u16> {
    u16::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}

/// `DRIVER=` and `PCI_SLOT_NAME=` from a device's uevent file.
pub fn parse_uevent(text: &str) -> (Option<String>, Option<String>) {
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(str::to_string)
    };
    (get("DRIVER"), get("PCI_SLOT_NAME"))
}

/// The graphics cards of /sys/class/drm (`card0`, `card1`; not their
/// connectors, `card0-HDMI-A-1`).
pub fn gpus(root: &Path) -> Vec<Gpu> {
    let Ok(entries) = fs::read_dir(root.join("sys/class/drm")) else {
        return Vec::new();
    };
    let mut found: Vec<Gpu> = entries
        .flatten()
        .filter_map(|e| {
            let card = e.file_name().into_string().ok()?;
            let number = card.strip_prefix("card")?;
            if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let dev = e.path().join("device");
            let vendor = match parse_hex(&sys::read_trimmed(dev.join("vendor"))?)? {
                0x1002 => GpuVendor::Amd,
                0x10de => GpuVendor::Nvidia,
                0x8086 => GpuVendor::Intel,
                _ => GpuVendor::Other,
            };
            let (driver, slot) =
                parse_uevent(&sys::read_trimmed(dev.join("uevent")).unwrap_or_default());
            Some(Gpu {
                card,
                vendor,
                device_id: parse_hex(&sys::read_trimmed(dev.join("device"))?)?,
                driver,
                slot,
                boot_vga: sys::read_trimmed(dev.join("boot_vga")).as_deref() == Some("1"),
            })
        })
        .collect();
    found.sort_by(|a, b| a.card.cmp(&b.card));
    found
}

/// The device name `lspci -s SLOT -mm` gives: `1002 "Advanced Micro
/// Devices, Inc. [AMD/ATI]" "Navi 31 [Radeon RX 7900 XT/7900 XTX]"`.
pub fn parse_lspci_name(output: &str) -> Option<String> {
    // Machine format: slot, then quoted fields: class, vendor, device, ...
    let mut fields = Vec::new();
    let mut rest = output.lines().next()?;
    while let Some(start) = rest.find('"') {
        let after = rest.get(start + 1..)?;
        let end = after.find('"')?;
        fields.push(after.get(..end)?.to_string());
        rest = after.get(end + 1..)?;
    }
    let vendor = fields.get(1)?;
    let device = fields.get(2)?;
    Some(format!("{vendor} {device}"))
}

pub fn lspci_name(slot: &str) -> Option<String> {
    sys::which("lspci")?;
    let out = sys::run("lspci", &["-s", slot, "-mm"]).ok()?;
    out.ok.then(|| parse_lspci_name(&out.stdout)).flatten()
}

/// What each GPU is called, for messages: lspci's name when it can be had.
pub fn describe(gpu: &Gpu) -> String {
    let named = gpu.slot.as_deref().and_then(lspci_name);
    named.unwrap_or_else(|| {
        let vendor = match gpu.vendor {
            GpuVendor::Amd => "AMD",
            GpuVendor::Nvidia => "NVIDIA",
            GpuVendor::Intel => "Intel",
            GpuVendor::Other => "unknown vendor",
        };
        format!("{vendor} GPU {:04x}", gpu.device_id)
    })
}

/// `/usr/lib/modules/<release>/pkgbase` of the running kernel: `linux-zen`.
pub fn kernel_package(root: &Path) -> Option<String> {
    let release = sys::read_trimmed(root.join("proc/sys/kernel/osrelease"))?;
    let base = sys::read_trimmed(root.join(format!("usr/lib/modules/{release}/pkgbase")))?;
    (!base.is_empty()
        && base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c)))
    .then_some(base)
}

/// `7.2.11-zen1-1-zen` -> (7, 2).
pub fn kernel_version(release: &str) -> Option<(u32, u32)> {
    let mut parts = release.trim().split(|c: char| !c.is_ascii_digit());
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// The cpufreq policy directories (one per CPU or group) below `root`.
pub fn cpufreq_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root.join("sys/devices/system/cpu")) else {
        return Vec::new();
    };
    let mut dirs: Vec<(usize, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let n: usize = name.strip_prefix("cpu")?.parse().ok()?;
            let dir = e.path().join("cpufreq");
            dir.is_dir().then_some((n, dir))
        })
        .collect();
    dirs.sort();
    dirs.into_iter().map(|(_, d)| d).collect()
}

#[cfg(test)]
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_lists() {
        assert_eq!(parse_cpulist("0-3,8"), Some(vec![0, 1, 2, 3, 8]));
        assert_eq!(
            parse_cpulist("0-7,16-23\n"),
            Some((0..8).chain(16..24).collect())
        );
        assert_eq!(parse_cpulist("5"), Some(vec![5]));
        assert_eq!(parse_cpulist("3,1,1"), Some(vec![1, 3]));
        for bad in ["", "a", "3-1", "1-", "0-99999", "1,,2"] {
            assert_eq!(parse_cpulist(bad), None, "{bad}");
        }
        assert_eq!(format_cpulist(&[0, 1, 2, 3, 8]), "0-3,8");
        assert_eq!(format_cpulist(&[4]), "4");
        assert_eq!(format_cpulist(&[]), "");
        let all: Vec<usize> = (0..8).chain(16..24).collect();
        assert_eq!(format_cpulist(&all), "0-7,16-23");
    }

    #[test]
    fn cache_sizes() {
        assert_eq!(parse_cache_size("98304K"), Some(98304));
        assert_eq!(parse_cache_size("32M"), Some(32768));
        assert_eq!(parse_cache_size("512"), Some(512));
        assert_eq!(parse_cache_size("lots"), None);
    }

    #[test]
    fn cpuinfo() {
        let c = cpu(&fixture("ryzen-7950x3d"));
        assert_eq!(c.vendor, CpuVendor::Amd);
        assert_eq!(c.model, "AMD Ryzen 9 7950X3D 16-Core Processor");
        assert_eq!(cpu(&fixture("intel-12700")).vendor, CpuVendor::Intel);
        assert_eq!(cpu(&fixture("vm")).model, "QEMU Virtual CPU version 2.5+");
        assert_eq!(parse_cpuinfo("").vendor, CpuVendor::Other);
    }

    #[test]
    fn a_dual_ccd_x3d_has_one_vcache_complex() {
        let groups = l3_groups(&fixture("ryzen-7950x3d"));
        assert_eq!(groups.len(), 2);
        let v = vcache_group(&groups).expect("a V-Cache complex");
        assert_eq!(v.size_kib, 98304);
        assert_eq!(format_cpulist(&v.cpus), "0-7,16-23");
    }

    #[test]
    fn the_cache_decides_not_the_complex_number() {
        let groups = l3_groups(&fixture("ryzen-second-ccd-vcache"));
        let v = vcache_group(&groups).expect("a V-Cache complex");
        assert_eq!(format_cpulist(&v.cpus), "8-15,24-31");
    }

    #[test]
    fn nothing_to_pin_when_the_complexes_are_alike() {
        // One complex, all V-Cache (7800X3D).
        let groups = l3_groups(&fixture("ryzen-7800x3d"));
        assert_eq!(groups.len(), 1);
        assert_eq!(vcache_group(&groups), None);
        // Two ordinary complexes.
        assert_eq!(vcache_group(&l3_groups(&fixture("ryzen-5950x"))), None);
        assert_eq!(vcache_group(&l3_groups(&fixture("intel-12700"))), None);
        // A virtual machine tells nothing.
        assert!(l3_groups(&fixture("vm")).is_empty());
        assert_eq!(vcache_group(&[]), None);
    }

    #[test]
    fn the_gpu_of_an_amd_desktop() {
        let g = gpus(&fixture("gpu-amd"));
        assert_eq!(g.len(), 1, "connectors are not cards: {g:?}");
        assert_eq!(g[0].card, "card0");
        assert_eq!(g[0].vendor, GpuVendor::Amd);
        assert_eq!(g[0].device_id, 0x744c);
        assert_eq!(g[0].driver.as_deref(), Some("amdgpu"));
        assert_eq!(g[0].slot.as_deref(), Some("0000:03:00.0"));
        assert!(g[0].boot_vga);
    }

    #[test]
    fn nvidia_generations() {
        let ada = &gpus(&fixture("gpu-nvidia-ada"))[0];
        assert!(ada.nvidia_open_capable());
        assert_eq!(ada.driver.as_deref(), Some("nvidia"));
        // The same card on the open-source driver, before the proprietary
        // one is installed.
        assert_eq!(
            gpus(&fixture("gpu-nvidia-ada-nouveau"))[0]
                .driver
                .as_deref(),
            Some("nouveau")
        );
        let pascal = &gpus(&fixture("gpu-nvidia-pascal"))[0];
        assert_eq!(pascal.vendor, GpuVendor::Nvidia);
        assert!(!pascal.nvidia_open_capable());
        assert!(!gpus(&fixture("gpu-amd"))[0].nvidia_open_capable());
    }

    #[test]
    fn a_hybrid_laptop_has_two_cards_in_order() {
        let g = gpus(&fixture("gpu-hybrid-laptop"));
        assert_eq!(g.len(), 2);
        assert_eq!(
            (g[0].card.as_str(), g[0].vendor, g[0].boot_vga),
            ("card0", GpuVendor::Intel, true)
        );
        assert_eq!(
            (g[1].card.as_str(), g[1].vendor, g[1].boot_vga),
            ("card1", GpuVendor::Nvidia, false)
        );
        assert!(g[1].nvidia_open_capable());
        assert_eq!(
            gpus(&fixture("gpu-intel-arc"))[0].driver.as_deref(),
            Some("i915")
        );
    }

    #[test]
    fn no_gpu_and_no_drm_directory() {
        assert!(gpus(&fixture("gpu-none")).is_empty());
        assert!(gpus(&fixture("vm")).is_empty());
        assert!(gpus(Path::new("/definitely/not/here")).is_empty());
    }

    #[test]
    fn uevent_and_lspci_text() {
        assert_eq!(
            parse_uevent("DRIVER=amdgpu\nPCI_SLOT_NAME=0000:03:00.0\n"),
            (Some("amdgpu".into()), Some("0000:03:00.0".into()))
        );
        assert_eq!(parse_uevent("PCI_CLASS=30000\n"), (None, None));
        let out = "03:00.0 \"VGA compatible controller\" \"Advanced Micro Devices, Inc. [AMD/ATI]\" \"Navi 31 [Radeon RX 7900 XT/7900 XTX]\" -rc8 \"Sapphire\" \"Nitro\"\n";
        assert_eq!(
            parse_lspci_name(out).as_deref(),
            Some("Advanced Micro Devices, Inc. [AMD/ATI] Navi 31 [Radeon RX 7900 XT/7900 XTX]")
        );
        assert_eq!(parse_lspci_name(""), None);
        assert_eq!(parse_lspci_name("03:00.0 \"VGA\""), None);
    }

    #[test]
    fn describe_falls_back_to_the_vendor() {
        let mut g = gpus(&fixture("gpu-amd"))[0].clone();
        g.slot = None;
        assert_eq!(describe(&g), "AMD GPU 744c");
    }

    #[test]
    fn kernel_names_and_versions() {
        assert_eq!(kernel_version("7.2.11-zen1-1-zen"), Some((7, 2)));
        assert_eq!(kernel_version("6.14.0-arch1-1\n"), Some((6, 14)));
        assert_eq!(kernel_version("garbage"), None);
        let t = sys::testutil::TempDir::new();
        t.put("proc/sys/kernel/osrelease", "7.2.11-zen1-1-zen\n");
        t.put("usr/lib/modules/7.2.11-zen1-1-zen/pkgbase", "linux-zen\n");
        assert_eq!(kernel_package(t.path()).as_deref(), Some("linux-zen"));
        t.put(
            "usr/lib/modules/7.2.11-zen1-1-zen/pkgbase",
            "linux zen; rm\n",
        );
        assert_eq!(kernel_package(t.path()), None);
    }

    #[test]
    fn cpufreq_policies_are_listed_in_cpu_order() {
        let t = sys::testutil::TempDir::new();
        for n in [10, 2, 0] {
            t.put(
                &format!("sys/devices/system/cpu/cpu{n}/cpufreq/scaling_governor"),
                "powersave\n",
            );
        }
        t.put("sys/devices/system/cpu/cpu1/online", "1\n");
        t.put("sys/devices/system/cpu/cpufreq/boost", "1\n");
        let dirs = cpufreq_dirs(t.path());
        let names: Vec<_> = dirs
            .iter()
            .map(|d| {
                d.parent()
                    .and_then(|p| p.file_name())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["cpu0", "cpu2", "cpu10"]);
    }
}
