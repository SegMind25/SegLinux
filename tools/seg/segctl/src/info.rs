//! `seg info`: a fast fetch-style summary. Reads /proc and /etc directly;
//! the only program it starts is `df`.

use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::init;
use crate::sys::{self, bold, human_bytes, human_duration};

/// `PRETTY_NAME` (else `NAME`) from os-release text.
pub fn parse_os_release(text: &str) -> Option<String> {
    let value = |key: &str| {
        text.lines().find_map(|l| {
            let v = l.strip_prefix(key)?.strip_prefix('=')?;
            Some(v.trim().trim_matches('"').trim_matches('\'').to_string())
        })
    };
    value("PRETTY_NAME")
        .or_else(|| value("NAME"))
        .filter(|s| !s.is_empty())
}

/// Seconds up, from /proc/uptime (`12345.67 54321.00`).
pub fn parse_uptime(text: &str) -> Option<u64> {
    let secs: f64 = text.split_whitespace().next()?.parse().ok()?;
    (secs >= 0.0).then_some(secs as u64)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Memory {
    pub total_kib: u64,
    pub available_kib: u64,
}

impl Memory {
    pub fn used_kib(&self) -> u64 {
        self.total_kib.saturating_sub(self.available_kib)
    }
}

pub fn parse_meminfo(text: &str) -> Option<Memory> {
    let field = |key: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    Some(Memory {
        total_kib: field("MemTotal:")?,
        available_kib: field("MemAvailable:").or_else(|| field("MemFree:"))?,
    })
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DfRow {
    pub filesystem: String,
    pub size_kib: u64,
    pub used_kib: u64,
    pub avail_kib: u64,
    pub use_percent: u8,
    pub mount: String,
}

/// `df -Pk` output (POSIX format: one line per file system).
pub fn parse_df(text: &str) -> Vec<DfRow> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let filesystem = f.next()?.to_string();
            let size_kib = f.next()?.parse().ok()?;
            let used_kib = f.next()?.parse().ok()?;
            let avail_kib = f.next()?.parse().ok()?;
            let use_percent = f.next()?.trim_end_matches('%').parse().ok()?;
            let mount = f.collect::<Vec<_>>().join(" ");
            (!mount.is_empty()).then_some(DfRow {
                filesystem,
                size_kib,
                used_kib,
                avail_kib,
                use_percent,
                mount,
            })
        })
        .collect()
}

pub fn df() -> Vec<DfRow> {
    sys::run("df", &["-Pk"])
        .map(|o| parse_df(&o.stdout))
        .unwrap_or_default()
}

/// Installed packages: one directory per package in pacman's local database.
pub fn count_packages(local_db: &Path) -> Option<usize> {
    let entries = fs::read_dir(local_db).ok()?;
    Some(entries.flatten().filter(|e| e.path().is_dir()).count())
}

pub fn run() -> Result<()> {
    let distro = sys::read_trimmed("/etc/os-release")
        .or_else(|| sys::read_trimmed("/usr/lib/os-release"))
        .and_then(|t| parse_os_release(&t))
        .unwrap_or_else(|| "Linux".into());
    let kernel = sys::read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_else(|| "?".into());
    let init = init::detect_running().map_or("unknown", |i| i.name());
    let uptime = sys::read_trimmed("/proc/uptime")
        .and_then(|t| parse_uptime(&t))
        .map_or_else(|| "?".into(), human_duration);
    let memory = sys::read_trimmed("/proc/meminfo")
        .and_then(|t| parse_meminfo(&t))
        .map_or_else(
            || "?".into(),
            |m| {
                let pct = m.used_kib() * 100 / m.total_kib.max(1);
                format!(
                    "{} / {} ({pct}%)",
                    human_bytes(m.used_kib() * 1024),
                    human_bytes(m.total_kib * 1024)
                )
            },
        );
    let packages = count_packages(Path::new("/var/lib/pacman/local"))
        .map_or_else(|| "?".into(), |n| format!("{n} (pacman)"));
    let disk = df().into_iter().find(|r| r.mount == "/").map_or_else(
        || "?".into(),
        |r| {
            format!(
                "{} / {} ({}%)",
                human_bytes(r.used_kib * 1024),
                human_bytes(r.size_kib * 1024),
                r.use_percent
            )
        },
    );

    outln!("{}", bold(&distro));
    for (label, value) in [
        ("Kernel", kernel),
        ("Init", init.to_string()),
        ("Uptime", uptime),
        ("Memory", memory),
        ("Packages", packages),
        ("Disk /", disk),
    ] {
        outln!("  {label:<9}{value}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    #[test]
    fn os_release_prefers_the_pretty_name() {
        let t = "NAME=\"SegLinux\"\nPRETTY_NAME=\"SegLinux (Artix)\"\nID=seglinux\n";
        assert_eq!(parse_os_release(t), Some("SegLinux (Artix)".into()));
        assert_eq!(
            parse_os_release("NAME='Artix Linux'\n"),
            Some("Artix Linux".into())
        );
        assert_eq!(parse_os_release("ID=x\n"), None);
        assert_eq!(parse_os_release("PRETTY_NAME=\"\"\n"), None);
    }

    #[test]
    fn uptime_and_memory() {
        assert_eq!(parse_uptime("8100.52 30000.10\n"), Some(8100));
        assert_eq!(parse_uptime(""), None);
        assert_eq!(parse_uptime("-1 2"), None);

        let m = parse_meminfo(
            "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:   12000000 kB\n",
        )
        .unwrap();
        assert_eq!(m.used_kib(), 4_000_000);
        // Old kernels have no MemAvailable.
        let m = parse_meminfo("MemTotal: 100 kB\nMemFree: 40 kB\n").unwrap();
        assert_eq!(m.used_kib(), 60);
        assert_eq!(parse_meminfo("MemTotal: 100 kB\n"), None);
    }

    #[test]
    fn df_rows() {
        let text = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
                    /dev/nvme0n1p2   244813824  21474836 210000000       9% /\n\
                    tmpfs              8000000         0   8000000       0% /dev/shm\n\
                    /dev/sdb1          1000000    500000    500000      50% /mnt/My Disk\n\
                    garbage line\n";
        let rows = parse_df(text);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].mount, "/");
        assert_eq!(rows[0].use_percent, 9);
        assert_eq!(rows[0].avail_kib, 210_000_000);
        assert_eq!(rows[2].mount, "/mnt/My Disk");
        assert!(parse_df("").is_empty());
    }

    #[test]
    fn package_count_counts_directories() {
        let t = TempDir::new();
        t.put("local/bash-5.2-1/desc", "");
        t.put("local/glibc-2.40-1/desc", "");
        t.put("local/ALPM_DB_VERSION", "9\n");
        assert_eq!(count_packages(&t.path().join("local")), Some(2));
        assert_eq!(count_packages(&t.path().join("missing")), None);
    }
}
