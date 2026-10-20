//! The part of `seg game run` that needs root: while at least one game runs,
//! the CPU governor and energy preference are `performance` and an AMD GPU is
//! held at its high power level; when the last game ends, every value goes
//! back to what it was.
//!
//! `seg game run` reaches this through `sudo -n seg game boost on|off PID`
//! (the sudoers file of the segctl package allows exactly that to the
//! `gamemode` group) or calls it directly when it runs as root. Several games
//! share one state file, /run/seg/game-boost, under a lock; a game that died
//! without saying so (SIGKILL, power loss of the session) is noticed by its
//! process id and start time and does not keep the boost alive.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::hw::{self, GpuVendor};
use super::proc;
use crate::error::{Context, Error, Result};
use crate::sys;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    /// Field 22 of /proc/PID/stat: tells a reused pid from the original.
    pub start: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct State {
    pub owners: Vec<Owner>,
    /// (file, value it had before the boost)
    pub saved: Vec<(String, String)>,
}

const HEADER: &str = "seg-game-boost 1";

pub fn serialize(state: &State) -> String {
    let mut text = format!("{HEADER}\n");
    for o in &state.owners {
        text.push_str(&format!("owner {} {}\n", o.pid, o.start));
    }
    for (path, value) in &state.saved {
        text.push_str(&format!("saved\t{path}\t{value}\n"));
    }
    text
}

/// Parse a state file; a file that is not ours gives an empty state.
pub fn parse_state(text: &str) -> State {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return State::default();
    }
    let mut state = State::default();
    for line in lines {
        if let Some(rest) = line.strip_prefix("owner ") {
            let mut f = rest.split_whitespace();
            if let (Some(pid), Some(start)) = (f.next(), f.next())
                && let (Ok(pid), Ok(start)) = (pid.parse(), start.parse())
            {
                state.owners.push(Owner { pid, start });
            }
        } else if let Some(rest) = line.strip_prefix("saved\t")
            && let Some((path, value)) = rest.split_once('\t')
        {
            state.saved.push((path.to_string(), value.to_string()));
        }
    }
    state
}

/// The start time (field 22) of `/proc/PID/stat`. The command name is in
/// parentheses and may contain spaces and parentheses, so count from the
/// last `)`.
pub fn parse_start_time(stat: &str) -> Option<u64> {
    let after = stat.get(stat.rfind(')')? + 1..)?;
    // Fields after the name start at field 3 (state); starttime is 22.
    after.split_whitespace().nth(22 - 3)?.parse().ok()
}

#[derive(Debug, Default)]
pub struct Report {
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
}

pub struct Boost {
    root: PathBuf,
}

impl Boost {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Boost { root: root.into() }
    }

    fn state_dir(&self) -> PathBuf {
        self.root.join("run/seg")
    }

    fn start_time(&self, pid: u32) -> Option<u64> {
        parse_start_time(&fs::read_to_string(self.root.join(format!("proc/{pid}/stat"))).ok()?)
    }

    fn alive(&self, o: &Owner) -> bool {
        self.start_time(o.pid) == Some(o.start)
    }

    fn load(&self) -> State {
        fs::read_to_string(self.state_dir().join("game-boost"))
            .map(|t| parse_state(&t))
            .unwrap_or_default()
    }

    fn store(&self, state: &State) -> Result<()> {
        let path = self.state_dir().join("game-boost");
        if state.owners.is_empty() && state.saved.is_empty() {
            let _ = fs::remove_file(&path);
            return Ok(());
        }
        sys::write_atomic(&path, &serialize(state))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .context(format!("securing {}", path.display()))
    }

    /// Run `f` holding the lock of the state directory.
    fn locked<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let dir = self.state_dir();
        fs::create_dir_all(&dir).context(format!("creating {}", dir.display()))?;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        let lock = dir.join("game.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .context(format!("opening {}", lock.display()))?;
        proc::lock_exclusive(&file).context(format!("locking {}", lock.display()))?;
        f()
    }

    /// The files to change, with the value that makes the computer fast.
    pub fn knobs(&self, governor: &str) -> Vec<(PathBuf, String)> {
        let mut knobs = Vec::new();
        for dir in hw::cpufreq_dirs(&self.root) {
            let available =
                sys::read_trimmed(dir.join("scaling_available_governors")).unwrap_or_default();
            if available.split_whitespace().any(|g| g == governor) {
                knobs.push((dir.join("scaling_governor"), governor.to_string()));
            }
            if governor == "performance" {
                let epp = dir.join("energy_performance_preference");
                let preferences =
                    sys::read_trimmed(dir.join("energy_performance_available_preferences"));
                let allowed = preferences
                    .as_deref()
                    .is_none_or(|p| p.split_whitespace().any(|x| x == "performance"));
                if epp.exists() && allowed {
                    knobs.push((epp, "performance".to_string()));
                }
            }
        }
        if governor == "performance" {
            for gpu in hw::gpus(&self.root) {
                if gpu.vendor == GpuVendor::Amd && gpu.driver.as_deref() == Some("amdgpu") {
                    let level = self.root.join(format!(
                        "sys/class/drm/{}/device/power_dpm_force_performance_level",
                        gpu.card
                    ));
                    if level.exists() {
                        knobs.push((level, "high".to_string()));
                    }
                }
            }
        }
        knobs
    }

    /// Start the boost for the game process `pid`, or join the one that is
    /// on. The first game decides the governor.
    pub fn on(&self, pid: u32, governor: &str) -> Result<Report> {
        let start = self
            .start_time(pid)
            .ok_or_else(|| Error::other(format!("no process {pid}")))?;
        self.locked(|| {
            let mut report = Report::default();
            let mut state = self.load();
            state.owners.retain(|o| self.alive(o));
            state.owners.retain(|o| o.pid != pid);
            if state.owners.is_empty() {
                // Nobody boosted: remember what is there now, then change it.
                // (A leftover `saved` of games that all died is the truth, so
                // it is put back first.)
                self.restore(&mut state, &mut report);
                for (path, value) in self.knobs(governor) {
                    let Some(before) = sys::read_trimmed(&path) else {
                        continue;
                    };
                    if before == value {
                        continue;
                    }
                    match fs::write(&path, &value) {
                        Ok(()) => {
                            state.saved.push((path.display().to_string(), before));
                        }
                        Err(e) => report.warnings.push(format!("{}: {e}", path.display())),
                    }
                }
                report.lines.push(format!(
                    "boost on: {} settings changed ({governor})",
                    state.saved.len()
                ));
            } else {
                report
                    .lines
                    .push("boost already on for another game".into());
            }
            state.owners.push(Owner { pid, start });
            self.store(&state)?;
            Ok(report)
        })
    }

    fn restore(&self, state: &mut State, report: &mut Report) {
        for (path, value) in state.saved.drain(..) {
            if let Err(e) = fs::write(&path, &value) {
                report.warnings.push(format!("{path}: {e}"));
            }
        }
    }

    /// The game `pid` has ended: when it was the last, put everything back.
    pub fn off(&self, pid: u32) -> Result<Report> {
        self.locked(|| {
            let mut report = Report::default();
            let mut state = self.load();
            state.owners.retain(|o| o.pid != pid && self.alive(o));
            if state.owners.is_empty() {
                let count = state.saved.len();
                self.restore(&mut state, &mut report);
                report
                    .lines
                    .push(format!("boost off: {count} settings restored"));
            } else {
                report.lines.push("boost stays on for another game".into());
            }
            self.store(&state)?;
            Ok(report)
        })
    }

    pub fn status(&self) -> String {
        let state = self.load();
        let live = state.owners.iter().filter(|o| self.alive(o)).count();
        format!(
            "game boost: {} ({live} game{} running, {} settings saved)\n",
            if live > 0 { "on" } else { "off" },
            if live == 1 { "" } else { "s" },
            state.saved.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    fn proc_entry(t: &TempDir, pid: u32, start: u64) {
        t.put(
            &format!("proc/{pid}/stat"),
            &format!("{pid} (my game (x)) S 1 {pid} {pid} 0 -1 4194560 100 0 0 0 1 1 0 0 20 0 1 0 {start} 1000 100 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0\n"),
        );
    }

    /// Two CPUs with cpufreq, an AMD GPU, and two games' worth of processes.
    fn machine() -> TempDir {
        let t = TempDir::new();
        for n in [0, 1] {
            let d = format!("sys/devices/system/cpu/cpu{n}/cpufreq");
            t.put(
                &format!("{d}/scaling_available_governors"),
                "performance schedutil powersave\n",
            );
            t.put(&format!("{d}/scaling_governor"), "schedutil\n");
            t.put(
                &format!("{d}/energy_performance_preference"),
                "balance_performance\n",
            );
            t.put(
                &format!("{d}/energy_performance_available_preferences"),
                "default performance balance_performance balance_power power\n",
            );
        }
        let g = "sys/class/drm/card0/device";
        t.put(&format!("{g}/vendor"), "0x1002\n");
        t.put(&format!("{g}/device"), "0x744c\n");
        t.put(
            &format!("{g}/uevent"),
            "DRIVER=amdgpu\nPCI_SLOT_NAME=0000:03:00.0\n",
        );
        t.put(&format!("{g}/power_dpm_force_performance_level"), "auto\n");
        proc_entry(&t, 100, 5000);
        proc_entry(&t, 200, 6000);
        t
    }

    fn read(t: &TempDir, rel: &str) -> String {
        fs::read_to_string(t.path().join(rel))
            .unwrap()
            .trim()
            .to_string()
    }

    const GOV: &str = "sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";
    const EPP: &str = "sys/devices/system/cpu/cpu1/cpufreq/energy_performance_preference";
    const GPU: &str = "sys/class/drm/card0/device/power_dpm_force_performance_level";

    #[test]
    fn state_files_round_trip() {
        let s = State {
            owners: vec![
                Owner { pid: 1, start: 10 },
                Owner {
                    pid: 22,
                    start: 333,
                },
            ],
            saved: vec![
                ("/sys/a b/gov".into(), "schedutil".into()),
                ("/sys/x".into(), "auto".into()),
            ],
        };
        assert_eq!(parse_state(&serialize(&s)), s);
        assert_eq!(parse_state("something else\n"), State::default());
        assert_eq!(parse_state(""), State::default());
        // Damaged lines are skipped, the rest is kept.
        let s = parse_state("seg-game-boost 1\nowner x y\nowner 5 6\nsaved\tnotab\n");
        assert_eq!(s.owners, vec![Owner { pid: 5, start: 6 }]);
        assert!(s.saved.is_empty());
    }

    #[test]
    fn the_start_time_survives_awkward_names() {
        let stat = "4242 (Web Content (1)) S 1 4242 4242 0 -1 4194560 1 0 0 0 1 1 0 0 20 0 1 0 987654 1 1 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0";
        assert_eq!(parse_start_time(stat), Some(987654));
        assert_eq!(parse_start_time("garbage"), None);
        assert_eq!(parse_start_time("1 (x) S 1"), None);
    }

    #[test]
    fn knobs_follow_what_the_machine_offers() {
        let t = machine();
        let b = Boost::new(t.path());
        let perf = b.knobs("performance");
        // Two governors, two EPPs, the GPU.
        assert_eq!(perf.len(), 5);
        assert!(perf.iter().any(|(p, v)| {
            p.ends_with("card0/device/power_dpm_force_performance_level") && v == "high"
        }));
        // Another governor changes the governor only.
        assert_eq!(b.knobs("powersave").len(), 2);
        // A governor the CPU lacks changes nothing.
        assert!(b.knobs("ondemand").is_empty());
    }

    #[test]
    fn a_game_boosts_and_the_end_of_the_game_restores() {
        let t = machine();
        let b = Boost::new(t.path());
        let r = b.on(100, "performance").unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(read(&t, GOV), "performance");
        assert_eq!(read(&t, EPP), "performance");
        assert_eq!(read(&t, GPU), "high");
        assert!(
            b.status().starts_with("game boost: on (1 game running"),
            "{}",
            b.status()
        );

        let r = b.off(100).unwrap();
        assert!(r.lines[0].contains("5 settings restored"), "{:?}", r.lines);
        assert_eq!(read(&t, GOV), "schedutil");
        assert_eq!(read(&t, EPP), "balance_performance");
        assert_eq!(read(&t, GPU), "auto");
        assert!(!t.path().join("run/seg/game-boost").exists());
        assert!(b.status().starts_with("game boost: off"));
    }

    #[test]
    fn two_games_share_one_boost() {
        let t = machine();
        let b = Boost::new(t.path());
        b.on(100, "performance").unwrap();
        // The second game must not save "performance" as the original.
        let r = b.on(200, "performance").unwrap();
        assert!(r.lines[0].contains("already on"), "{:?}", r.lines);
        b.off(100).unwrap();
        assert_eq!(read(&t, GOV), "performance", "the other game still runs");
        b.off(200).unwrap();
        assert_eq!(read(&t, GOV), "schedutil");
    }

    #[test]
    fn a_game_that_died_does_not_keep_the_boost_on() {
        let t = machine();
        let b = Boost::new(t.path());
        b.on(100, "performance").unwrap();
        // Game 100 was killed (its process is gone); game 200 starts and ends.
        fs::remove_dir_all(t.path().join("proc/100")).unwrap();
        let r = b.on(200, "performance").unwrap();
        assert!(!r.lines[0].contains("already on"), "{:?}", r.lines);
        b.off(200).unwrap();
        // Back to the original values, not to "performance".
        assert_eq!(read(&t, GOV), "schedutil");
        assert_eq!(read(&t, GPU), "auto");
    }

    #[test]
    fn a_reused_process_id_is_not_the_old_game() {
        let t = machine();
        let b = Boost::new(t.path());
        b.on(100, "performance").unwrap();
        // pid 100 now belongs to a different process (other start time).
        proc_entry(&t, 100, 9999);
        b.off(200).unwrap();
        assert_eq!(read(&t, GOV), "schedutil");
    }

    #[test]
    fn off_without_on_changes_nothing() {
        let t = machine();
        let b = Boost::new(t.path());
        let r = b.off(100).unwrap();
        assert!(r.lines[0].contains("0 settings restored"), "{:?}", r.lines);
        assert_eq!(read(&t, GOV), "schedutil");
    }

    #[test]
    fn a_missing_process_is_refused() {
        let t = machine();
        let e = Boost::new(t.path()).on(777, "performance").unwrap_err();
        assert!(e.to_string().contains("no process 777"), "{e}");
        assert_eq!(read(&t, GOV), "schedutil");
    }

    #[test]
    fn the_state_directory_is_private() {
        let t = machine();
        let b = Boost::new(t.path());
        b.on(100, "performance").unwrap();
        let mode = |rel: &str| {
            fs::metadata(t.path().join(rel))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("run/seg"), 0o700);
        assert_eq!(mode("run/seg/game-boost"), 0o600);
    }
}
