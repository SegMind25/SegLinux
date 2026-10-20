//! `seg doctor`: looks for the problems that most often break an Artix
//! system and prints the fix for each one it finds. Read-only.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::info::{self, DfRow};
use crate::init;
use crate::svc::{Service, State, Svc};
use crate::sys::{self, bold, human_bytes};

pub const HELP: &str = "\
Usage: seg doctor

Checks for: services that are enabled but not running, firmware the kernel
could not load, low disk space, a stale pacman lock (db.lck) and a missing
or incomplete pacman keyring. For each problem it prints the command that
fixes it. Exits with status 1 when it found something.

Some checks need root to read the kernel log and the keyring: run
`sudo seg doctor` to get all of them.
";

#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    pub detail: String,
    pub fix: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Healthy(String),
    Problems(Vec<Problem>),
    /// The check could not run here.
    Skipped(String),
}

/// Services that start at boot but are not running.
pub fn failed_services(list: &[Service]) -> Vec<&Service> {
    list.iter()
        .filter(|s| s.runs_at_boot && matches!(s.state, State::Crashed | State::Stopped))
        .collect()
}

fn services_problems(list: &[Service], init: init::Init) -> Outcome {
    let failed = failed_services(list);
    if failed.is_empty() {
        let known = list.iter().any(|s| s.state != State::Unknown);
        return Outcome::Healthy(if known {
            "every enabled service is running".into()
        } else {
            "no service state to check".into()
        });
    }
    let start = |name: &str| match init {
        init::Init::OpenRc => {
            format!("sudo rc-service {name} start   (then: rc-service {name} status)")
        }
        init::Init::Runit => format!(
            "sudo sv start /etc/runit/runsvdir/default/{name}   (log: /var/log/{name}/current)"
        ),
        init::Init::Dinit => format!("sudo dinitctl start {name}   (log: dinitctl catlog {name})"),
        init::Init::S6 => format!("sudo s6-rc -u change {name}"),
    };
    Outcome::Problems(
        failed
            .into_iter()
            .map(|s| Problem {
                detail: format!("{} is enabled but {}", s.name, s.state.label()),
                fix: format!(
                    "{}; if it should not start at boot: sudo seg svc disable {}",
                    start(&s.name),
                    s.name
                ),
            })
            .collect(),
    )
}

/// Whether a log line reports error -2 (ENOENT), and not -22 or -20.
fn is_enoent(line: &str) -> bool {
    ["error -2", "(-2"].iter().any(|needle| {
        line.match_indices(needle).any(|(at, _)| {
            !line
                .as_bytes()
                .get(at + needle.len())
                .is_some_and(u8::is_ascii_digit)
        })
    })
}

/// File names the kernel could not load firmware for, from `dmesg`.
pub fn parse_missing_firmware(dmesg: &str) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    for line in dmesg.lines() {
        let found = if let Some(rest) = line.split("Direct firmware load for ").nth(1) {
            rest.split_whitespace().next()
        } else if let Some(rest) = line.split("firmware: failed to load ").nth(1) {
            rest.split_whitespace().next()
        } else if let Some(rest) = line.split("Failed to load firmware \"").nth(1) {
            rest.split('"').next()
        } else {
            None
        };
        if let Some(name) = found.map(|n| n.trim_end_matches([',', '.', ';'])) {
            // Only "file not found" (-2) is a missing file; other errors mean
            // a bad file, which a package cannot fix.
            let missing = is_enoent(line) || line.contains("Failed to load firmware");
            if !name.is_empty() && missing {
                files.insert(name.to_string());
            }
        }
    }
    files
}

fn firmware_outcome(dmesg: Option<&str>) -> Outcome {
    let Some(text) = dmesg else {
        return Outcome::Skipped("cannot read the kernel log; run `sudo seg doctor`".into());
    };
    let files = parse_missing_firmware(text);
    if files.is_empty() {
        return Outcome::Healthy("the kernel found all the firmware it asked for".into());
    }
    let first = files.iter().next().cloned().unwrap_or_default();
    Outcome::Problems(vec![Problem {
        detail: format!(
            "the kernel could not load {} firmware file{}: {}. Drivers that try a newer version first and fall back (iwlwifi, amdgpu) are harmless when the device works",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            files.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
                + if files.len() > 6 { ", ..." } else { "" }
        ),
        fix: format!(
            "sudo pacman -Fy && pacman -F usr/lib/firmware/{first}   (install the package it names, or linux-firmware), then reboot"
        ),
    }])
}

const BOOT_MOUNTS: [&str; 3] = ["/boot", "/boot/efi", "/efi"];
const WATCHED_MOUNTS: [&str; 4] = ["/", "/home", "/var", "/usr"];

/// File systems that are nearly full. The boot partition is small by design,
/// so it only counts when it has almost nothing left.
pub fn low_space(rows: &[DfRow]) -> Vec<&DfRow> {
    const GIB_KIB: u64 = 1024 * 1024;
    rows.iter()
        .filter(|r| {
            if BOOT_MOUNTS.contains(&r.mount.as_str()) {
                r.avail_kib < 20 * 1024
            } else if WATCHED_MOUNTS.contains(&r.mount.as_str()) {
                r.avail_kib < GIB_KIB || (r.use_percent >= 90 && r.avail_kib < 5 * GIB_KIB)
            } else {
                false
            }
        })
        .collect()
}

fn disk_outcome(rows: &[DfRow]) -> Outcome {
    if rows.is_empty() {
        return Outcome::Skipped("`df` gave no answer".into());
    }
    let low = low_space(rows);
    if low.is_empty() {
        return Outcome::Healthy("enough free space on /, /home, /var and /boot".into());
    }
    Outcome::Problems(
        low.into_iter()
            .map(|r| Problem {
                detail: format!(
                    "{} is {}% full, {} left",
                    r.mount,
                    r.use_percent,
                    human_bytes(r.avail_kib * 1024)
                ),
                fix: if BOOT_MOUNTS.contains(&r.mount.as_str()) {
                    "remove old kernels and initramfs images you do not boot (pacman -Rns the old kernel package)".into()
                } else {
                    "sudo seg clean --apply   (then: du -xh --max-depth=1 / | sort -h | tail)".into()
                },
            })
            .collect(),
    )
}

/// A lock is stale when pacman is not running.
pub fn lock_is_stale(lock_exists: bool, pacman_running: bool) -> bool {
    lock_exists && !pacman_running
}

fn pacman_running(proc: &Path) -> bool {
    let Ok(entries) = fs::read_dir(proc) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
            && sys::read_trimmed(e.path().join("comm")).as_deref() == Some("pacman")
    })
}

fn lock_outcome(lock: &Path, proc: &Path) -> Outcome {
    let exists = lock.exists();
    if !exists {
        return Outcome::Healthy("pacman is not locked".into());
    }
    if lock_is_stale(exists, pacman_running(proc)) {
        Outcome::Problems(vec![Problem {
            detail: format!("{} exists but pacman is not running", lock.display()),
            fix: format!("sudo rm {}", lock.display()),
        }])
    } else {
        Outcome::Healthy("pacman is running (its lock is in use)".into())
    }
}

/// Section names of pacman.conf, without `options`.
pub fn enabled_repos(conf: &str) -> Vec<String> {
    conf.lines()
        .filter_map(|l| {
            let l = l.trim();
            let name = l.strip_prefix('[')?.strip_suffix(']')?;
            (name != "options").then(|| name.to_string())
        })
        .collect()
}

pub enum Keys<'a> {
    Listed(&'a str),
    Failed(&'a str),
    Unavailable,
}

pub struct KeyringState<'a> {
    pub gnupg_dir: bool,
    pub artix_keyring_installed: bool,
    pub keys: Keys<'a>,
    pub arch_repos: bool,
}

pub fn analyze_keyring(k: &KeyringState) -> Vec<Problem> {
    let mut problems = Vec::new();
    let reinit = "sudo pacman-key --init && sudo pacman-key --populate artix";
    if !k.gnupg_dir {
        problems.push(Problem {
            detail: "pacman's keyring does not exist (/etc/pacman.d/gnupg)".into(),
            fix: reinit.into(),
        });
    }
    if !k.artix_keyring_installed {
        problems.push(Problem {
            detail: "the artix-keyring package is not installed".into(),
            fix: "sudo pacman -Sy artix-keyring && sudo pacman-key --populate artix".into(),
        });
    }
    match k.keys {
        Keys::Listed(text) => {
            if !text.contains("Artix") {
                problems.push(Problem {
                    detail: "the keyring has no Artix packager keys, so packages fail with \"signature is unknown trust\"".into(),
                    fix: "sudo pacman-key --populate artix".into(),
                });
            }
            if k.arch_repos && !text.contains("Arch Linux") {
                problems.push(Problem {
                    detail:
                        "the Arch repositories are enabled but the keyring has no Arch Linux keys"
                            .into(),
                    fix: "sudo pacman -S archlinux-keyring && sudo pacman-key --populate archlinux"
                        .into(),
                });
            }
        }
        Keys::Failed(err) if k.gnupg_dir => problems.push(Problem {
            detail: format!("pacman-key cannot read the keyring: {}", err.trim()),
            fix: format!("sudo rm -rf /etc/pacman.d/gnupg && {reinit}"),
        }),
        Keys::Failed(_) | Keys::Unavailable => {}
    }
    problems
}

fn keyring_outcome(root_user: bool) -> Outcome {
    let conf = fs::read_to_string("/etc/pacman.conf").unwrap_or_default();
    let repos = enabled_repos(&conf);
    let arch_repos = repos
        .iter()
        .any(|r| matches!(r.as_str(), "core" | "extra" | "multilib"));
    let artix_keyring_installed = fs::read_dir("/var/lib/pacman/local")
        .map(|d| {
            d.flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("artix-keyring-")
            })
        })
        .unwrap_or(false);
    let listed = if sys::which("pacman-key").is_some() {
        sys::run("pacman-key", &["--list-keys"]).ok()
    } else {
        None
    };
    // Without root the keyring may simply be unreadable.
    let unreadable = !root_user
        && listed
            .as_ref()
            .is_some_and(|o| !o.ok && o.stderr.contains("ermission"));
    let keys = match &listed {
        Some(o) if o.ok => Keys::Listed(&o.stdout),
        Some(_) if unreadable => Keys::Unavailable,
        Some(o) => Keys::Failed(&o.stderr),
        None => Keys::Unavailable,
    };
    let problems = analyze_keyring(&KeyringState {
        gnupg_dir: Path::new("/etc/pacman.d/gnupg").exists(),
        artix_keyring_installed,
        keys,
        arch_repos,
    });
    if !problems.is_empty() {
        Outcome::Problems(problems)
    } else if unreadable {
        Outcome::Skipped("cannot read the keyring; run `sudo seg doctor`".into())
    } else if listed.is_none() {
        Outcome::Skipped("pacman-key is not installed".into())
    } else {
        Outcome::Healthy("pacman's keyring has the Artix keys".into())
    }
}

fn print_outcome(name: &str, outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Healthy(msg) => outln!("[ ok ] {name:<10}{msg}"),
        Outcome::Skipped(msg) => outln!("[skip] {name:<10}{msg}"),
        Outcome::Problems(problems) => {
            for p in problems {
                outln!("[FAIL] {name:<10}{}", p.detail);
                outln!("       {:<10}fix: {}", "", p.fix);
            }
        }
    }
    !matches!(outcome, Outcome::Problems(_))
}

/// Runs every check. Returns whether the system is healthy.
pub fn run() -> Result<bool> {
    outln!("{}", bold("seg doctor"));
    let root_user = sys::euid().map(|u| u == 0).unwrap_or(false);

    let services = match init::detect_running() {
        None => Outcome::Skipped("no supported init system found".into()),
        Some(init) => match Svc::new(init, "/").list() {
            Ok(list) => services_problems(&list, init),
            Err(e) => Outcome::Skipped(e.to_string()),
        },
    };
    let dmesg = sys::run("dmesg", &[])
        .ok()
        .filter(|o| o.ok)
        .map(|o| o.stdout);
    let outcomes = [
        ("services", services),
        ("firmware", firmware_outcome(dmesg.as_deref())),
        ("disk", disk_outcome(&info::df())),
        (
            "pacman",
            lock_outcome(Path::new("/var/lib/pacman/db.lck"), Path::new("/proc")),
        ),
        ("keyring", keyring_outcome(root_user)),
    ];
    let mut healthy = true;
    for (name, outcome) in &outcomes {
        healthy &= print_outcome(name, outcome);
    }
    if healthy {
        outln!("\nNo problems found.");
    } else {
        outln!("\nProblems found: run the fix shown under each one.");
    }
    Ok(healthy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init::Init;
    use crate::sys::testutil::TempDir;

    fn svc(name: &str, enabled: bool, state: State) -> Service {
        Service {
            name: name.into(),
            enabled,
            runs_at_boot: enabled,
            state,
        }
    }

    fn row(mount: &str, avail_kib: u64, use_percent: u8) -> DfRow {
        DfRow {
            filesystem: "/dev/x".into(),
            size_kib: 100_000_000,
            used_kib: 0,
            avail_kib,
            use_percent,
            mount: mount.into(),
        }
    }

    #[test]
    fn only_enabled_services_that_are_down_are_failures() {
        let list = [
            svc("a", true, State::Started),
            svc("b", true, State::Crashed),
            svc("c", true, State::Stopped),
            svc("d", false, State::Stopped),
            svc("e", true, State::Unknown),
        ];
        let failed: Vec<_> = failed_services(&list)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(failed, ["b", "c"]);
    }

    #[test]
    fn services_of_runlevels_that_boot_does_not_enter_are_not_failures() {
        // OpenRC's killprocs, mount-ro and savecache sit in `shutdown`.
        let mut late = svc("killprocs", true, State::Stopped);
        late.runs_at_boot = false;
        assert!(failed_services(&[late]).is_empty());
    }

    #[test]
    fn the_service_fix_names_the_inits_own_command() {
        let list = [svc("cupsd", true, State::Crashed)];
        let Outcome::Problems(p) = services_problems(&list, Init::OpenRc) else {
            panic!("no problem")
        };
        assert!(p[0].detail.contains("cupsd is enabled but crashed"));
        assert!(p[0].fix.contains("rc-service cupsd start"));
        assert!(p[0].fix.contains("seg svc disable cupsd"));
        let Outcome::Problems(p) = services_problems(&list, Init::Dinit) else {
            panic!("no problem")
        };
        assert!(p[0].fix.contains("dinitctl start cupsd"));
        assert_eq!(
            services_problems(&[svc("a", true, State::Started)], Init::OpenRc),
            Outcome::Healthy("every enabled service is running".into())
        );
    }

    #[test]
    fn firmware_failures_from_dmesg() {
        let log = "\
[    3.1] r8169 0000:02:00.0: Direct firmware load for rtl_nic/rtl8168h-2.fw failed with error -2
[    3.2] r8169 0000:02:00.0: Unable to load firmware rtl_nic/rtl8168h-2.fw (-2)
[    4.0] iwlwifi 0000:03:00.0: Direct firmware load for iwlwifi-cc-a0-77.ucode failed with error -2
[    5.0] bluetooth hci0: Direct firmware load for intel/ibt-0040-0041.sfi failed with error -2
[    6.0] foo: firmware: failed to load bar/baz.bin (-2)
[    6.1] foo: Failed to load firmware \"old/style.fw\"
[    7.0] amdgpu: Direct firmware load for amdgpu/x.bin failed with error -22
[    8.0] usb 1-1: new high-speed USB device
";
        let files = parse_missing_firmware(log);
        assert_eq!(
            files.into_iter().collect::<Vec<_>>(),
            [
                "bar/baz.bin",
                "intel/ibt-0040-0041.sfi",
                "iwlwifi-cc-a0-77.ucode",
                "old/style.fw",
                "rtl_nic/rtl8168h-2.fw"
            ]
        );
        assert!(parse_missing_firmware("").is_empty());
    }

    #[test]
    fn firmware_outcomes() {
        assert!(matches!(firmware_outcome(None), Outcome::Skipped(_)));
        assert!(matches!(
            firmware_outcome(Some("all fine\n")),
            Outcome::Healthy(_)
        ));
        let Outcome::Problems(p) = firmware_outcome(Some(
            "x: Direct firmware load for a/b.fw failed with error -2\n",
        )) else {
            panic!("no problem")
        };
        assert!(p[0].detail.contains("1 firmware file: a/b.fw"));
        assert!(p[0].fix.contains("pacman -F usr/lib/firmware/a/b.fw"));
    }

    #[test]
    fn low_disk_space() {
        let rows = [
            row("/", 500 * 1024, 99),            // 500 MiB left
            row("/home", 400 * 1024 * 1024, 40), // plenty
            row("/var", 3 * 1024 * 1024, 93),    // 3 GiB left at 93%
            row("/boot", 10 * 1024, 95),         // 10 MiB: nearly nothing
            row("/boot/efi", 90 * 1024, 70),     // small by design, fine
            row("/mnt/data", 1, 100),            // not watched
            row("/usr", 50 * 1024 * 1024, 92),   // 92% but 50 GiB left
        ];
        let low: Vec<_> = low_space(&rows).iter().map(|r| r.mount.as_str()).collect();
        assert_eq!(low, ["/", "/var", "/boot"]);
        let Outcome::Problems(p) = disk_outcome(&rows) else {
            panic!("no problem")
        };
        assert!(p[0].detail.contains("/ is 99% full"));
        assert!(p[0].fix.contains("seg clean"));
        assert!(p[2].fix.contains("old kernels"));
        assert!(matches!(disk_outcome(&[]), Outcome::Skipped(_)));
    }

    #[test]
    fn a_lock_is_stale_only_without_a_running_pacman() {
        assert!(lock_is_stale(true, false));
        assert!(!lock_is_stale(true, true));
        assert!(!lock_is_stale(false, false));
    }

    #[test]
    fn lock_check_looks_for_a_pacman_process() {
        let t = TempDir::new();
        let lock = t.put("db.lck", "");
        t.put("proc/1/comm", "init\n");
        t.put("proc/42/comm", "bash\n");
        let Outcome::Problems(p) = lock_outcome(&lock, &t.path().join("proc")) else {
            panic!("no problem")
        };
        assert!(p[0].fix.starts_with("sudo rm "));
        t.put("proc/43/comm", "pacman\n");
        assert!(matches!(
            lock_outcome(&lock, &t.path().join("proc")),
            Outcome::Healthy(_)
        ));
        fs::remove_file(&lock).unwrap();
        assert_eq!(
            lock_outcome(&lock, &t.path().join("proc")),
            Outcome::Healthy("pacman is not locked".into())
        );
    }

    #[test]
    fn repos_of_pacman_conf() {
        let conf = "[options]\nColor\n[system]\nInclude = x\n# [extra]\n[world]\n[lib32]\n";
        assert_eq!(enabled_repos(conf), ["system", "world", "lib32"]);
        assert_eq!(enabled_repos("[options]\n"), Vec::<String>::new());
    }

    #[test]
    fn keyring_analysis() {
        let healthy = KeyringState {
            gnupg_dir: true,
            artix_keyring_installed: true,
            keys: Keys::Listed("uid [ full ] Artix Linux Master Key"),
            arch_repos: false,
        };
        assert!(analyze_keyring(&healthy).is_empty());

        let empty = KeyringState {
            gnupg_dir: false,
            artix_keyring_installed: false,
            keys: Keys::Unavailable,
            arch_repos: false,
        };
        let p = analyze_keyring(&empty);
        assert_eq!(p.len(), 2);
        assert!(p[0].fix.contains("pacman-key --init"));
        assert!(p[1].fix.contains("pacman -Sy artix-keyring"));

        let unpopulated = KeyringState {
            keys: Keys::Listed("pubring.kbx\n-----\n"),
            ..healthy
        };
        let p = analyze_keyring(&unpopulated);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].fix, "sudo pacman-key --populate artix");

        let arch = KeyringState {
            gnupg_dir: true,
            artix_keyring_installed: true,
            keys: Keys::Listed("Artix"),
            arch_repos: true,
        };
        let p = analyze_keyring(&arch);
        assert_eq!(p.len(), 1);
        assert!(p[0].fix.contains("archlinux-keyring"));

        let broken = KeyringState {
            gnupg_dir: true,
            artix_keyring_installed: true,
            keys: Keys::Failed("gpg: keyblock resource: No such file\n"),
            arch_repos: false,
        };
        let p = analyze_keyring(&broken);
        assert!(
            p[0].detail
                .contains("cannot read the keyring: gpg: keyblock resource")
        );
        assert!(p[0].fix.contains("rm -rf /etc/pacman.d/gnupg"));
    }
}
