//! `seg game setup`: reads the graphics cards from /sys/class/drm (and their
//! names from lspci), says which packages this computer needs for games, and
//! with `--yes` installs them. Only drivers for hardware that is there.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::check::installed_packages;
use super::hw::{self, Gpu, GpuVendor};
use crate::error::{Error, Result};
use crate::sys;

pub const HELP: &str = "\
Usage: seg game setup [--yes] [--games] [--lutris]

Finds the graphics cards in this computer (/sys/class/drm, names from lspci)
and lists the packages they need to play: the Mesa and Vulkan drivers and
their 32-bit versions for AMD and Intel, NVIDIA's open kernel driver (with
the headers of the running kernel, for DKMS) and utilities for GeForce GTX 16
and RTX cards, the open-source driver for older NVIDIA cards.

Without --yes it only prints them and the command that installs them. With
--yes (as root) it runs  pacman -S --needed --noconfirm  with that list.

  --games   also Steam, gamescope, MangoHud (64-bit: the Artix repositories
            have no 32-bit MangoHud), Wine Staging and winetricks
  --lutris  also Lutris (a big package; Steam does not need it)
";

#[derive(Debug, Default)]
pub struct Options {
    pub yes: bool,
    pub games: bool,
    pub lutris: bool,
}

pub fn parse_args(parser: &mut lexopt::Parser) -> Result<Options> {
    use lexopt::prelude::*;
    let mut o = Options::default();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("yes") | Short('y') => o.yes = true,
            Long("games") => o.games = true,
            Long("lutris") => o.lutris = true,
            Short('h') | Long("help") => {
                out!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(other.unexpected().into()),
        }
    }
    Ok(o)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// (what it is called, the packages for it)
    pub per_gpu: Vec<(String, Vec<String>)>,
    /// Everything to install, in order, without repeats.
    pub packages: Vec<String>,
    pub notes: Vec<String>,
    pub needs_nvidia_modprobe: bool,
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// The packages for one card. `kernel` is the package of the running kernel.
pub fn packages_for(gpu: &Gpu, kernel: &str) -> (Vec<String>, Option<&'static str>) {
    match gpu.vendor {
        GpuVendor::Amd => (
            strings(&["mesa", "lib32-mesa", "vulkan-radeon", "lib32-vulkan-radeon"]),
            None,
        ),
        GpuVendor::Intel => (
            strings(&["mesa", "lib32-mesa", "vulkan-intel", "lib32-vulkan-intel"]),
            None,
        ),
        GpuVendor::Nvidia if gpu.nvidia_open_capable() => (
            vec![
                format!("{kernel}-headers"),
                "nvidia-open-dkms".into(),
                "nvidia-utils".into(),
                "lib32-nvidia-utils".into(),
                "nvidia-settings".into(),
            ],
            None,
        ),
        GpuVendor::Nvidia => (
            strings(&["mesa", "lib32-mesa", "vulkan-nouveau"]),
            Some(
                "this NVIDIA card is older than Turing, which NVIDIA's current driver no longer supports; the open-source NVK driver is used",
            ),
        ),
        GpuVendor::Other => (Vec::new(), Some("no driver known for this graphics card")),
    }
}

/// `[lib32]` enabled in pacman.conf?
pub fn lib32_enabled(pacman_conf: &str) -> bool {
    pacman_conf.lines().any(|l| l.trim() == "[lib32]")
}

pub fn plan(
    gpus: &[Gpu],
    describe: &dyn Fn(&Gpu) -> String,
    kernel: Option<&str>,
    games: bool,
    lutris: bool,
) -> Plan {
    let kernel = kernel.unwrap_or("linux");
    let mut plan = Plan {
        per_gpu: Vec::new(),
        packages: Vec::new(),
        notes: Vec::new(),
        needs_nvidia_modprobe: false,
    };
    let mut seen = BTreeSet::new();
    let mut add = |plan: &mut Plan, pkg: &str| {
        if seen.insert(pkg.to_string()) {
            plan.packages.push(pkg.to_string());
        }
    };
    if gpus.is_empty() {
        plan.notes.push("no graphics card found in /sys/class/drm (a virtual machine without GPU?); no driver to install".into());
    }
    for gpu in gpus {
        let (pkgs, note) = packages_for(gpu, kernel);
        if let Some(n) = note {
            plan.notes.push(format!("{}: {n}", describe(gpu)));
        }
        if gpu.nvidia_open_capable() {
            plan.needs_nvidia_modprobe = true;
        }
        for p in &pkgs {
            add(&mut plan, p);
        }
        plan.per_gpu.push((describe(gpu), pkgs));
    }
    if !gpus.is_empty() {
        for p in [
            "vulkan-icd-loader",
            "lib32-vulkan-icd-loader",
            "vulkan-tools",
        ] {
            add(&mut plan, p);
        }
    }
    if games {
        for p in [
            "steam",
            "gamescope",
            "mangohud",
            "wine-staging",
            "winetricks",
        ] {
            add(&mut plan, p);
        }
    }
    if lutris {
        add(&mut plan, "lutris");
    }
    plan
}

pub fn render(plan: &Plan, installed: &BTreeSet<String>) -> String {
    let mut text = String::new();
    for (name, pkgs) in &plan.per_gpu {
        text.push_str(&format!(
            "{name}\n  {}\n",
            if pkgs.is_empty() {
                "(nothing)".to_string()
            } else {
                pkgs.join(" ")
            }
        ));
    }
    for n in &plan.notes {
        text.push_str(&format!("note: {n}\n"));
    }
    let todo: Vec<&String> = plan
        .packages
        .iter()
        .filter(|p| !installed.contains(*p))
        .collect();
    if plan.packages.is_empty() {
        text.push_str("nothing to install\n");
    } else if todo.is_empty() {
        text.push_str(&format!(
            "all {} packages are installed already\n",
            plan.packages.len()
        ));
    } else {
        text.push_str(&format!(
            "\nto install ({} of {}):\n  sudo pacman -S --needed {}\n",
            todo.len(),
            plan.packages.len(),
            todo.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    text
}

/// The modprobe options NVIDIA's DRM driver wants for Wayland and a
/// framebuffer console.
pub const NVIDIA_MODPROBE: &str = "# Written by seg game setup: kernel mode setting for Wayland, and the\n# framebuffer console on NVIDIA's driver.\noptions nvidia_drm modeset=1 fbdev=1\n";

/// Is a modprobe file with these options already there (the seglinux package
/// ships one)?
pub fn has_nvidia_modprobe(root: &Path) -> bool {
    ["usr/lib/modprobe.d", "etc/modprobe.d"].iter().any(|dir| {
        fs::read_dir(root.join(dir)).is_ok_and(|d| {
            d.flatten().any(|e| {
                fs::read_to_string(e.path()).is_ok_and(|t| {
                    t.lines().any(|l| {
                        l.trim().starts_with("options nvidia_drm") && l.contains("modeset=1")
                    })
                })
            })
        })
    })
}

pub fn run(o: Options) -> Result<u8> {
    let root = Path::new("/");
    let gpus = hw::gpus(root);
    let kernel = hw::kernel_package(root);
    let p = plan(&gpus, &hw::describe, kernel.as_deref(), o.games, o.lutris);
    let installed = installed_packages(&root.join("var/lib/pacman/local"));
    out!("{}", render(&p, &installed));
    let todo: Vec<&str> = p
        .packages
        .iter()
        .filter(|x| !installed.contains(*x))
        .map(String::as_str)
        .collect();
    let needs_modprobe = p.needs_nvidia_modprobe && !has_nvidia_modprobe(root);
    if !o.yes {
        if !todo.is_empty() || needs_modprobe {
            outln!("\nRun `sudo seg game setup --yes` to install them.");
        }
        return Ok(0);
    }
    sys::require_root("`seg game setup --yes`")?;
    let conf = fs::read_to_string("/etc/pacman.conf").unwrap_or_default();
    if p.packages.iter().any(|x| x.starts_with("lib32-")) && !lib32_enabled(&conf) {
        return Err(Error::other(
            "the [lib32] repository is not enabled in /etc/pacman.conf: add\n\n  [lib32]\n  Include = /etc/pacman.d/mirrorlist\n\nand run `sudo pacman -Sy`, then try again",
        ));
    }
    if !todo.is_empty() {
        let mut args = vec!["-S", "--needed", "--noconfirm"];
        args.extend(todo.iter().copied());
        sys::run_inherit("pacman", &args)?;
    }
    if needs_modprobe {
        sys::write("/etc/modprobe.d/seg-nvidia.conf", NVIDIA_MODPROBE)?;
        outln!("wrote /etc/modprobe.d/seg-nvidia.conf");
    }
    if p.needs_nvidia_modprobe {
        outln!("\nReboot to use NVIDIA's driver.");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::hw::{fixture, gpus};

    fn name(g: &Gpu) -> String {
        format!("{} {:04x}", g.card, g.device_id)
    }

    fn p(fixture_name: &str) -> Plan {
        plan(
            &gpus(&fixture(fixture_name)),
            &name,
            Some("linux-zen"),
            false,
            false,
        )
    }

    /// Every package `seg game setup` can ever pick, for the CI check that
    /// they exist in the Artix repositories (check-packages.sh -x).
    #[test]
    fn every_package_setup_can_choose_is_in_the_ci_list() {
        let mut all = BTreeSet::new();
        for card in [
            "gpu-amd",
            "gpu-intel-arc",
            "gpu-nvidia-ada",
            "gpu-nvidia-pascal",
            "gpu-hybrid-laptop",
        ] {
            let plan = plan(&gpus(&fixture(card)), &name, Some("linux-zen"), true, true);
            all.extend(plan.packages);
        }
        let mut expected = String::from(
            "# Every package `seg game setup` can choose, one per line. A test keeps this in step\n\
             # with src/game/setup.rs; CI resolves them on Artix (check-packages.sh -x).\n",
        );
        for pkg in &all {
            expected.push_str(pkg);
            expected.push('\n');
        }
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/setup-packages.txt");
        if std::env::var_os("SEG_WRITE_FIXTURES").is_some() {
            std::fs::write(&path, &expected).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            expected,
            "run the tests once with SEG_WRITE_FIXTURES=1 to rewrite {}",
            path.display()
        );
    }

    #[test]
    fn amd_gets_mesa_and_radv_in_both_bitnesses() {
        let plan = p("gpu-amd");
        assert_eq!(
            plan.packages,
            [
                "mesa",
                "lib32-mesa",
                "vulkan-radeon",
                "lib32-vulkan-radeon",
                "vulkan-icd-loader",
                "lib32-vulkan-icd-loader",
                "vulkan-tools"
            ]
        );
        assert!(!plan.needs_nvidia_modprobe);
        assert!(plan.notes.is_empty());
    }

    #[test]
    fn intel_gets_anv() {
        let plan = p("gpu-intel-arc");
        assert!(plan.packages.contains(&"vulkan-intel".to_string()));
        assert!(plan.packages.contains(&"lib32-vulkan-intel".to_string()));
        assert!(
            !plan
                .packages
                .iter()
                .any(|x| x.contains("nvidia") || x.contains("radeon"))
        );
    }

    #[test]
    fn a_turing_or_newer_nvidia_gets_the_open_dkms_driver_and_the_running_kernels_headers() {
        let plan = p("gpu-nvidia-ada-nouveau");
        assert_eq!(
            &plan.packages[..5],
            [
                "linux-zen-headers",
                "nvidia-open-dkms",
                "nvidia-utils",
                "lib32-nvidia-utils",
                "nvidia-settings"
            ]
        );
        assert!(plan.needs_nvidia_modprobe);
        // No Mesa packages the proprietary driver does not use.
        assert!(!plan.packages.contains(&"vulkan-radeon".to_string()));
    }

    #[test]
    fn an_old_nvidia_card_stays_on_nouveau_and_says_why() {
        let plan = p("gpu-nvidia-pascal");
        assert!(plan.packages.contains(&"vulkan-nouveau".to_string()));
        assert!(!plan.packages.iter().any(|x| x.contains("nvidia")));
        assert!(!plan.needs_nvidia_modprobe);
        assert!(
            plan.notes[0].contains("older than Turing"),
            "{:?}",
            plan.notes
        );
    }

    #[test]
    fn a_hybrid_laptop_gets_both_without_repeats() {
        let plan = p("gpu-hybrid-laptop");
        assert_eq!(plan.per_gpu.len(), 2);
        let count = |x: &str| plan.packages.iter().filter(|p| *p == x).count();
        assert_eq!(count("lib32-mesa"), 1);
        assert_eq!(count("vulkan-icd-loader"), 1);
        assert_eq!(count("nvidia-open-dkms"), 1);
        assert_eq!(count("vulkan-intel"), 1);
    }

    #[test]
    fn no_gpu_installs_no_driver() {
        let plan = p("gpu-none");
        assert!(plan.packages.is_empty());
        assert!(
            plan.notes[0].contains("no graphics card"),
            "{:?}",
            plan.notes
        );
        let text = render(&plan, &BTreeSet::new());
        assert!(text.contains("nothing to install"), "{text}");
        // The games stack can still be asked for.
        let with = plan_games();
        assert!(with.packages.contains(&"steam".to_string()));
    }

    fn plan_games() -> Plan {
        plan(&[], &name, None, true, true)
    }

    #[test]
    fn the_games_stack_and_lutris_are_opt_in() {
        let base = plan(&gpus(&fixture("gpu-amd")), &name, None, false, false);
        assert!(!base.packages.contains(&"steam".to_string()));
        assert!(!base.packages.contains(&"lutris".to_string()));
        let all = plan(&gpus(&fixture("gpu-amd")), &name, None, true, true);
        for pkg in [
            "steam",
            "gamescope",
            "mangohud",
            "wine-staging",
            "winetricks",
            "lutris",
        ] {
            assert!(all.packages.contains(&pkg.to_string()), "{pkg}");
        }
        // Not in the Artix repositories (CI's pacman -Sp said so): do not add
        // it back without checking.
        assert!(!all.packages.contains(&"lib32-mangohud".to_string()));
        let games_only = plan_games();
        assert_eq!(
            games_only.packages.last().map(String::as_str),
            Some("lutris")
        );
    }

    #[test]
    fn an_unknown_running_kernel_means_the_stock_headers() {
        let plan = plan(&gpus(&fixture("gpu-nvidia-ada")), &name, None, false, false);
        assert_eq!(plan.packages[0], "linux-headers");
    }

    #[test]
    fn the_report_lists_only_what_is_missing() {
        let plan = p("gpu-amd");
        let have: BTreeSet<String> = ["mesa", "lib32-mesa"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let text = render(&plan, &have);
        assert!(text.contains("to install (5 of 7)"), "{text}");
        assert!(
            text.contains("sudo pacman -S --needed vulkan-radeon lib32-vulkan-radeon"),
            "{text}"
        );
        let all: BTreeSet<String> = plan.packages.iter().cloned().collect();
        assert!(render(&plan, &all).contains("all 7 packages are installed already"));
    }

    #[test]
    fn the_lib32_repository_and_the_nvidia_options() {
        assert!(lib32_enabled("[options]\n[system]\n[lib32]\nInclude = x\n"));
        assert!(!lib32_enabled("[options]\n#[lib32]\n"));
        let t = sys::testutil::TempDir::new();
        assert!(!has_nvidia_modprobe(t.path()));
        t.put(
            "usr/lib/modprobe.d/seglinux-nvidia.conf",
            "options nvidia_drm modeset=1 fbdev=1\n",
        );
        assert!(has_nvidia_modprobe(t.path()));
        let t = sys::testutil::TempDir::new();
        t.put(
            "etc/modprobe.d/other.conf",
            "options nvidia_drm modeset=0\n",
        );
        assert!(!has_nvidia_modprobe(t.path()));
        assert!(NVIDIA_MODPROBE.contains("options nvidia_drm modeset=1 fbdev=1"));
    }
}
