//! `segrun doctor`: is this machine ready to run Windows programs?

use std::fmt;
use std::fs;
use std::path::Path;

use seglinux_core::cmd::Cmd;
use seglinux_core::which;

use crate::binfmt;
use crate::launch::has_vulkan_icd;
use crate::prefix;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub status: Status,
    pub title: String,
    pub detail: String,
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = match self.status {
            Status::Ok => "[ OK ]",
            Status::Warn => "[WARN]",
            Status::Fail => "[FAIL]",
        };
        write!(f, "{tag} {}", self.title)?;
        if !self.detail.is_empty() {
            write!(f, "\n       {}", self.detail.replace('\n', "\n       "))?;
        }
        Ok(())
    }
}

fn check(status: Status, title: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        status,
        title: title.into(),
        detail: detail.into(),
    }
}

/// Value of a line such as `Max open files  1024  524288  files` in
/// `/proc/self/limits`: returns the hard limit.
pub fn parse_nofile_hard_limit(limits: &str) -> Option<u64> {
    let line = limits.lines().find(|l| l.starts_with("Max open files"))?;
    let mut fields = line["Max open files".len()..].split_whitespace();
    let _soft = fields.next()?;
    let hard = fields.next()?;
    if hard == "unlimited" {
        Some(u64::MAX)
    } else {
        hard.parse().ok()
    }
}

pub fn run_checks(wine: &str) -> Vec<Check> {
    let mut out = Vec::new();

    match which(wine) {
        Some(path) => {
            let version = Cmd::new(path.to_string_lossy())
                .arg("--version")
                .output()
                .map(|v| v.trim().to_string())
                .unwrap_or_else(|_| "unknown version".into());
            out.push(check(
                Status::Ok,
                format!("Wine is installed ({version})"),
                "",
            ));
        }
        None => out.push(check(
            Status::Fail,
            "Wine is not installed",
            "Install it with: sudo pacman -S wine",
        )),
    }

    out.push(match which("winetricks") {
        Some(_) => check(Status::Ok, "winetricks is installed", ""),
        None => check(
            Status::Warn,
            "winetricks is missing",
            "DXVK and Microsoft runtimes cannot be added automatically: sudo pacman -S winetricks",
        ),
    });

    out.push(if has_vulkan_icd() {
        match crate::launch::detect_vulkan_api() {
            Some((0, 0)) => check(
                Status::Warn,
                "Only the software Vulkan renderer (llvmpipe) is available",
                "Install the Vulkan driver for your GPU: vulkan-intel, vulkan-radeon, vulkan-nouveau or nvidia-utils.",
            ),
            Some(v) if v < (1, 3) => check(
                Status::Warn,
                format!("The GPU supports Vulkan {}.{}, not 1.3", v.0, v.1),
                "Direct3D 9-11 games use DXVK 1.10.3; Direct3D 12 games cannot run on this GPU.",
            ),
            Some(v) => check(
                Status::Ok,
                format!("Vulkan {}.{} GPU driver (DXVK and VKD3D-Proton available)", v.0, v.1),
                "",
            ),
            None => check(Status::Ok, "A Vulkan driver is installed (DXVK/VKD3D available)", ""),
        }
    } else {
        check(
            Status::Warn,
            "No Vulkan driver found",
            "Games will fall back to OpenGL. Install vulkan-radeon, vulkan-intel or the NVIDIA driver.",
        )
    });

    out.push(if Path::new("/dev/ntsync").exists() {
        check(Status::Ok, "NTSYNC is available (fast Windows synchronisation)", "")
    } else {
        check(
            Status::Warn,
            "NTSYNC is not available",
            "Load it with `sudo modprobe ntsync` (Linux 6.14+). SegLinux loads it at boot via modules-load.d.",
        )
    });

    if let Ok(limits) = fs::read_to_string("/proc/self/limits")
        && let Some(hard) = parse_nofile_hard_limit(&limits)
    {
        out.push(if hard >= 524_288 {
            check(Status::Ok, "Open file limit is high enough for esync", "")
        } else {
            check(
                Status::Warn,
                format!("Open file limit is {hard} (esync wants 524288)"),
                "See /etc/security/limits.d/90-seglinux-esync.conf and log in again.",
            )
        });
    }

    if let Ok(v) = fs::read_to_string("/proc/sys/vm/max_map_count")
        && let Ok(n) = v.trim().parse::<u64>()
    {
        out.push(if n >= 1_048_576 {
            check(Status::Ok, "vm.max_map_count is suitable for games", "")
        } else {
            check(
                Status::Warn,
                format!("vm.max_map_count is {n}"),
                "Some games crash with low values; SegLinux sets it in /etc/sysctl.d/80-seglinux-gaming.conf.",
            )
        });
    }

    for (program, what, package) in [
        (
            "gamemoderun",
            "GameMode (CPU and GPU tuning while games run)",
            "gamemode lib32-gamemode",
        ),
        (
            "mangohud",
            "MangoHud (FPS and frame-time overlay)",
            "mangohud",
        ),
        ("gamescope", "gamescope (gaming compositor)", "gamescope"),
    ] {
        out.push(match which(program) {
            Some(_) => check(Status::Ok, format!("{what} is installed"), ""),
            None => check(
                Status::Warn,
                format!("{what} is missing"),
                format!("sudo pacman -S {package}"),
            ),
        });
    }

    if let Ok(v) = fs::read_to_string("/proc/sys/kernel/split_lock_mitigate")
        && v.trim() != "0"
    {
        out.push(check(
            Status::Warn,
            "Split lock mitigation is on",
            "It slows down some games a lot; SegLinux turns it off in /etc/sysctl.d/80-seglinux-gaming.conf.",
        ));
    }

    if let Ok(swaps) = fs::read_to_string("/proc/swaps") {
        out.push(if swaps.contains("/dev/zram") {
            check(Status::Ok, "Swap is compressed in RAM (zram)", "")
        } else {
            check(
                Status::Ok,
                "zram is not in use",
                "Games with large memory needs run more smoothly with zram swap (SegLinux Setup's default).",
            )
        });
    }

    let handlers = binfmt::mz_handlers(Path::new(binfmt::BINFMT_DIR)).unwrap_or_default();
    out.push(match handlers.iter().find(|h| h.enabled) {
        Some(h) => check(
            Status::Ok,
            format!(
                "Windows programs can be started directly (handler `{}` -> {})",
                h.name, h.interpreter
            ),
            "",
        ),
        None => check(
            Status::Warn,
            "Windows programs cannot be started with ./program.exe",
            "Enable the binfmt service or run: sudo segrun binfmt register",
        ),
    });

    match prefix::list() {
        Ok(names) if !names.is_empty() => out.push(check(
            Status::Ok,
            format!(
                "{} Windows environment(s): {}",
                names.len(),
                names.join(", ")
            ),
            "",
        )),
        _ => out.push(check(
            Status::Ok,
            "No Windows environment yet; one is created on first run",
            "",
        )),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_limits() {
        let limits = "Limit                     Soft Limit           Hard Limit           Units\n\
                      Max cpu time              unlimited            unlimited            seconds\n\
                      Max open files            1024                 524288               files\n";
        assert_eq!(parse_nofile_hard_limit(limits), Some(524_288));
        assert_eq!(
            parse_nofile_hard_limit(
                "Max open files            1024                 unlimited            files"
            ),
            Some(u64::MAX)
        );
        assert_eq!(parse_nofile_hard_limit("nothing"), None);
    }

    #[test]
    fn checks_run_without_panicking() {
        let checks = run_checks("definitely-not-wine");
        assert_eq!(checks[0].status, Status::Fail);
        assert!(
            checks[0]
                .to_string()
                .starts_with("[FAIL] Wine is not installed")
        );
    }
}
