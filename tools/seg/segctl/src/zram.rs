//! `seg zram on|off|status`: compressed swap in memory, without systemd.
//! zstd, half the RAM (at most 8 GiB: the table of a larger device costs
//! memory of its own), priority 100 so it is used before any disk swap.

use std::fs;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::sys::{self, human_bytes};
use crate::tune;

pub const HELP: &str = "\
Usage: seg zram on|off|status [--size SIZE] [--if-unconfigured]

  on       set up /dev/zram0 as swap: zstd, half of the RAM (at most 8G)
  off      stop using it and free its memory
  status   show whether it is on, its size and how well it compresses

--size SIZE        size of the device instead of half the RAM: 2G, 4096M, 512M
--if-unconfigured  do nothing when /dev/zram0 already has a size (set by a
                   udev rule: used by SegLinux's own rule, which runs at boot)

`on` and `off` need root. The seg-zram service runs `seg zram on` at boot.
";

const MAX_AUTO_BYTES: u64 = 8 << 30;
const DEVICE: &str = "/dev/zram0";

pub enum Action {
    On,
    Off,
    Status,
}

pub struct Options {
    pub action: Action,
    pub on: OnOptions,
}

#[derive(Default)]
pub struct OnOptions {
    /// Size of the device instead of half the RAM.
    pub size: Option<u64>,
    /// Do nothing when /dev/zram0 has been given a size already: udev rules
    /// (SegLinux Setup writes one) and fstab set it up in that case.
    pub if_unconfigured: bool,
}

/// `2G`, `4096M`, `512m`, `1048576` (bytes) -> bytes.
pub fn parse_size(text: &str) -> Result<u64> {
    let bad = || Error::Usage(format!("`{text}` is not a size (try 2G or 4096M)"));
    let text = text.trim();
    let (digits, shift) = match text.chars().last() {
        Some('G' | 'g') => (&text[..text.len() - 1], 30),
        Some('M' | 'm') => (&text[..text.len() - 1], 20),
        Some('K' | 'k') => (&text[..text.len() - 1], 10),
        _ => (text, 0),
    };
    let n: u64 = digits.parse().map_err(|_| bad())?;
    let bytes = n.checked_mul(1 << shift).ok_or_else(bad)?;
    if bytes == 0 {
        return Err(bad());
    }
    Ok(bytes)
}

/// Half the RAM, at most 8 GiB.
pub fn auto_size(mem_total_kib: u64) -> u64 {
    (mem_total_kib.saturating_mul(1024) / 2).min(MAX_AUTO_BYTES)
}

pub fn mem_total_kib(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find(|l| l.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[derive(Debug, PartialEq, Eq)]
pub struct SwapEntry {
    pub path: String,
    pub size_kib: u64,
    pub used_kib: u64,
    pub priority: i64,
}

/// /proc/swaps: `Filename Type Size Used Priority`, sizes in KiB.
pub fn parse_swaps(text: &str) -> Vec<SwapEntry> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let path = f.next()?.to_string();
            let _kind = f.next()?;
            Some(SwapEntry {
                path,
                size_kib: f.next()?.parse().ok()?,
                used_kib: f.next()?.parse().ok()?,
                priority: f.next()?.parse().ok()?,
            })
        })
        .collect()
}

/// `lzo lzo-rle [lz4] zstd` -> (available, selected).
pub fn parse_algorithms(text: &str) -> (Vec<String>, Option<String>) {
    let mut all = Vec::new();
    let mut selected = None;
    for w in text.split_whitespace() {
        match w.strip_prefix('[').and_then(|w| w.strip_suffix(']')) {
            Some(name) => {
                selected = Some(name.to_string());
                all.push(name.to_string());
            }
            None => all.push(w.to_string()),
        }
    }
    (all, selected)
}

/// The first three numbers of mm_stat: original size, compressed size and
/// the memory used, in bytes.
pub fn parse_mm_stat(text: &str) -> Option<(u64, u64, u64)> {
    let mut f = text.split_whitespace().map(|w| w.parse::<u64>());
    Some((f.next()?.ok()?, f.next()?.ok()?, f.next()?.ok()?))
}

/// Runs the helper programs; replaced by a recorder in tests.
pub trait Runner {
    fn run(&mut self, program: &str, args: &[&str]) -> Result<()>;
}

pub struct RealRunner;

impl Runner for RealRunner {
    fn run(&mut self, program: &str, args: &[&str]) -> Result<()> {
        sys::run_ok(program, args).map(|_| ())
    }
}

pub struct Zram {
    root: PathBuf,
}

impl Zram {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Zram { root: root.into() }
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    fn dev(&self, file: &str) -> PathBuf {
        self.p(&format!("/sys/block/zram0/{file}"))
    }

    fn swap(&self) -> Option<SwapEntry> {
        let text = fs::read_to_string(self.p("/proc/swaps")).ok()?;
        parse_swaps(&text).into_iter().find(|e| e.path == DEVICE)
    }

    /// The module provides /sys/block/zram0; load it when it is missing.
    fn ensure_device(&self, runner: &mut dyn Runner, wait: Duration) -> Result<()> {
        if self.dev("disksize").exists() {
            return Ok(());
        }
        runner
            .run("modprobe", &["zram"])
            .map_err(|e| Error::Unsupported(format!("this kernel has no zram module ({e})")))?;
        for _ in 0..20 {
            if self.dev("disksize").exists() {
                return Ok(());
            }
            sleep(wait);
        }
        Err(Error::Unsupported(
            "the zram module is loaded but /sys/block/zram0 did not appear".into(),
        ))
    }

    pub fn on(&self, runner: &mut dyn Runner, opts: &OnOptions, wait: Duration) -> Result<String> {
        if let Some(swap) = self.swap() {
            return Ok(format!(
                "zram swap is already on ({})",
                human_bytes(swap.size_kib * 1024)
            ));
        }
        self.ensure_device(runner, wait)?;
        let configured = sys::read_trimmed(self.dev("disksize")).is_some_and(|s| s != "0");
        if configured && opts.if_unconfigured {
            return Ok("zram0 is set up by another rule already; leaving it".into());
        }
        let mut note = String::new();
        let mut bytes = sys::read_trimmed(self.dev("disksize"))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        // A device somebody sized keeps its size (and compression) unless a
        // size is asked for; an unused one must be reset to change.
        if !configured || opts.size.is_some() {
            bytes = match opts.size {
                Some(b) => b,
                None => {
                    let meminfo = sys::read(self.p("/proc/meminfo"))?;
                    auto_size(mem_total_kib(&meminfo).ok_or_else(|| {
                        Error::other("cannot read the memory size from /proc/meminfo")
                    })?)
                }
            };
            if configured {
                sys::write(self.dev("reset"), "1")?;
            }
            let algorithms = sys::read_trimmed(self.dev("comp_algorithm")).unwrap_or_default();
            if parse_algorithms(&algorithms).0.iter().any(|a| a == "zstd") {
                sys::write(self.dev("comp_algorithm"), "zstd")?;
            } else {
                note = " (zstd is not available: using the kernel's default compression)".into();
            }
            sys::write(self.dev("disksize"), &bytes.to_string())?;
        }
        runner.run("mkswap", &["-U", "clear", DEVICE])?;
        runner.run("swapon", &["-p", "100", DEVICE])?;

        // zswap would compress the same pages a second time on their way here.
        let zswap = self.p("/sys/module/zswap/parameters/enabled");
        if zswap.exists() {
            let _ = fs::write(zswap, "0");
        }
        // Swapping to memory needs no read-ahead.
        let _ = fs::write(self.p("/proc/sys/vm/page-cluster"), "0");
        // A saved `seg tune` profile decides the swappiness; without one,
        // swap to memory early, as Setup's 90-seglinux-zram.conf does.
        let tuned = tune::Fs::new(&self.root);
        if tuned.saved_mode().is_some() {
            tuned.reapply_memory()?;
        } else {
            let _ = fs::write(
                self.p("/proc/sys/vm/swappiness"),
                tune::ZRAM_SWAPPINESS.to_string(),
            );
        }
        Ok(format!(
            "zram swap is on: {} of compressed memory{note}",
            human_bytes(bytes)
        ))
    }

    pub fn off(&self, runner: &mut dyn Runner) -> Result<String> {
        let was_on = self.swap().is_some();
        if was_on {
            runner.run("swapoff", &[DEVICE])?;
        }
        if self.dev("reset").exists()
            && sys::read_trimmed(self.dev("disksize")).is_some_and(|s| s != "0")
        {
            sys::write(self.dev("reset"), "1")?;
        }
        if was_on {
            // Without zram the saved profile's own swappiness applies again.
            tune::Fs::new(&self.root).reapply_memory()?;
            Ok("zram swap is off and its memory is freed".into())
        } else {
            Ok("zram swap is already off".into())
        }
    }

    pub fn status(&self) -> String {
        if !self.dev("disksize").exists() {
            return "zram: off (the module is not loaded; `sudo seg zram on` loads it)\n".into();
        }
        let Some(swap) = self.swap() else {
            return "zram: off\n".into();
        };
        let mut out = format!(
            "zram: on\n  device      {DEVICE}\n  size        {}\n  swap used   {}\n  priority    {}\n",
            human_bytes(swap.size_kib * 1024),
            human_bytes(swap.used_kib * 1024),
            swap.priority
        );
        if let Some(text) = sys::read_trimmed(self.dev("comp_algorithm")) {
            out.push_str(&format!(
                "  compression {}\n",
                parse_algorithms(&text).1.unwrap_or_else(|| "?".into())
            ));
        }
        if let Some((orig, compressed, memory)) =
            sys::read_trimmed(self.dev("mm_stat")).and_then(|t| parse_mm_stat(&t))
        {
            if compressed > 0 {
                out.push_str(&format!(
                    "  stored      {} as {} ({:.1}:1), {} of memory\n",
                    human_bytes(orig),
                    human_bytes(compressed),
                    orig as f64 / compressed as f64,
                    human_bytes(memory)
                ));
            } else {
                out.push_str("  stored      nothing yet\n");
            }
        }
        out
    }
}

pub fn parse_args(parser: &mut lexopt::Parser) -> Result<Options> {
    use lexopt::prelude::*;
    let mut action = None;
    let mut on = OnOptions::default();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("size") => on.size = Some(parse_size(&parser.value()?.string()?)?),
            Long("if-unconfigured") => on.if_unconfigured = true,
            Short('h') | Long("help") => {
                out!("{HELP}");
                std::process::exit(0);
            }
            Value(v) if action.is_none() => {
                action = Some(match v.string()?.as_str() {
                    "on" => Action::On,
                    "off" => Action::Off,
                    "status" => Action::Status,
                    other => {
                        return Err(Error::Usage(format!(
                            "unknown zram command `{other}` (on, off or status)"
                        )));
                    }
                });
            }
            other => return Err(other.unexpected().into()),
        }
    }
    Ok(Options {
        action: action.unwrap_or(Action::Status),
        on,
    })
}

pub fn run(o: Options) -> Result<()> {
    let zram = Zram::new("/");
    match o.action {
        Action::Status => out!("{}", zram.status()),
        Action::On => {
            sys::require_root("`seg zram on`")?;
            outln!(
                "{}",
                zram.on(&mut RealRunner, &o.on, Duration::from_millis(100))?
            );
        }
        Action::Off => {
            sys::require_root("`seg zram off`")?;
            outln!("{}", zram.off(&mut RealRunner)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    #[derive(Default)]
    struct Recorder(Vec<String>);

    impl Runner for Recorder {
        fn run(&mut self, program: &str, args: &[&str]) -> Result<()> {
            self.0.push(format!("{program} {}", args.join(" ")));
            Ok(())
        }
    }

    const NO_WAIT: Duration = Duration::ZERO;

    fn fake(disksize: &str, algos: &str) -> TempDir {
        let t = TempDir::new();
        t.put("sys/block/zram0/disksize", disksize);
        t.put("sys/block/zram0/comp_algorithm", algos);
        t.put("sys/block/zram0/reset", "");
        t.put(
            "proc/meminfo",
            "MemTotal:        8000000 kB\nMemFree: 1 kB\n",
        );
        t.put("proc/swaps", "Filename Type Size Used Priority\n");
        t.put("proc/sys/vm/swappiness", "60\n");
        t.put("proc/sys/vm/page-cluster", "3\n");
        t.put("sys/module/zswap/parameters/enabled", "Y\n");
        t
    }

    fn read(t: &TempDir, rel: &str) -> String {
        fs::read_to_string(t.path().join(rel))
            .unwrap()
            .trim()
            .to_string()
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("2G").unwrap(), 2 << 30);
        assert_eq!(parse_size("4096M").unwrap(), 4096 << 20);
        assert_eq!(parse_size("512m").unwrap(), 512 << 20);
        assert_eq!(parse_size("1048576").unwrap(), 1_048_576);
        for bad in [
            "",
            "G",
            "0",
            "0M",
            "-1G",
            "1.5G",
            "2T",
            "99999999999999999999G",
        ] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn half_the_ram_up_to_8_gib() {
        assert_eq!(auto_size(8_000_000), 8_000_000 * 1024 / 2);
        assert_eq!(auto_size(4 * 1024 * 1024), 2 << 30);
        assert_eq!(auto_size(64 * 1024 * 1024), 8 << 30);
        assert_eq!(auto_size(0), 0);
        assert_eq!(mem_total_kib("MemTotal:  42 kB\n"), Some(42));
        assert_eq!(mem_total_kib("nothing"), None);
    }

    #[test]
    fn swap_table_and_stats() {
        let swaps = "Filename\tType\tSize\tUsed\tPriority\n/dev/zram0 partition 4194300 1024 100\n/swapfile file 2097148 0 -2\n";
        let list = parse_swaps(swaps);
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[0],
            SwapEntry {
                path: "/dev/zram0".into(),
                size_kib: 4_194_300,
                used_kib: 1024,
                priority: 100
            }
        );
        assert_eq!(list[1].priority, -2);
        assert!(parse_swaps("Filename Type Size Used Priority\n").is_empty());

        assert_eq!(
            parse_algorithms("lzo lzo-rle lz4 [zstd]"),
            (
                vec!["lzo".into(), "lzo-rle".into(), "lz4".into(), "zstd".into()],
                Some("zstd".into())
            )
        );
        assert_eq!(
            parse_mm_stat("40960 10240 16384 0 16384 0 0 0 0"),
            Some((40960, 10240, 16384))
        );
        assert_eq!(parse_mm_stat("1 2"), None);
        assert_eq!(parse_mm_stat("a b c"), None);
    }

    #[test]
    fn on_configures_zstd_half_the_ram_and_swaps_to_it() {
        let t = fake("0\n", "lzo lzo-rle [lz4] zstd\n");
        let mut r = Recorder::default();
        let msg = Zram::new(t.path())
            .on(&mut r, &OnOptions::default(), NO_WAIT)
            .unwrap();
        assert!(msg.starts_with("zram swap is on: "), "{msg}");
        assert_eq!(read(&t, "sys/block/zram0/comp_algorithm"), "zstd");
        assert_eq!(
            read(&t, "sys/block/zram0/disksize"),
            (8_000_000u64 * 1024 / 2).to_string()
        );
        assert_eq!(
            r.0,
            ["mkswap -U clear /dev/zram0", "swapon -p 100 /dev/zram0"]
        );
        assert_eq!(read(&t, "sys/module/zswap/parameters/enabled"), "0");
        assert_eq!(read(&t, "proc/sys/vm/page-cluster"), "0");
        // No saved `seg tune` profile: swap to memory early.
        assert_eq!(read(&t, "proc/sys/vm/swappiness"), "180");
    }

    #[test]
    fn on_uses_an_explicit_size_and_resets_a_configured_device() {
        let t = fake("1048576\n", "[lzo] zstd\n");
        let mut r = Recorder::default();
        Zram::new(t.path())
            .on(
                &mut r,
                &OnOptions {
                    size: Some(2 << 30),
                    ..OnOptions::default()
                },
                NO_WAIT,
            )
            .unwrap();
        assert_eq!(read(&t, "sys/block/zram0/reset"), "1");
        assert_eq!(
            read(&t, "sys/block/zram0/disksize"),
            (2u64 << 30).to_string()
        );
    }

    #[test]
    fn a_device_another_rule_sized_is_left_alone() {
        // SegLinux Setup's udev rule gives zram0 its size and formats it; the
        // package's rule must not reset it under that rule's feet.
        let t = fake("4294967296\n", "[zstd]\n");
        let mut r = Recorder::default();
        let msg = Zram::new(t.path())
            .on(
                &mut r,
                &OnOptions {
                    if_unconfigured: true,
                    ..OnOptions::default()
                },
                NO_WAIT,
            )
            .unwrap();
        assert!(msg.contains("leaving it"), "{msg}");
        assert!(r.0.is_empty());
        assert_eq!(read(&t, "sys/block/zram0/reset"), "");
        assert_eq!(read(&t, "sys/block/zram0/disksize"), "4294967296");
    }

    #[test]
    fn a_sized_device_that_is_not_swap_keeps_its_size_for_a_plain_on() {
        let t = fake("4294967296\n", "[zstd]\n");
        let mut r = Recorder::default();
        let msg = Zram::new(t.path())
            .on(&mut r, &OnOptions::default(), NO_WAIT)
            .unwrap();
        assert!(msg.starts_with("zram swap is on: 4.0 GiB"), "{msg}");
        // No reset, same size; just formatted and switched on.
        assert_eq!(read(&t, "sys/block/zram0/reset"), "");
        assert_eq!(read(&t, "sys/block/zram0/disksize"), "4294967296");
        assert_eq!(
            r.0,
            ["mkswap -U clear /dev/zram0", "swapon -p 100 /dev/zram0"]
        );
    }

    #[test]
    fn on_falls_back_when_the_kernel_lacks_zstd() {
        let t = fake("0\n", "[lzo] lzo-rle lz4\n");
        let mut r = Recorder::default();
        let msg = Zram::new(t.path())
            .on(&mut r, &OnOptions::default(), NO_WAIT)
            .unwrap();
        assert!(msg.contains("zstd is not available"), "{msg}");
        assert_eq!(
            read(&t, "sys/block/zram0/comp_algorithm"),
            "[lzo] lzo-rle lz4"
        );
    }

    #[test]
    fn on_when_already_on_does_nothing() {
        let t = fake("0\n", "[zstd]\n");
        t.put(
            "proc/swaps",
            "Filename Type Size Used Priority\n/dev/zram0 partition 4194300 0 100\n",
        );
        let mut r = Recorder::default();
        let msg = Zram::new(t.path())
            .on(&mut r, &OnOptions::default(), NO_WAIT)
            .unwrap();
        assert!(msg.starts_with("zram swap is already on"), "{msg}");
        assert!(r.0.is_empty());
    }

    #[test]
    fn on_loads_the_module_when_the_device_is_missing() {
        let t = TempDir::new();
        t.put("proc/meminfo", "MemTotal: 1000 kB\n");
        t.put("proc/swaps", "Filename Type Size Used Priority\n");
        let mut r = Recorder::default();
        // The recorder does not create the device, so this gives up after
        // asking for the module.
        let e = Zram::new(t.path())
            .on(&mut r, &OnOptions::default(), NO_WAIT)
            .unwrap_err();
        assert_eq!(r.0, ["modprobe zram"]);
        assert!(matches!(e, Error::Unsupported(_)), "{e}");
    }

    #[test]
    fn a_saved_tune_profile_decides_the_swappiness() {
        let t = fake("0\n", "[zstd]\n");
        t.put("etc/seg/tune.conf", "mode = performance\n");
        t.put("proc/sys/vm/vfs_cache_pressure", "100\n");
        t.put("proc/sys/vm/dirty_ratio", "20\n");
        t.put("proc/sys/vm/dirty_background_ratio", "10\n");
        let mut r = Recorder::default();
        Zram::new(t.path())
            .on(&mut r, &OnOptions::default(), NO_WAIT)
            .unwrap();
        // The recorder cannot add the swap, so the profile sees no zram and
        // wants its own value; what matters is that tune decided, not 100.
        assert_eq!(read(&t, "proc/sys/vm/swappiness"), "10");
    }

    #[test]
    fn off_swaps_off_and_resets() {
        let t = fake("4294967296\n", "[zstd]\n");
        t.put(
            "proc/swaps",
            "Filename Type Size Used Priority\n/dev/zram0 partition 4194300 0 100\n",
        );
        let mut r = Recorder::default();
        let msg = Zram::new(t.path()).off(&mut r).unwrap();
        assert_eq!(r.0, ["swapoff /dev/zram0"]);
        assert_eq!(read(&t, "sys/block/zram0/reset"), "1");
        assert!(msg.contains("off"), "{msg}");

        let t = fake("0\n", "[zstd]\n");
        let mut r = Recorder::default();
        assert_eq!(
            Zram::new(t.path()).off(&mut r).unwrap(),
            "zram swap is already off"
        );
        assert!(r.0.is_empty());
    }

    #[test]
    fn status_reports_the_ratio() {
        let t = fake("4294967296\n", "lzo [zstd]\n");
        t.put(
            "proc/swaps",
            "Filename Type Size Used Priority\n/dev/zram0 partition 4194300 2048 100\n",
        );
        t.put(
            "sys/block/zram0/mm_stat",
            "4194304 1048576 1310720 0 1310720 0 0 0 0\n",
        );
        let text = Zram::new(t.path()).status();
        assert!(text.starts_with("zram: on"), "{text}");
        assert!(text.contains("zstd"));
        assert!(text.contains("4.0 MiB as 1.0 MiB (4.0:1)"), "{text}");

        let t = fake("0\n", "[zstd]\n");
        assert_eq!(Zram::new(t.path()).status(), "zram: off\n");
        let t = TempDir::new();
        assert!(
            Zram::new(t.path())
                .status()
                .contains("module is not loaded")
        );
    }
}
