//! `seg game check`: is this computer ready for games? One line per
//! question, PASS or FAIL (or WARN when it works but is not as good as it
//! could be), and the exact command that fixes what is not right.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::hw::{self, Gpu, GpuVendor};
use crate::error::Result;
use crate::sys;

pub const HELP: &str = "\
Usage: seg game check

Checks Vulkan, the 32-bit graphics drivers, the GPU driver, NTSYNC,
vm.max_map_count, the open files limit, the kernel (and a fallback kernel),
the CPU governor, whether `seg game run` may boost the computer, transparent
hugepages, split lock detection, memory compaction and the security
mitigations. Prints PASS, WARN or FAIL for each, and the fix for each FAIL and
WARN. Exits with status 1 when something FAILs.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub fix: Option<String>,
}

fn check(
    name: &'static str,
    status: Status,
    detail: impl Into<String>,
    fix: Option<&str>,
) -> Check {
    Check {
        name,
        status,
        detail: detail.into(),
        fix: fix.map(str::to_string),
    }
}

/// What was found on the computer; `evaluate` only judges it.
#[derive(Debug, Default)]
pub struct Inputs {
    pub kernel_release: Option<String>,
    pub kernel_package: Option<String>,
    /// Kernel images in /boot (`vmlinuz-linux-zen`).
    pub kernel_images: Vec<String>,
    /// Vulkan devices; None when vulkaninfo is not installed or failed.
    pub vulkan_devices: Option<Vec<String>>,
    pub gpus: Vec<Gpu>,
    pub packages: BTreeSet<String>,
    pub ntsync_device_mode: Option<u32>,
    pub max_map_count: Option<u64>,
    /// (soft, hard); `u64::MAX` for unlimited.
    pub nofile: Option<(u64, u64)>,
    pub governor: Option<String>,
    pub is_root: bool,
    pub in_gamemode_group: bool,
    /// Whether sudo lets this user run the boost helper without a password
    /// (`sudo -n -l ...`); None when it was not asked.
    pub boost_allowed: Option<bool>,
    pub thp: Option<String>,
    pub split_lock_mitigate: Option<u64>,
    pub compaction_proactiveness: Option<u64>,
    pub cmdline: Option<String>,
    pub cpu: Option<hw::Cpu>,
    /// `amd-pstate-epp`, `acpi-cpufreq`, `intel_pstate`, ...
    pub scaling_driver: Option<String>,
}

/// `deviceName` lines of `vulkaninfo --summary`.
pub fn parse_vulkan_devices(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("deviceName")?;
            Some(rest.trim_start().strip_prefix('=')?.trim().to_string())
        })
        .collect()
}

/// `/proc/PID/limits`: the soft and hard value of "Max open files".
pub fn parse_nofile(limits: &str) -> Option<(u64, u64)> {
    let line = limits.lines().find(|l| l.starts_with("Max open files"))?;
    let mut f = line.trim_start_matches("Max open files").split_whitespace();
    let num = |w: &str| {
        if w == "unlimited" {
            Some(u64::MAX)
        } else {
            w.parse().ok()
        }
    };
    Some((num(f.next()?)?, num(f.next()?)?))
}

/// The supplementary group ids in /proc/PID/status.
pub fn parse_groups(status: &str) -> Vec<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Groups:"))
        .map(|g| {
            g.split_whitespace()
                .filter_map(|w| w.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn group_gid(group_file: &str, name: &str) -> Option<u32> {
    group_file.lines().find_map(|l| {
        let mut f = l.split(':');
        (f.next()? == name).then(|| f.nth(1)?.parse().ok())?
    })
}

/// Package names from pacman's local database directories (`bash-5.2-1`).
pub fn installed_packages(local_db: &Path) -> BTreeSet<String> {
    let Ok(entries) = fs::read_dir(local_db) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let mut parts = name.rsplitn(3, '-');
            parts.next()?; // pkgrel
            parts.next()?; // pkgver
            Some(parts.next()?.to_string())
        })
        .collect()
}

/// The 32-bit packages a GPU needs for 32-bit games (Steam is 32-bit).
pub fn lib32_packages(vendor: GpuVendor) -> &'static [&'static str] {
    match vendor {
        GpuVendor::Amd => &[
            "lib32-mesa",
            "lib32-vulkan-radeon",
            "lib32-vulkan-icd-loader",
        ],
        GpuVendor::Intel => &[
            "lib32-mesa",
            "lib32-vulkan-intel",
            "lib32-vulkan-icd-loader",
        ],
        GpuVendor::Nvidia => &["lib32-nvidia-utils", "lib32-vulkan-icd-loader"],
        GpuVendor::Other => &[],
    }
}

fn kernel_is_gaming_flavour(release: &str) -> bool {
    let r = release.to_lowercase();
    ["zen", "xanmod", "cachyos", "liquorix", "tkg"]
        .iter()
        .any(|f| r.contains(f))
}

pub fn evaluate(i: &Inputs) -> Vec<Check> {
    use Status::*;
    let mut out = Vec::new();

    // Vulkan
    out.push(match &i.vulkan_devices {
        None => check(
            "vulkan",
            Fail,
            "vulkaninfo did not run (not installed, or no Vulkan driver)",
            Some("sudo pacman -S vulkan-tools vulkan-icd-loader   (then: sudo seg game setup)"),
        ),
        Some(devices) => {
            let real: Vec<&String> = devices
                .iter()
                .filter(|d| !d.to_lowercase().contains("llvmpipe") && !d.to_lowercase().contains("lavapipe"))
                .collect();
            if let Some(first) = real.first() {
                check("vulkan", Pass, first.as_str(), None)
            } else if devices.is_empty() {
                check("vulkan", Fail, "vulkaninfo lists no device", Some("sudo seg game setup --yes"))
            } else {
                check(
                    "vulkan",
                    Fail,
                    "only the software renderer (llvmpipe) is available: games would run at a crawl",
                    Some("sudo seg game setup --yes   (installs the driver for the GPU found)"),
                )
            }
        }
    });

    // GPU driver in the kernel
    if i.gpus.is_empty() {
        out.push(check(
            "gpu driver",
            Skip,
            "no graphics card found in /sys/class/drm (virtual machine?)",
            None,
        ));
    }
    for gpu in &i.gpus {
        let name = hw::describe(gpu);
        let driver = gpu.driver.as_deref();
        out.push(match (gpu.vendor, driver) {
            (_, None) => check("gpu driver", Fail, format!("{name}: no kernel driver is bound"), Some("sudo seg game setup --yes   (then reboot)")),
            (GpuVendor::Amd, Some("amdgpu")) | (GpuVendor::Intel, Some("i915" | "xe")) => {
                check("gpu driver", Pass, format!("{name}: {}", driver.unwrap_or("")), None)
            }
            (GpuVendor::Nvidia, Some("nvidia")) => check("gpu driver", Pass, format!("{name}: nvidia"), None),
            (GpuVendor::Nvidia, Some("nouveau")) if gpu.nvidia_open_capable() => check(
                "gpu driver",
                Fail,
                format!("{name}: running on the open-source nouveau driver; NVIDIA's own is much faster for games"),
                Some("sudo seg game setup --yes   (installs nvidia-open-dkms; then reboot)"),
            ),
            (GpuVendor::Nvidia, Some("nouveau")) => check(
                "gpu driver",
                Pass,
                format!("{name}: nouveau (this card is too old for NVIDIA's current driver)"),
                None,
            ),
            (_, Some(other)) => check("gpu driver", Warn, format!("{name}: driver {other}"), None),
        });
    }

    // 32-bit drivers
    let mut missing: BTreeSet<&str> = BTreeSet::new();
    for gpu in &i.gpus {
        for pkg in lib32_packages(gpu.vendor) {
            if !i.packages.contains(*pkg) {
                missing.insert(pkg);
            }
        }
    }
    out.push(if i.gpus.is_empty() {
        check("32-bit drivers", Skip, "no graphics card found", None)
    } else if missing.is_empty() {
        check("32-bit drivers", Pass, "the lib32 graphics packages are installed", None)
    } else {
        let list = missing.iter().copied().collect::<Vec<_>>().join(" ");
        check(
            "32-bit drivers",
            Fail,
            format!("missing: {list} (Steam and many games are 32-bit; the [lib32] repository must be enabled)"),
            Some(&format!("sudo pacman -S {list}")),
        )
    });

    // NTSYNC
    let version = i.kernel_release.as_deref().and_then(hw::kernel_version);
    out.push(match (version, i.ntsync_device_mode) {
        (Some(v), _) if v < (6, 14) => check(
            "ntsync",
            Fail,
            format!("kernel {}.{} is older than 6.14, which added NTSYNC", v.0, v.1),
            Some("sudo pacman -S linux-zen linux-zen-headers   (then pick it in the boot menu)"),
        ),
        (_, None) => check(
            "ntsync",
            Fail,
            "/dev/ntsync does not exist: the module is not loaded",
            Some("sudo modprobe ntsync   (always: echo ntsync | sudo tee /etc/modules-load.d/10-ntsync.conf)"),
        ),
        (_, Some(mode)) if mode & 0o006 != 0o006 => check(
            "ntsync",
            Fail,
            format!("/dev/ntsync is mode {mode:o}: Wine, running as you, cannot use it"),
            Some("sudo chmod 666 /dev/ntsync   (always: KERNEL==\"ntsync\", MODE=\"0666\" in /etc/udev/rules.d/70-ntsync.rules)"),
        ),
        (_, Some(_)) => check("ntsync", Pass, "/dev/ntsync is there and usable by Wine and Proton", None),
    });

    // Memory maps
    out.push(match i.max_map_count {
        Some(n) if n >= 1_048_576 => check("max_map_count", Pass, n.to_string(), None),
        Some(n) => check(
            "max_map_count",
            Fail,
            format!("{n}: some games crash below 1048576"),
            Some("sudo sysctl -w vm.max_map_count=2147483642   (always: /etc/sysctl.d/80-seglinux-gaming.conf)"),
        ),
        None => check("max_map_count", Skip, "cannot read vm.max_map_count", None),
    });

    // Open files
    out.push(match i.nofile {
        Some((_, hard)) if hard >= 524_288 => check(
            "open files",
            Pass,
            format!("hard limit {}", if hard == u64::MAX { "unlimited".to_string() } else { hard.to_string() }),
            None,
        ),
        Some((_, hard)) => check(
            "open files",
            Fail,
            format!("hard limit {hard}: esync needs 524288 or more"),
            Some("add `*  hard  nofile  1048576` to /etc/security/limits.d/90-seglinux-esync.conf, then log out and in"),
        ),
        None => check("open files", Skip, "cannot read the limits of this process", None),
    });

    // Kernel
    out.push(match (&i.kernel_release, &i.kernel_package) {
        (Some(r), pkg) if kernel_is_gaming_flavour(r) => check(
            "kernel",
            Pass,
            format!("{r}{}", pkg.as_deref().map(|p| format!(" ({p})")).unwrap_or_default()),
            None,
        ),
        (Some(r), _) => check(
            "kernel",
            Warn,
            format!("{r}: a stock kernel; linux-zen is tuned for desktop latency"),
            Some("sudo pacman -S linux-zen linux-zen-headers && sudo grub-mkconfig -o /boot/grub/grub.cfg   (the current kernel stays as a fallback)"),
        ),
        (None, _) => check("kernel", Skip, "cannot read the kernel release", None),
    });
    out.push(if i.kernel_images.len() >= 2 {
        check("fallback kernel", Pass, i.kernel_images.join(", "), None)
    } else {
        check(
            "fallback kernel",
            Warn,
            "only one kernel is installed: if an update breaks it, there is nothing to boot instead",
            Some("sudo pacman -S linux && sudo grub-mkconfig -o /boot/grub/grub.cfg   (it appears under Advanced options)"),
        )
    });

    // Governor
    out.push(match i.governor.as_deref() {
        None => check("cpu governor", Skip, "no cpufreq on this machine (virtual machine?)", None),
        Some("performance") => check(
            "cpu governor",
            Warn,
            "performance all the time: the CPU runs hot and loud even when idle",
            Some("sudo seg tune balanced   (seg game run switches to performance only while a game runs)"),
        ),
        Some(g) => check(
            "cpu governor",
            Pass,
            format!("{g} at rest; performance while a game runs under seg game run"),
            None,
        ),
    });

    // AMD: the amd-pstate driver in active mode, the one with energy hints.
    if i.cpu
        .as_ref()
        .is_some_and(|c| c.vendor == hw::CpuVendor::Amd)
    {
        out.push(match i.scaling_driver.as_deref() {
            Some("amd-pstate-epp") => check("cpu driver", Pass, "amd-pstate in active mode", None),
            Some(other) => check(
                "cpu driver",
                Warn,
                format!("{other}: AMD CPUs from Zen 2 on do better with amd_pstate=active (the CPU picks frequencies itself, guided by the energy hints seg game run sets)"),
                Some("add amd_pstate=active to GRUB_CMDLINE_LINUX_DEFAULT in /etc/default/grub, then sudo grub-mkconfig -o /boot/grub/grub.cfg and reboot"),
            ),
            None => check("cpu driver", Skip, "no cpufreq driver (virtual machine?)", None),
        });
    }

    // May seg game run boost?
    out.push(if i.is_root {
        check("game boost", Pass, "running as root", None)
    } else if i.boost_allowed == Some(true) {
        check("game boost", Pass, "sudo lets you run the boost helper, as the gamemode group may", None)
    } else if !i.packages.contains("segctl") {
        check(
            "game boost",
            Fail,
            "segctl is not installed, so there is no sudo rule: seg game run cannot raise the governor",
            Some("sudo pacman -S segctl"),
        )
    } else if i.in_gamemode_group {
        check(
            "game boost",
            Fail,
            "you are in the gamemode group but sudo does not allow the boost helper",
            Some("sudo visudo -c   (a broken file in /etc/sudoers.d disables the rule; /etc/sudoers.d/seg-game comes from segctl)"),
        )
    } else {
        check(
            "game boost",
            Fail,
            "you are not in the gamemode group (or have not logged in again since): seg game run cannot raise the governor",
            Some("sudo gpasswd -a $USER gamemode   (then log out and in)"),
        )
    });

    // Transparent hugepages
    out.push(match i.thp.as_deref() {
        Some(t) if t.contains("[madvise]") => check("hugepages", Pass, "transparent hugepages: madvise", None),
        Some(t) => check(
            "hugepages",
            Warn,
            format!("transparent hugepages: {t}; `always` can cause stalls, `madvise` lets games ask"),
            Some("echo madvise | sudo tee /sys/kernel/mm/transparent_hugepage/enabled   (always: transparent_hugepage=madvise on the kernel command line)"),
        ),
        None => check("hugepages", Skip, "this kernel has no transparent hugepages", None),
    });

    // Split locks
    out.push(match i.split_lock_mitigate {
        Some(0) => check("split lock", Pass, "kernel.split_lock_mitigate = 0", None),
        Some(n) => check(
            "split lock",
            Warn,
            format!("kernel.split_lock_mitigate = {n}: games that do split locks are slowed down on purpose"),
            Some("sudo sysctl -w kernel.split_lock_mitigate=0   (always: /etc/sysctl.d/80-seglinux-gaming.conf)"),
        ),
        None => check("split lock", Skip, "this kernel has no split lock detection setting", None),
    });

    // Compaction
    out.push(match i.compaction_proactiveness {
        Some(0) => check("compaction", Pass, "vm.compaction_proactiveness = 0", None),
        Some(n) => check(
            "compaction",
            Warn,
            format!("vm.compaction_proactiveness = {n}: the kernel defragments memory in the background, which can stall a game"),
            Some("sudo sysctl -w vm.compaction_proactiveness=0   (always: /etc/sysctl.d/80-seglinux-gaming.conf)"),
        ),
        None => check("compaction", Skip, "this kernel has no vm.compaction_proactiveness", None),
    });

    // Mitigations stay on
    out.push(match &i.cmdline {
        Some(c) if c.split_whitespace().any(|w| w == "mitigations=off") => check(
            "mitigations",
            Warn,
            "mitigations=off is on the kernel command line: faster, but the CPU's protections against Spectre-style attacks are off",
            Some("sudo seg game unsafe-boost off"),
        ),
        Some(_) => check("mitigations", Pass, "the kernel's default security mitigations are on", None),
        None => check("mitigations", Skip, "cannot read /proc/cmdline", None),
    });
    out
}

/// Look at the computer. `live` allows starting programs (vulkaninfo, lspci).
pub fn collect(root: &Path, live: bool) -> Inputs {
    let read = |rel: &str| sys::read_trimmed(root.join(rel));
    let num = |rel: &str| read(rel).and_then(|t| t.parse::<u64>().ok());
    let release = read("proc/sys/kernel/osrelease");
    let mut images: Vec<String> = fs::read_dir(root.join("boot"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.starts_with("vmlinuz"))
                .collect()
        })
        .unwrap_or_default();
    images.sort();
    let status = read("proc/self/status").unwrap_or_default();
    let is_root = sys::parse_euid(&status) == Some(0);
    let groups = parse_groups(&status);
    let gamemode = read("etc/group").and_then(|g| group_gid(&g, "gamemode"));
    let vulkan = if live && sys::which("vulkaninfo").is_some() {
        sys::run("vulkaninfo", &["--summary"])
            .ok()
            .filter(|o| o.ok)
            .map(|o| parse_vulkan_devices(&o.stdout))
    } else {
        None
    };
    Inputs {
        kernel_package: hw::kernel_package(root),
        kernel_release: release,
        kernel_images: images,
        vulkan_devices: vulkan,
        gpus: hw::gpus(root),
        packages: installed_packages(&root.join("var/lib/pacman/local")),
        ntsync_device_mode: fs::metadata(root.join("dev/ntsync"))
            .ok()
            .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777),
        max_map_count: num("proc/sys/vm/max_map_count"),
        nofile: read("proc/self/limits").and_then(|l| parse_nofile(&l)),
        governor: read("sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        is_root,
        in_gamemode_group: gamemode.is_some_and(|g| groups.contains(&g)),
        boost_allowed: if live && sys::which("sudo").is_some() {
            sys::run(
                "sudo",
                &["-n", "-l", "/usr/bin/seg", "game", "boost", "off", "1"],
            )
            .ok()
            .map(|o| o.ok)
        } else {
            None
        },
        thp: read("sys/kernel/mm/transparent_hugepage/enabled"),
        split_lock_mitigate: num("proc/sys/kernel/split_lock_mitigate"),
        compaction_proactiveness: num("proc/sys/vm/compaction_proactiveness"),
        cmdline: read("proc/cmdline"),
        cpu: Some(hw::cpu(root)),
        scaling_driver: read("sys/devices/system/cpu/cpu0/cpufreq/scaling_driver"),
    }
}

pub fn label(s: Status) -> &'static str {
    match s {
        Status::Pass => "PASS",
        Status::Warn => "WARN",
        Status::Fail => "FAIL",
        Status::Skip => "SKIP",
    }
}

pub fn render(checks: &[Check]) -> String {
    let mut text = String::new();
    for c in checks {
        text.push_str(&format!(
            "[{}] {:<15} {}\n",
            label(c.status),
            c.name,
            c.detail
        ));
        if let (Status::Fail | Status::Warn, Some(fix)) = (c.status, &c.fix) {
            text.push_str(&format!("       {:<15} fix: {fix}\n", ""));
        }
    }
    let count = |s| checks.iter().filter(|c| c.status == s).count();
    text.push_str(&format!(
        "\n{} passed, {} to improve, {} failed\n",
        count(Status::Pass),
        count(Status::Warn),
        count(Status::Fail)
    ));
    text
}

pub fn run() -> Result<u8> {
    let inputs = collect(Path::new("/"), true);
    let checks = evaluate(&inputs);
    out!("{}", render(&checks));
    Ok(u8::from(checks.iter().any(|c| c.status == Status::Fail)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::hw::{fixture, gpus};

    fn pkgs(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// A machine where everything is right.
    fn good() -> Inputs {
        Inputs {
            kernel_release: Some("7.2.11-zen1-1-zen".into()),
            kernel_package: Some("linux-zen".into()),
            kernel_images: vec!["vmlinuz-linux".into(), "vmlinuz-linux-zen".into()],
            vulkan_devices: Some(vec!["AMD Radeon RX 7900 XTX (RADV NAVI31)".into()]),
            gpus: gpus(&fixture("gpu-amd")),
            packages: pkgs(&[
                "lib32-mesa",
                "lib32-vulkan-radeon",
                "lib32-vulkan-icd-loader",
                "segctl",
            ]),
            ntsync_device_mode: Some(0o666),
            max_map_count: Some(2_147_483_642),
            nofile: Some((1024, 1_048_576)),
            governor: Some("powersave".into()),
            is_root: false,
            in_gamemode_group: true,
            boost_allowed: Some(true),
            thp: Some("always [madvise] never".into()),
            split_lock_mitigate: Some(0),
            compaction_proactiveness: Some(0),
            cmdline: Some("root=/dev/sda2 quiet".into()),
            cpu: Some(hw::cpu(&fixture("ryzen-7950x3d"))),
            scaling_driver: Some("amd-pstate-epp".into()),
        }
    }

    fn status(checks: &[Check], name: &str) -> Vec<Status> {
        checks
            .iter()
            .filter(|c| c.name == name)
            .map(|c| c.status)
            .collect()
    }

    #[test]
    fn a_ready_machine_passes_everything() {
        let checks = evaluate(&good());
        for c in &checks {
            assert_eq!(c.status, Status::Pass, "{} -> {}", c.name, c.detail);
            assert!(c.fix.is_none());
        }
        assert_eq!(
            checks.len(),
            15,
            "{:?}",
            checks.iter().map(|c| c.name).collect::<Vec<_>>()
        );
        let text = render(&checks);
        assert!(text.contains("[PASS] vulkan"));
        assert!(text.contains("15 passed, 0 to improve, 0 failed"), "{text}");
    }

    #[test]
    fn software_rendering_is_a_failure_with_the_fix() {
        let mut i = good();
        i.vulkan_devices = Some(vec!["llvmpipe (LLVM 19.1.7, 256 bits)".into()]);
        let c = evaluate(&i);
        assert_eq!(status(&c, "vulkan"), [Status::Fail]);
        assert!(
            c[0].fix
                .as_deref()
                .unwrap()
                .contains("seg game setup --yes")
        );
        i.vulkan_devices = None;
        assert!(
            evaluate(&i)[0]
                .fix
                .as_deref()
                .unwrap()
                .contains("vulkan-tools")
        );
        i.vulkan_devices = Some(vec![]);
        assert_eq!(status(&evaluate(&i), "vulkan"), [Status::Fail]);
    }

    #[test]
    fn missing_32_bit_drivers_are_named() {
        let mut i = good();
        i.packages = pkgs(&["lib32-mesa"]);
        let c = evaluate(&i);
        let d = c.iter().find(|c| c.name == "32-bit drivers").unwrap();
        assert_eq!(d.status, Status::Fail);
        assert!(
            d.detail
                .contains("lib32-vulkan-icd-loader lib32-vulkan-radeon"),
            "{}",
            d.detail
        );
        assert_eq!(
            d.fix.as_deref(),
            Some("sudo pacman -S lib32-vulkan-icd-loader lib32-vulkan-radeon")
        );
    }

    #[test]
    fn nvidia_needs_its_own_driver_and_its_lib32_utils() {
        let mut i = good();
        i.gpus = gpus(&fixture("gpu-nvidia-ada-nouveau"));
        i.packages = pkgs(&[]);
        let c = evaluate(&i);
        assert_eq!(status(&c, "gpu driver"), [Status::Fail]);
        let d = c.iter().find(|c| c.name == "32-bit drivers").unwrap();
        assert!(d.detail.contains("lib32-nvidia-utils"), "{}", d.detail);
        // The proprietary driver loaded: fine.
        i.gpus = gpus(&fixture("gpu-nvidia-ada"));
        assert_eq!(status(&evaluate(&i), "gpu driver"), [Status::Pass]);
        // An old card cannot have it: nouveau is the right driver.
        i.gpus = gpus(&fixture("gpu-nvidia-pascal"));
        assert_eq!(status(&evaluate(&i), "gpu driver"), [Status::Pass]);
    }

    #[test]
    fn a_hybrid_laptop_is_checked_card_by_card() {
        let mut i = good();
        i.gpus = gpus(&fixture("gpu-hybrid-laptop"));
        i.packages = pkgs(&[
            "lib32-mesa",
            "lib32-vulkan-intel",
            "lib32-vulkan-icd-loader",
            "lib32-nvidia-utils",
        ]);
        let c = evaluate(&i);
        assert_eq!(status(&c, "gpu driver"), [Status::Pass, Status::Pass]);
        assert_eq!(status(&c, "32-bit drivers"), [Status::Pass]);
    }

    #[test]
    fn no_gpu_skips_the_gpu_questions() {
        let mut i = good();
        i.gpus = vec![];
        let c = evaluate(&i);
        assert_eq!(status(&c, "gpu driver"), [Status::Skip]);
        assert_eq!(status(&c, "32-bit drivers"), [Status::Skip]);
    }

    #[test]
    fn ntsync_has_three_ways_to_fail() {
        let mut i = good();
        i.ntsync_device_mode = None;
        let c = evaluate(&i);
        let n = c.iter().find(|c| c.name == "ntsync").unwrap();
        assert_eq!(n.status, Status::Fail);
        assert!(
            n.fix
                .as_deref()
                .unwrap()
                .starts_with("sudo modprobe ntsync")
        );
        i.ntsync_device_mode = Some(0o600);
        let n = evaluate(&i)
            .into_iter()
            .find(|c| c.name == "ntsync")
            .unwrap();
        assert!(n.fix.as_deref().unwrap().contains("chmod 666"));
        i.kernel_release = Some("6.12.4-arch1-1".into());
        let n = evaluate(&i)
            .into_iter()
            .find(|c| c.name == "ntsync")
            .unwrap();
        assert!(n.detail.contains("older than 6.14"), "{}", n.detail);
    }

    #[test]
    fn sysctls_and_limits() {
        let mut i = good();
        i.max_map_count = Some(65530);
        i.nofile = Some((1024, 4096));
        i.split_lock_mitigate = Some(1);
        i.compaction_proactiveness = Some(20);
        let c = evaluate(&i);
        assert_eq!(status(&c, "max_map_count"), [Status::Fail]);
        assert_eq!(status(&c, "open files"), [Status::Fail]);
        assert_eq!(status(&c, "split lock"), [Status::Warn]);
        assert_eq!(status(&c, "compaction"), [Status::Warn]);
        let text = render(&c);
        assert!(
            text.contains("fix: sudo sysctl -w vm.max_map_count=2147483642"),
            "{text}"
        );
        assert!(text.contains("limits.d/90-seglinux-esync.conf"), "{text}");
        // Older kernels simply lack the keys.
        i.split_lock_mitigate = None;
        i.compaction_proactiveness = None;
        let c = evaluate(&i);
        assert_eq!(status(&c, "split lock"), [Status::Skip]);
        assert_eq!(status(&c, "compaction"), [Status::Skip]);
    }

    #[test]
    fn kernels() {
        let mut i = good();
        i.kernel_release = Some("6.14.2-arch1-1".into());
        i.kernel_images = vec!["vmlinuz-linux".into()];
        let c = evaluate(&i);
        assert_eq!(status(&c, "kernel"), [Status::Warn]);
        assert_eq!(status(&c, "fallback kernel"), [Status::Warn]);
        let text = render(&c);
        assert!(
            text.contains("pacman -S linux-zen linux-zen-headers"),
            "{text}"
        );
        assert!(text.contains("grub-mkconfig"), "{text}");
    }

    #[test]
    fn amd_cpus_want_the_active_pstate_driver_and_others_are_not_asked() {
        let mut i = good();
        i.scaling_driver = Some("acpi-cpufreq".into());
        let c = evaluate(&i);
        assert_eq!(status(&c, "cpu driver"), [Status::Warn]);
        assert!(render(&c).contains("amd_pstate=active"));
        i.scaling_driver = Some("amd-pstate".into());
        assert_eq!(status(&evaluate(&i), "cpu driver"), [Status::Warn]);
        i.scaling_driver = None;
        assert_eq!(status(&evaluate(&i), "cpu driver"), [Status::Skip]);
        i.cpu = Some(hw::cpu(&fixture("intel-12700")));
        i.scaling_driver = Some("intel_pstate".into());
        assert!(status(&evaluate(&i), "cpu driver").is_empty());
    }

    #[test]
    fn the_governor_is_judged_at_rest() {
        let mut i = good();
        i.governor = Some("performance".into());
        assert_eq!(status(&evaluate(&i), "cpu governor"), [Status::Warn]);
        i.governor = None;
        assert_eq!(status(&evaluate(&i), "cpu governor"), [Status::Skip]);
    }

    #[test]
    fn who_may_boost() {
        let mut i = good();
        i.boost_allowed = Some(false);
        i.in_gamemode_group = false;
        let b = evaluate(&i)
            .into_iter()
            .find(|c| c.name == "game boost")
            .unwrap();
        assert_eq!(b.status, Status::Fail);
        assert!(
            b.fix
                .as_deref()
                .unwrap()
                .contains("gpasswd -a $USER gamemode")
        );
        // In the group and still refused: a sudoers problem.
        i.in_gamemode_group = true;
        let b = evaluate(&i)
            .into_iter()
            .find(|c| c.name == "game boost")
            .unwrap();
        assert!(
            b.fix.as_deref().unwrap().contains("visudo -c"),
            "{:?}",
            b.fix
        );
        // Not asked (not a live check): the group decides what is said.
        i.boost_allowed = None;
        assert_eq!(status(&evaluate(&i), "game boost"), [Status::Fail]);
        // No segctl, no rule.
        i.packages.remove("segctl");
        let b = evaluate(&i)
            .into_iter()
            .find(|c| c.name == "game boost")
            .unwrap();
        assert!(b.fix.as_deref().unwrap().contains("pacman -S segctl"));
        i.is_root = true;
        assert_eq!(status(&evaluate(&i), "game boost"), [Status::Pass]);
    }

    #[test]
    fn hugepages_and_mitigations() {
        let mut i = good();
        i.thp = Some("[always] madvise never".into());
        i.cmdline = Some("quiet mitigations=off".into());
        let c = evaluate(&i);
        assert_eq!(status(&c, "hugepages"), [Status::Warn]);
        assert_eq!(status(&c, "mitigations"), [Status::Warn]);
        assert!(render(&c).contains("seg game unsafe-boost off"));
        // A word that only contains the text is not the setting.
        i.cmdline = Some("quiet nomitigations=off2".into());
        assert_eq!(status(&evaluate(&i), "mitigations"), [Status::Pass]);
    }

    #[test]
    fn the_exit_status_follows_the_failures_only() {
        let mut i = good();
        i.thp = Some("[always] madvise never".into());
        assert!(!evaluate(&i).iter().any(|c| c.status == Status::Fail));
        i.max_map_count = Some(1);
        assert!(evaluate(&i).iter().any(|c| c.status == Status::Fail));
    }

    #[test]
    fn parsers() {
        let vk = "Devices:\n========\nGPU0:\n\tapiVersion         = 1.3.296\n\tdeviceName         = AMD Radeon RX 7900 XTX (RADV NAVI31)\n\tdriverName         = radv\nGPU1:\n\tdeviceName = llvmpipe (LLVM 19.1.7, 256 bits)\n";
        assert_eq!(
            parse_vulkan_devices(vk),
            [
                "AMD Radeon RX 7900 XTX (RADV NAVI31)",
                "llvmpipe (LLVM 19.1.7, 256 bits)"
            ]
        );
        let limits = "Limit                     Soft Limit           Hard Limit           Units\nMax cpu time              unlimited            unlimited            seconds\nMax open files            1024                 1048576              files\n";
        assert_eq!(parse_nofile(limits), Some((1024, 1_048_576)));
        assert_eq!(
            parse_nofile("Max open files unlimited unlimited files"),
            Some((u64::MAX, u64::MAX))
        );
        assert_eq!(parse_nofile(""), None);
        assert_eq!(
            parse_groups("Name:\tx\nGroups:\t10 1000 998\n"),
            [10, 1000, 998]
        );
        assert_eq!(parse_groups("Groups:\t\n"), Vec::<u32>::new());
        assert_eq!(
            group_gid("root:x:0:\ngamemode:x:998:alice\n", "gamemode"),
            Some(998)
        );
        assert_eq!(group_gid("root:x:0:\n", "gamemode"), None);
    }

    #[test]
    fn installed_packages_come_from_the_database_folder_names() {
        let t = sys::testutil::TempDir::new();
        for n in [
            "lib32-mesa-25.0.1-1",
            "linux-zen-headers-7.2.11.zen1-1",
            "bash-5.2-1",
            "x",
        ] {
            t.put(&format!("{n}/desc"), "");
        }
        t.put("ALPM_DB_VERSION", "9\n");
        let p = installed_packages(t.path());
        assert!(p.contains("lib32-mesa") && p.contains("linux-zen-headers") && p.contains("bash"));
        assert!(!p.contains("x"));
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn collecting_from_a_fake_root() {
        let t = sys::testutil::TempDir::new();
        t.put("proc/sys/kernel/osrelease", "7.2.11-zen1-1-zen\n");
        t.put("proc/sys/vm/max_map_count", "2147483642\n");
        t.put("proc/sys/kernel/split_lock_mitigate", "0\n");
        t.put(
            "sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
            "powersave\n",
        );
        t.put(
            "sys/kernel/mm/transparent_hugepage/enabled",
            "always [madvise] never\n",
        );
        t.put("boot/vmlinuz-linux", "");
        t.put("boot/vmlinuz-linux-zen", "");
        t.put("boot/initramfs-linux.img", "");
        t.put("proc/cmdline", "quiet\n");
        t.put(
            "proc/cpuinfo",
            "vendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 7 5800X\n",
        );
        t.put(
            "sys/devices/system/cpu/cpu0/cpufreq/scaling_driver",
            "amd-pstate-epp\n",
        );
        let i = collect(t.path(), false);
        assert_eq!(i.kernel_images, ["vmlinuz-linux", "vmlinuz-linux-zen"]);
        assert_eq!(i.max_map_count, Some(2_147_483_642));
        assert_eq!(i.split_lock_mitigate, Some(0));
        assert_eq!(i.governor.as_deref(), Some("powersave"));
        assert_eq!(i.scaling_driver.as_deref(), Some("amd-pstate-epp"));
        assert_eq!(i.cpu.map(|c| c.vendor), Some(hw::CpuVendor::Amd));
        assert_eq!(i.boost_allowed, None, "sudo is only asked in a live check");
        assert_eq!(
            i.vulkan_devices, None,
            "no programs are started when not live"
        );
        assert_eq!(i.ntsync_device_mode, None);
        assert_eq!(i.compaction_proactiveness, None);
    }
}
