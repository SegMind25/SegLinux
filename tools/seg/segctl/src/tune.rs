//! `seg tune light|balanced|performance`: memory and writeback sysctls, the
//! CPU governor and the I/O scheduler of every disk, applied now and saved
//! for the next boot (/etc/sysctl.d/99-seg.conf, /etc/seg/tune.conf, and the
//! `seg-tune` init service that runs `seg tune apply`).
//!
//! All paths are looked up below a root directory, `/` on a real system, so
//! the whole thing can be tested against a fake /sys and /proc.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::error::{Context, Error, Result};
use crate::sys::{self, bold};

pub const HELP: &str = "\
Usage: seg tune [light|balanced|performance|apply|status] [--dry-run]

  light         save memory and battery: aggressive reclaim, small dirty
                limits, the most frugal CPU governor
  balanced      kernel defaults for memory, a dynamic governor
  performance   keep caches, short writeback bursts, governor `performance`
  apply         apply the saved profile (the seg-tune service does this at boot)
  status        show the saved profile and the current values (default)

Also sets the I/O scheduler of every disk: none for NVMe, mq-deadline for
SSDs, bfq for spinning disks. With zram swap on, swappiness is at least 100
(compressed memory is cheaper to swap to than disk is to read from).

The profile is saved in /etc/seg/tune.conf and /etc/sysctl.d/99-seg.conf.
Needs root, except `status` and --dry-run.
";

pub const CONF: &str = "/etc/seg/tune.conf";
pub const SYSCTL_CONF: &str = "/etc/sysctl.d/99-seg.conf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Light,
    Balanced,
    Performance,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Balanced => "balanced",
            Mode::Performance => "performance",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Mode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Mode> {
        match s {
            "light" => Ok(Mode::Light),
            "balanced" => Ok(Mode::Balanced),
            "performance" => Ok(Mode::Performance),
            other => Err(Error::Usage(format!(
                "unknown profile `{other}` (light, balanced or performance)"
            ))),
        }
    }
}

struct Profile {
    swappiness: u32,
    vfs_cache_pressure: u32,
    dirty_ratio: u32,
    dirty_background_ratio: u32,
    /// Governors in order of preference; the first the CPU offers is used.
    /// (intel_pstate offers only performance and powersave, the latter being
    /// its dynamic mode.)
    governors: &'static [&'static str],
    /// Energy/performance preference of CPUs that have one (HWP, amd-pstate).
    epp: &'static str,
}

fn profile(mode: Mode) -> Profile {
    match mode {
        Mode::Light => Profile {
            swappiness: 100,
            vfs_cache_pressure: 150,
            dirty_ratio: 10,
            dirty_background_ratio: 5,
            governors: &["powersave", "conservative", "schedutil", "ondemand"],
            epp: "power",
        },
        Mode::Balanced => Profile {
            swappiness: 60,
            vfs_cache_pressure: 100,
            dirty_ratio: 20,
            dirty_background_ratio: 10,
            governors: &["schedutil", "ondemand", "powersave"],
            epp: "balance_performance",
        },
        Mode::Performance => Profile {
            swappiness: 10,
            vfs_cache_pressure: 50,
            dirty_ratio: 15,
            dirty_background_ratio: 5,
            governors: &["performance"],
            epp: "performance",
        },
    }
}

/// The sysctl keys and values of a mode.
pub fn sysctl_values(mode: Mode, zram_active: bool) -> [(&'static str, u32); 4] {
    let p = profile(mode);
    let swappiness = if zram_active {
        p.swappiness.max(100)
    } else {
        p.swappiness
    };
    [
        ("vm.swappiness", swappiness),
        ("vm.vfs_cache_pressure", p.vfs_cache_pressure),
        ("vm.dirty_ratio", p.dirty_ratio),
        ("vm.dirty_background_ratio", p.dirty_background_ratio),
    ]
}

pub fn sysctl_conf(mode: Mode, zram_active: bool) -> String {
    let mut text = format!(
        "# Written by `seg tune {mode}`; change it with `sudo seg tune <light|balanced|performance>`.\n"
    );
    for (key, value) in sysctl_values(mode, zram_active) {
        text.push_str(&format!("{key} = {value}\n"));
    }
    text
}

/// `mode = balanced` from /etc/seg/tune.conf.
pub fn parse_conf(text: &str) -> Option<Mode> {
    text.lines().find_map(|l| {
        let l = l.trim();
        if l.starts_with('#') {
            return None;
        }
        let (key, value) = l.split_once('=')?;
        (key.trim() == "mode").then(|| value.trim().parse().ok())?
    })
}

/// The first of `preferred` that `available` (space separated) contains.
pub fn pick_governor<'a>(available: &str, preferred: &[&'a str]) -> Option<&'a str> {
    let have: Vec<&str> = available.split_whitespace().collect();
    preferred.iter().copied().find(|g| have.contains(g))
}

/// `mq-deadline kyber [bfq] none` -> (all schedulers, the active one).
pub fn parse_schedulers(text: &str) -> (Vec<String>, Option<String>) {
    let mut all = Vec::new();
    let mut active = None;
    for word in text.split_whitespace() {
        match word.strip_prefix('[').and_then(|w| w.strip_suffix(']')) {
            Some(name) => {
                active = Some(name.to_string());
                all.push(name.to_string());
            }
            None => all.push(word.to_string()),
        }
    }
    (all, active)
}

/// Block devices that have no I/O scheduler worth setting.
fn skip_device(name: &str) -> bool {
    ["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd", "mtd"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// none for NVMe (and virtual disks), mq-deadline for SSDs, bfq for spinning
/// disks.
pub fn scheduler_for(name: &str, rotational: bool) -> Option<&'static str> {
    if skip_device(name) {
        return None;
    }
    Some(
        if name.starts_with("nvme") || name.starts_with("vd") || name.starts_with("xvd") {
            // NVMe schedules itself; so does the host of a virtual disk.
            "none"
        } else if rotational {
            "bfq"
        } else {
            "mq-deadline"
        },
    )
}

/// A system's files, below `root`.
pub struct Fs {
    root: PathBuf,
}

#[derive(Debug, Default)]
pub struct Report {
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
}

impl Fs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Fs { root: root.into() }
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    pub fn zram_active(&self) -> bool {
        sys::read_trimmed(self.p("/proc/swaps"))
            .is_some_and(|t| t.lines().any(|l| l.starts_with("/dev/zram")))
    }

    pub fn saved_mode(&self) -> Option<Mode> {
        sys::read_trimmed(self.p(CONF)).and_then(|t| parse_conf(&t))
    }

    fn sysctl_path(&self, key: &str) -> PathBuf {
        self.p(&format!("/proc/sys/{}", key.replace('.', "/")))
    }

    fn sysctl_now(&self, key: &str) -> Option<String> {
        sys::read_trimmed(self.sysctl_path(key))
    }

    /// Set a file's value if it differs; the report says what changed.
    fn set(
        &self,
        label: &str,
        path: &Path,
        value: &str,
        dry_run: bool,
        report: &mut Report,
    ) -> Result<()> {
        let Some(now) = sys::read_trimmed(path) else {
            report
                .warnings
                .push(format!("{label}: not available on this system"));
            return Ok(());
        };
        if now == value {
            return Ok(());
        }
        if !dry_run {
            fs::write(path, value).context(format!("setting {label} ({})", path.display()))?;
        }
        report.lines.push(format!("{label:<28}{now} -> {value}"));
        Ok(())
    }

    fn apply_sysctl(&self, mode: Mode, dry_run: bool, report: &mut Report) -> Result<()> {
        for (key, value) in sysctl_values(mode, self.zram_active()) {
            self.set(
                key,
                &self.sysctl_path(key),
                &value.to_string(),
                dry_run,
                report,
            )?;
        }
        Ok(())
    }

    fn apply_cpu(&self, mode: Mode, dry_run: bool, report: &mut Report) -> Result<()> {
        let p = profile(mode);
        let Ok(entries) = fs::read_dir(self.p("/sys/devices/system/cpu")) else {
            return Ok(());
        };
        let mut cpus: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.strip_prefix("cpu")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
            .map(|e| e.path().join("cpufreq"))
            .filter(|d| d.is_dir())
            .collect();
        cpus.sort();
        if cpus.is_empty() {
            report
                .warnings
                .push("cpu governor: this CPU has no frequency scaling (virtual machine?)".into());
            return Ok(());
        }
        let mut changed = 0;
        let mut used = None;
        for dir in &cpus {
            let available =
                sys::read_trimmed(dir.join("scaling_available_governors")).unwrap_or_default();
            let Some(governor) = pick_governor(&available, p.governors) else {
                continue;
            };
            used = Some(governor);
            let path = dir.join("scaling_governor");
            if sys::read_trimmed(&path).as_deref() != Some(governor) {
                if !dry_run {
                    fs::write(&path, governor)
                        .context(format!("setting the governor ({})", path.display()))?;
                }
                changed += 1;
            }
            // Best effort: with the performance governor the CPU refuses.
            let epp = dir.join("energy_performance_preference");
            if epp.exists() && sys::read_trimmed(&epp).as_deref() != Some(p.epp) && !dry_run {
                let _ = fs::write(&epp, p.epp);
            }
        }
        match used {
            Some(g) if changed > 0 => report.lines.push(format!(
                "{:<28}{g} on {changed} of {} CPUs",
                "cpu governor",
                cpus.len()
            )),
            Some(_) => {}
            None => report.warnings.push(format!(
                "cpu governor: none of {} is available on this CPU",
                p.governors.join(", ")
            )),
        }
        Ok(())
    }

    fn apply_io(&self, dry_run: bool, report: &mut Report) -> Result<()> {
        let Ok(entries) = fs::read_dir(self.p("/sys/block")) else {
            return Ok(());
        };
        let mut devices: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        devices.sort();
        for name in devices {
            let queue = self.p(&format!("/sys/block/{name}/queue"));
            let Some(text) = sys::read_trimmed(queue.join("scheduler")) else {
                continue;
            };
            let rotational = sys::read_trimmed(queue.join("rotational")).as_deref() != Some("0");
            let Some(want) = scheduler_for(&name, rotational) else {
                continue;
            };
            let (all, active) = parse_schedulers(&text);
            if active.as_deref() == Some(want) || text == want {
                continue;
            }
            if !all.iter().any(|s| s == want) {
                report.warnings.push(format!(
                    "{name}: scheduler {want} is not built into this kernel (has {})",
                    all.join(", ")
                ));
                continue;
            }
            if !dry_run {
                fs::write(queue.join("scheduler"), want)
                    .context(format!("setting the scheduler of {name}"))?;
            }
            report.lines.push(format!(
                "{:<28}{} -> {want}",
                format!("{name} scheduler"),
                active.as_deref().unwrap_or("?")
            ));
        }
        Ok(())
    }

    /// Apply `mode` now; unless `dry_run`, also save it for the next boot.
    pub fn apply(&self, mode: Mode, save: bool, dry_run: bool) -> Result<Report> {
        let mut report = Report::default();
        self.apply_sysctl(mode, dry_run, &mut report)?;
        self.apply_cpu(mode, dry_run, &mut report)?;
        self.apply_io(dry_run, &mut report)?;
        if save && !dry_run {
            sys::write_atomic(&self.p(SYSCTL_CONF), &sysctl_conf(mode, self.zram_active()))?;
            sys::write_atomic(
                &self.p(CONF),
                &format!("# seg tune profile\nmode = {mode}\n"),
            )?;
            report.lines.push(format!("saved {CONF} and {SYSCTL_CONF}"));
        }
        Ok(report)
    }

    /// The memory sysctls again for the saved profile: zram swap coming or
    /// going changes the swappiness it wants. Does nothing without a profile.
    pub fn reapply_memory(&self) -> Result<()> {
        if let Some(mode) = self.saved_mode() {
            let mut report = Report::default();
            self.apply_sysctl(mode, false, &mut report)?;
        }
        Ok(())
    }

    pub fn status(&self) -> String {
        let mut out = String::new();
        match self.saved_mode() {
            Some(mode) => out.push_str(&format!("profile: {mode}\n")),
            None => out.push_str("profile: none saved (`sudo seg tune balanced` saves one)\n"),
        }
        for (key, _) in sysctl_values(Mode::Balanced, false) {
            out.push_str(&format!(
                "  {key:<28}{}\n",
                self.sysctl_now(key).unwrap_or_else(|| "?".into())
            ));
        }
        let governor =
            sys::read_trimmed(self.p("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"));
        out.push_str(&format!(
            "  {:<28}{}\n",
            "cpu governor",
            governor.unwrap_or_else(|| "- (no frequency scaling)".into())
        ));
        if let Ok(entries) = fs::read_dir(self.p("/sys/block")) {
            let mut devices: Vec<String> = entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !skip_device(n))
                .collect();
            devices.sort();
            for name in devices {
                if let Some(text) =
                    sys::read_trimmed(self.p(&format!("/sys/block/{name}/queue/scheduler")))
                {
                    let (_, active) = parse_schedulers(&text);
                    out.push_str(&format!(
                        "  {:<28}{}\n",
                        format!("{name} scheduler"),
                        active.unwrap_or_else(|| "?".into())
                    ));
                }
            }
        }
        out.push_str(&format!(
            "  {:<28}{}\n",
            "zram swap",
            if self.zram_active() { "on" } else { "off" }
        ));
        out
    }
}

pub enum Command {
    Set(Mode),
    Apply,
    Status,
}

pub struct Options {
    pub command: Command,
    pub dry_run: bool,
}

pub fn parse_args(parser: &mut lexopt::Parser) -> Result<Options> {
    use lexopt::prelude::*;
    let mut command = None;
    let mut dry_run = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("dry-run") => dry_run = true,
            Short('h') | Long("help") => {
                out!("{HELP}");
                std::process::exit(0);
            }
            Value(v) if command.is_none() => {
                let v = v.string()?;
                command = Some(match v.as_str() {
                    "apply" => Command::Apply,
                    "status" => Command::Status,
                    other => Command::Set(other.parse()?),
                });
            }
            other => return Err(other.unexpected().into()),
        }
    }
    Ok(Options {
        command: command.unwrap_or(Command::Status),
        dry_run,
    })
}

fn print_report(report: &Report) {
    for line in &report.lines {
        outln!("  {line}");
    }
    for warning in &report.warnings {
        outln!("  note: {warning}");
    }
}

pub fn run(o: Options) -> Result<()> {
    let fs = Fs::new("/");
    match o.command {
        Command::Status => {
            out!("{}", fs.status());
            Ok(())
        }
        Command::Set(mode) => {
            if !o.dry_run {
                sys::require_root(&format!("`seg tune {mode}`"))?;
            }
            outln!(
                "{}",
                bold(&format!(
                    "seg tune {mode}{}",
                    if o.dry_run { " (dry run)" } else { "" }
                ))
            );
            let report = fs.apply(mode, true, o.dry_run)?;
            if report.lines.is_empty() && report.warnings.is_empty() {
                outln!("  already set");
            }
            print_report(&report);
            Ok(())
        }
        Command::Apply => {
            if !o.dry_run {
                sys::require_root("`seg tune apply`")?;
            }
            // Nothing saved is fine: the service is enabled in the image
            // before anyone has chosen a profile.
            let Some(mode) = fs.saved_mode() else {
                outln!("seg tune: no profile saved ({CONF}); nothing to apply");
                return Ok(());
            };
            outln!("{}", bold(&format!("seg tune apply ({mode})")));
            print_report(&fs.apply(mode, false, o.dry_run)?);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    #[test]
    fn modes_parse_and_print() {
        for (text, mode) in [
            ("light", Mode::Light),
            ("balanced", Mode::Balanced),
            ("performance", Mode::Performance),
        ] {
            assert_eq!(text.parse::<Mode>().unwrap(), mode);
            assert_eq!(mode.to_string(), text);
        }
        let e = "turbo".parse::<Mode>().unwrap_err();
        assert!(matches!(e, Error::Usage(_)));
        assert!(e.to_string().contains("light, balanced or performance"));
    }

    #[test]
    fn profiles_are_ordered_from_frugal_to_fast() {
        let swap = |m| sysctl_values(m, false)[0].1;
        assert!(swap(Mode::Light) > swap(Mode::Balanced));
        assert!(swap(Mode::Balanced) > swap(Mode::Performance));
        let vfs = |m| sysctl_values(m, false)[1].1;
        assert!(
            vfs(Mode::Light) > vfs(Mode::Balanced) && vfs(Mode::Balanced) > vfs(Mode::Performance)
        );
    }

    #[test]
    fn zram_raises_swappiness_to_at_least_100() {
        assert_eq!(
            sysctl_values(Mode::Performance, true)[0],
            ("vm.swappiness", 100)
        );
        assert_eq!(
            sysctl_values(Mode::Balanced, true)[0],
            ("vm.swappiness", 100)
        );
        assert_eq!(
            sysctl_values(Mode::Performance, false)[0],
            ("vm.swappiness", 10)
        );
        // Never lowered.
        assert_eq!(sysctl_values(Mode::Light, true)[0].1, 100);
    }

    #[test]
    fn sysctl_file_is_plain_key_value() {
        let text = sysctl_conf(Mode::Balanced, false);
        assert!(text.starts_with('#'));
        assert!(text.contains("\nvm.swappiness = 60\n"));
        assert!(text.contains("\nvm.vfs_cache_pressure = 100\n"));
        assert!(text.contains("\nvm.dirty_ratio = 20\n"));
        assert!(text.contains("\nvm.dirty_background_ratio = 10\n"));
    }

    #[test]
    fn the_shipped_defaults_are_the_balanced_profile() {
        // dist/ is what the package installs as /etc/sysctl.d/99-seg.conf and
        // /etc/seg/tune.conf: it must say what `seg tune balanced` writes.
        assert_eq!(
            include_str!("../../dist/99-seg.conf"),
            sysctl_conf(Mode::Balanced, false)
        );
        assert_eq!(
            parse_conf(include_str!("../../dist/tune.conf")),
            Some(Mode::Balanced)
        );
    }

    #[test]
    fn saved_mode_file() {
        assert_eq!(
            parse_conf("# seg tune profile\nmode = performance\n"),
            Some(Mode::Performance)
        );
        assert_eq!(parse_conf("mode=light"), Some(Mode::Light));
        assert_eq!(parse_conf("mode = turbo\n"), None);
        assert_eq!(parse_conf("# mode = light\n"), None);
        assert_eq!(parse_conf(""), None);
    }

    #[test]
    fn governors_follow_the_preference_order() {
        let modern = "performance powersave";
        let acpi = "conservative ondemand userspace powersave performance schedutil";
        assert_eq!(
            pick_governor(modern, profile(Mode::Balanced).governors),
            Some("powersave")
        );
        assert_eq!(
            pick_governor(modern, profile(Mode::Performance).governors),
            Some("performance")
        );
        assert_eq!(
            pick_governor(acpi, profile(Mode::Balanced).governors),
            Some("schedutil")
        );
        assert_eq!(
            pick_governor(acpi, profile(Mode::Light).governors),
            Some("powersave")
        );
        assert_eq!(
            pick_governor("userspace", profile(Mode::Performance).governors),
            None
        );
        assert_eq!(pick_governor("", profile(Mode::Light).governors), None);
    }

    #[test]
    fn scheduler_lists() {
        assert_eq!(
            parse_schedulers("mq-deadline kyber [bfq] none"),
            (
                vec![
                    "mq-deadline".into(),
                    "kyber".into(),
                    "bfq".into(),
                    "none".into()
                ],
                Some("bfq".into())
            )
        );
        assert_eq!(parse_schedulers("none"), (vec!["none".into()], None));
        assert_eq!(parse_schedulers(""), (vec![], None));
    }

    #[test]
    fn scheduler_by_disk_type() {
        assert_eq!(scheduler_for("nvme0n1", false), Some("none"));
        assert_eq!(scheduler_for("sda", false), Some("mq-deadline"));
        assert_eq!(scheduler_for("sdb", true), Some("bfq"));
        assert_eq!(scheduler_for("mmcblk0", false), Some("mq-deadline"));
        // Virtual disks claim to spin; their host does the scheduling.
        assert_eq!(scheduler_for("vda", true), Some("none"));
        assert_eq!(scheduler_for("xvda", true), Some("none"));
        for skipped in ["loop0", "zram0", "dm-0", "md127", "sr0", "ram0", "nbd0"] {
            assert_eq!(scheduler_for(skipped, true), None, "{skipped}");
        }
    }

    /// A machine with two CPUs, an NVMe drive, an SSD, a hard disk and a loop
    /// device.
    fn fake_system() -> TempDir {
        let t = TempDir::new();
        for cpu in ["cpu0", "cpu1"] {
            let d = format!("sys/devices/system/cpu/{cpu}/cpufreq");
            t.put(
                &format!("{d}/scaling_available_governors"),
                "performance schedutil powersave\n",
            );
            t.put(&format!("{d}/scaling_governor"), "powersave\n");
            t.put(
                &format!("{d}/energy_performance_preference"),
                "balance_performance\n",
            );
        }
        t.put("sys/devices/system/cpu/cpufreq/policy0/x", "");
        t.put("sys/devices/system/cpu/online", "0-1\n");
        t.put("sys/block/nvme0n1/queue/scheduler", "[none] mq-deadline\n");
        t.put("sys/block/nvme0n1/queue/rotational", "0\n");
        t.put(
            "sys/block/sda/queue/scheduler",
            "[none] mq-deadline kyber bfq\n",
        );
        t.put("sys/block/sda/queue/rotational", "0\n");
        t.put(
            "sys/block/sdb/queue/scheduler",
            "mq-deadline [none] kyber bfq\n",
        );
        t.put("sys/block/sdb/queue/rotational", "1\n");
        t.put("sys/block/loop0/queue/scheduler", "[none]\n");
        t.put("proc/sys/vm/swappiness", "60\n");
        t.put("proc/sys/vm/vfs_cache_pressure", "100\n");
        t.put("proc/sys/vm/dirty_ratio", "20\n");
        t.put("proc/sys/vm/dirty_background_ratio", "10\n");
        t.put("proc/swaps", "Filename Type Size Used Priority\n");
        t
    }

    fn read(t: &TempDir, rel: &str) -> String {
        fs::read_to_string(t.path().join(rel))
            .unwrap()
            .trim()
            .to_string()
    }

    #[test]
    fn performance_changes_everything_and_saves_it() {
        let t = fake_system();
        let report = Fs::new(t.path())
            .apply(Mode::Performance, true, false)
            .unwrap();
        assert_eq!(read(&t, "proc/sys/vm/swappiness"), "10");
        assert_eq!(read(&t, "proc/sys/vm/vfs_cache_pressure"), "50");
        assert_eq!(read(&t, "proc/sys/vm/dirty_ratio"), "15");
        assert_eq!(read(&t, "proc/sys/vm/dirty_background_ratio"), "5");
        for cpu in ["cpu0", "cpu1"] {
            assert_eq!(
                read(
                    &t,
                    &format!("sys/devices/system/cpu/{cpu}/cpufreq/scaling_governor")
                ),
                "performance"
            );
            assert_eq!(
                read(
                    &t,
                    &format!("sys/devices/system/cpu/{cpu}/cpufreq/energy_performance_preference")
                ),
                "performance"
            );
        }
        // NVMe already had none; the SSD and the hard disk change.
        assert_eq!(
            read(&t, "sys/block/nvme0n1/queue/scheduler"),
            "[none] mq-deadline"
        );
        assert_eq!(read(&t, "sys/block/sda/queue/scheduler"), "mq-deadline");
        assert_eq!(read(&t, "sys/block/sdb/queue/scheduler"), "bfq");
        assert_eq!(read(&t, "sys/block/loop0/queue/scheduler"), "[none]");
        assert!(
            report
                .lines
                .iter()
                .any(|l| l.contains("performance on 2 of 2 CPUs"))
        );
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        assert_eq!(
            read(&t, "etc/seg/tune.conf").lines().last(),
            Some("mode = performance")
        );
        let sysctl = read(&t, "etc/sysctl.d/99-seg.conf");
        assert!(sysctl.contains("vm.swappiness = 10"));
        assert_eq!(Fs::new(t.path()).saved_mode(), Some(Mode::Performance));
    }

    #[test]
    fn applying_twice_changes_nothing_the_second_time() {
        let t = fake_system();
        let fs_ = Fs::new(t.path());
        fs_.apply(Mode::Balanced, true, false).unwrap();
        let again = fs_.apply(Mode::Balanced, false, false).unwrap();
        assert!(again.lines.is_empty(), "{:?}", again.lines);
    }

    #[test]
    fn a_dry_run_touches_nothing() {
        let t = fake_system();
        let report = Fs::new(t.path()).apply(Mode::Light, true, true).unwrap();
        assert!(!report.lines.is_empty());
        assert_eq!(read(&t, "proc/sys/vm/swappiness"), "60");
        assert_eq!(
            read(&t, "sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
            "powersave"
        );
        assert_eq!(
            read(&t, "sys/block/sdb/queue/scheduler"),
            "mq-deadline [none] kyber bfq"
        );
        assert!(!t.path().join("etc/seg/tune.conf").exists());
        assert!(!t.path().join("etc/sysctl.d/99-seg.conf").exists());
    }

    #[test]
    fn zram_swap_lifts_swappiness() {
        let t = fake_system();
        t.put(
            "proc/swaps",
            "Filename Type Size Used Priority\n/dev/zram0 partition 4194300 0 100\n",
        );
        let fs_ = Fs::new(t.path());
        assert!(fs_.zram_active());
        fs_.apply(Mode::Performance, true, false).unwrap();
        assert_eq!(read(&t, "proc/sys/vm/swappiness"), "100");
        assert!(read(&t, "etc/sysctl.d/99-seg.conf").contains("vm.swappiness = 100"));
        // zram goes away: the saved profile's own value comes back.
        t.put("proc/swaps", "Filename Type Size Used Priority\n");
        fs_.reapply_memory().unwrap();
        assert_eq!(read(&t, "proc/sys/vm/swappiness"), "10");
    }

    #[test]
    fn missing_pieces_are_notes_not_failures() {
        // A virtual machine: no cpufreq, a disk without bfq, no vm sysctl.
        let t = TempDir::new();
        t.put("sys/devices/system/cpu/cpu0/online", "1\n");
        t.put("sys/block/sdz/queue/scheduler", "[none] mq-deadline\n");
        t.put("sys/block/sdz/queue/rotational", "1\n");
        let report = Fs::new(t.path()).apply(Mode::Light, false, false).unwrap();
        let notes = report.warnings.join("\n");
        assert!(notes.contains("no frequency scaling"), "{notes}");
        assert!(
            notes.contains("sdz: scheduler bfq is not built into this kernel"),
            "{notes}"
        );
        assert!(notes.contains("vm.swappiness: not available"), "{notes}");
    }

    #[test]
    fn reapply_without_a_profile_does_nothing() {
        let t = fake_system();
        Fs::new(t.path()).reapply_memory().unwrap();
        assert_eq!(read(&t, "proc/sys/vm/swappiness"), "60");
    }

    #[test]
    fn status_lists_the_current_values() {
        let t = fake_system();
        let text = Fs::new(t.path()).status();
        assert!(text.contains("profile: none saved"));
        assert!(text.contains("vm.swappiness"));
        assert!(text.contains("nvme0n1 scheduler"));
        assert!(!text.contains("loop0"));
        assert!(text.contains("zram swap") && text.contains("off"));
    }
}
