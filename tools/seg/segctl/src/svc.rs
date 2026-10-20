//! `seg svc`: one interface over the services of OpenRC, runit, dinit and s6.
//!
//! Enabling and disabling only touch the files each init reads at boot (the
//! symlinks `rc-update`, `ln -s ... /etc/runit/runsvdir/default` and
//! `dinitctl enable` make), so they work on a running system, in a chroot and
//! on an offline root alike. Starting and stopping ask the running init.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use crate::error::{Context, Error, Result};
use crate::init::Init;
use crate::sys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Started,
    Stopped,
    Crashed,
    Unknown,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Started => "started",
            State::Stopped => "stopped",
            State::Crashed => "crashed",
            State::Unknown => "-",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub enabled: bool,
    pub state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Enabled,
    AlreadyEnabled,
    Disabled,
    AlreadyDisabled,
}

/// Service names become file names under /etc: no paths, no hidden files.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.@+".contains(c))
}

/// OpenRC services that belong in the `boot` runlevel rather than `default`.
fn openrc_runlevel(name: &str) -> &'static str {
    const BOOT: &[&str] = &[
        "elogind",
        "udev",
        "udev-trigger",
        "binfmt",
        "sysctl",
        "modules",
        "hwclock",
        "keymaps",
        "hostname",
        "localmount",
        "lvm",
        "dmcrypt",
        "mdraid",
        "haveged",
    ];
    if BOOT.contains(&name) {
        "boot"
    } else {
        "default"
    }
}

/// The services of one init system under `root` (`/` for the running one).
pub struct Svc {
    init: Init,
    root: PathBuf,
}

impl Svc {
    pub fn new(init: Init, root: impl Into<PathBuf>) -> Self {
        Svc {
            init,
            root: root.into(),
        }
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    fn live(&self) -> bool {
        self.root == Path::new("/")
    }

    /// Where the installed definition of `name` lives.
    fn definition(&self, name: &str) -> String {
        match self.init {
            Init::OpenRc => format!("/etc/init.d/{name}"),
            Init::Runit => format!("/etc/runit/sv/{name}"),
            Init::Dinit => format!("/etc/dinit.d/{name}"),
            Init::S6 => format!("/etc/s6/sv/{name}"),
        }
    }

    pub fn exists(&self, name: &str) -> bool {
        self.p(&self.definition(name)).exists()
    }

    fn check(&self, name: &str) -> Result<()> {
        if !valid_name(name) {
            return Err(Error::Usage(format!("`{name}` is not a service name")));
        }
        if !self.exists(name) {
            return Err(Error::other(format!(
                "no service `{name}` for {} (expected {}); `seg svc list` shows the installed ones",
                self.init,
                self.definition(name)
            )));
        }
        Ok(())
    }

    /// Symlinks (or marker files) that make `name` start at boot.
    fn boot_links(&self, name: &str) -> Vec<(String, Option<String>)> {
        match self.init {
            Init::OpenRc => vec![(
                format!("/etc/runlevels/{}/{name}", openrc_runlevel(name)),
                Some(format!("/etc/init.d/{name}")),
            )],
            Init::Runit => vec![(
                format!("/etc/runit/runsvdir/default/{name}"),
                Some(format!("/etc/runit/sv/{name}")),
            )],
            Init::Dinit => vec![(
                format!("/etc/dinit.d/boot.d/{name}"),
                Some(format!("../{name}")),
            )],
            // s6-rc bundles are compiled databases: the source of the
            // `default` bundle lists its contents as empty files.
            Init::S6 => vec![(format!("/etc/s6/adminsv/default/contents.d/{name}"), None)],
        }
    }

    /// Every place `name` may be enabled in (OpenRC has several runlevels).
    fn enabled_paths(&self, name: &str) -> Vec<PathBuf> {
        match self.init {
            Init::OpenRc => {
                let Ok(levels) = fs::read_dir(self.p("/etc/runlevels")) else {
                    return Vec::new();
                };
                levels
                    .flatten()
                    .map(|l| l.path().join(name))
                    .filter(|p| p.symlink_metadata().is_ok())
                    .collect()
            }
            _ => self
                .boot_links(name)
                .into_iter()
                .map(|(link, _)| self.p(&link))
                .filter(|p| p.symlink_metadata().is_ok())
                .collect(),
        }
    }

    pub fn enable(&self, name: &str) -> Result<Change> {
        self.check(name)?;
        if !self.enabled_paths(name).is_empty() {
            return Ok(Change::AlreadyEnabled);
        }
        for (link, target) in self.boot_links(name) {
            let link = self.p(&link);
            if let Some(dir) = link.parent() {
                fs::create_dir_all(dir).context(format!("creating {}", dir.display()))?;
            }
            match target {
                Some(target) => symlink(&target, &link)
                    .context(format!("linking {} to {target}", link.display()))?,
                None => fs::write(&link, "").context(format!("creating {}", link.display()))?,
            }
        }
        if self.init == Init::S6 && self.live() && sys::which("s6-db-reload").is_some() {
            sys::run_inherit("s6-db-reload", &[])?;
        }
        Ok(Change::Enabled)
    }

    pub fn disable(&self, name: &str) -> Result<Change> {
        if !valid_name(name) {
            return Err(Error::Usage(format!("`{name}` is not a service name")));
        }
        let paths = self.enabled_paths(name);
        if paths.is_empty() {
            return Ok(Change::AlreadyDisabled);
        }
        for path in paths {
            fs::remove_file(&path).context(format!("removing {}", path.display()))?;
        }
        if self.init == Init::S6 && self.live() && sys::which("s6-db-reload").is_some() {
            sys::run_inherit("s6-db-reload", &[])?;
        }
        Ok(Change::Disabled)
    }

    /// All installed services with whether they start at boot and, when the
    /// init can be asked, whether they run.
    pub fn list(&self) -> Result<Vec<Service>> {
        let (available, enabled) = self.available_and_enabled();
        let states = if self.live() {
            self.states(&enabled)
        } else {
            HashMap::new()
        };
        Ok(available
            .into_iter()
            .map(|name| Service {
                enabled: enabled.contains(&name),
                state: states.get(&name).copied().unwrap_or(State::Unknown),
                name,
            })
            .collect())
    }

    fn available_and_enabled(&self) -> (BTreeSet<String>, BTreeSet<String>) {
        let names = |dir: &str, want_dirs: bool| -> BTreeSet<String> {
            let Ok(entries) = fs::read_dir(self.p(dir)) else {
                return BTreeSet::new();
            };
            entries
                .flatten()
                .filter(|e| {
                    let is_dir = e.path().is_dir();
                    is_dir == want_dirs
                })
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| valid_name(n))
                .collect()
        };
        match self.init {
            Init::OpenRc => {
                let mut enabled = BTreeSet::new();
                if let Ok(levels) = fs::read_dir(self.p("/etc/runlevels")) {
                    for level in levels.flatten() {
                        if let Ok(entries) = fs::read_dir(level.path()) {
                            enabled.extend(
                                entries
                                    .flatten()
                                    .filter_map(|e| e.file_name().into_string().ok()),
                            );
                        }
                    }
                }
                (names("/etc/init.d", false), enabled)
            }
            Init::Runit => {
                let enabled = fs::read_dir(self.p("/etc/runit/runsvdir/default"))
                    .map(|d| {
                        d.flatten()
                            .filter_map(|e| e.file_name().into_string().ok())
                            .collect()
                    })
                    .unwrap_or_default();
                (names("/etc/runit/sv", true), enabled)
            }
            Init::Dinit => {
                let enabled = fs::read_dir(self.p("/etc/dinit.d/boot.d"))
                    .map(|d| {
                        d.flatten()
                            .filter_map(|e| e.file_name().into_string().ok())
                            .collect()
                    })
                    .unwrap_or_default();
                (names("/etc/dinit.d", false), enabled)
            }
            Init::S6 => {
                let enabled = fs::read_dir(self.p("/etc/s6/adminsv/default/contents.d"))
                    .map(|d| {
                        d.flatten()
                            .filter_map(|e| e.file_name().into_string().ok())
                            .collect()
                    })
                    .unwrap_or_default();
                (names("/etc/s6/sv", true), enabled)
            }
        }
    }

    /// Ask the running init what runs. Best effort: a missing tool means
    /// "unknown", not an error.
    fn states(&self, enabled: &BTreeSet<String>) -> HashMap<String, State> {
        let ask = |program: &str, args: &[&str]| -> Option<String> {
            sys::which(program)?;
            let out = sys::run(program, args).ok()?;
            // rc-status and sv exit non-zero when something is down, with
            // the list on stdout all the same.
            (!out.stdout.is_empty()).then_some(out.stdout)
        };
        match self.init {
            Init::OpenRc => ask("rc-status", &["-s"])
                .map(|t| parse_rc_status(&t))
                .unwrap_or_default(),
            Init::Runit => {
                let dirs: Vec<String> = enabled
                    .iter()
                    .map(|n| format!("/etc/runit/runsvdir/default/{n}"))
                    .collect();
                if dirs.is_empty() {
                    return HashMap::new();
                }
                let mut args = vec!["status"];
                args.extend(dirs.iter().map(String::as_str));
                ask("sv", &args)
                    .map(|t| parse_sv_status(&t))
                    .unwrap_or_default()
            }
            Init::Dinit => ask("dinitctl", &["list"])
                .map(|t| parse_dinit_list(&t))
                .unwrap_or_default(),
            Init::S6 => {
                let Some(active) = ask("s6-rc", &["-a", "list"]) else {
                    return HashMap::new();
                };
                let active: BTreeSet<&str> = active.lines().map(str::trim).collect();
                let (available, _) = self.available_and_enabled();
                available
                    .into_iter()
                    .map(|n| {
                        let state = if active.contains(n.as_str()) {
                            State::Started
                        } else {
                            State::Stopped
                        };
                        (n, state)
                    })
                    .collect()
            }
        }
    }

    /// The command that starts or stops `name` on the running init.
    pub fn control_command(&self, action: Action, name: &str) -> Result<(String, Vec<String>)> {
        self.check(name)?;
        let verb = match action {
            Action::Start => "start",
            Action::Stop => "stop",
        };
        Ok(match self.init {
            Init::OpenRc => ("rc-service".into(), vec![name.into(), verb.into()]),
            Init::Runit => {
                let dir = self.p(&format!("/etc/runit/runsvdir/default/{name}"));
                if dir.symlink_metadata().is_err() {
                    return Err(Error::other(format!(
                        "`{name}` is not enabled, so runit does not supervise it; run `seg svc enable {name}` first"
                    )));
                }
                ("sv".into(), vec![verb.into(), dir.display().to_string()])
            }
            Init::Dinit => ("dinitctl".into(), vec![verb.into(), name.into()]),
            Init::S6 => (
                "s6-rc".into(),
                vec![
                    match action {
                        Action::Start => "-u",
                        Action::Stop => "-d",
                    }
                    .into(),
                    "change".into(),
                    name.into(),
                ],
            ),
        })
    }

    pub fn control(&self, action: Action, name: &str) -> Result<()> {
        let (program, args) = self.control_command(action, name)?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        sys::run_inherit(&program, &args)
    }
}

/// `rc-status -s`: ` sshd      [  started  ]`, ` foo   [  started 00:01:02 (0) ]`.
pub fn parse_rc_status(text: &str) -> HashMap<String, State> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once('[')?;
            let name = name.trim();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return None;
            }
            let word = rest.split(']').next()?.split_whitespace().next()?;
            let state = match word {
                "started" => State::Started,
                "stopped" | "inactive" | "stopping" => State::Stopped,
                "crashed" => State::Crashed,
                _ => State::Unknown,
            };
            Some((name.to_string(), state))
        })
        .collect()
}

/// `sv status`: `run: /etc/runit/runsvdir/default/sshd: (pid 12) 5s; ...`.
pub fn parse_sv_status(text: &str) -> HashMap<String, State> {
    text.lines()
        .filter_map(|line| {
            let (word, rest) = line.split_once(": ")?;
            let path = rest.split(':').next()?;
            let name = path.rsplit('/').next()?.trim();
            if name.is_empty() {
                return None;
            }
            let state = match word {
                "run" => State::Started,
                "down" | "finish" | "warning" => State::Stopped,
                "fail" => State::Crashed,
                _ => State::Unknown,
            };
            Some((name.to_string(), state))
        })
        .collect()
}

/// `dinitctl list`: `[{+}     ] NetworkManager (pid: 450)`. The first
/// three characters after the bracket are the service's state; a `+` in
/// them means started.
pub fn parse_dinit_list(text: &str) -> HashMap<String, State> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let group = line.strip_prefix('[')?.get(..3)?;
            let tail = line.get(4..)?;
            let rest = tail.get(tail.find(']')? + 1..)?;
            let name = rest.split_whitespace().next()?;
            let state = if group.contains('+') {
                State::Started
            } else {
                State::Stopped
            };
            Some((name.to_string(), state))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    fn link_target(root: &Path, rel: &str) -> PathBuf {
        fs::read_link(root.join(rel)).unwrap()
    }

    #[test]
    fn service_names_are_file_names() {
        for good in [
            "sshd",
            "NetworkManager",
            "seg-zram",
            "getty@tty1",
            "a.b_c+d",
        ] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", ".hidden", "../etc/passwd", "a/b", "a b", "a;b", "$(x)"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn openrc_enable_disable_and_runlevels() {
        let t = TempDir::new();
        t.put("etc/init.d/sshd", "");
        t.put("etc/init.d/sysctl", "");
        let svc = Svc::new(Init::OpenRc, t.path());
        assert_eq!(svc.enable("sshd").unwrap(), Change::Enabled);
        assert_eq!(svc.enable("sshd").unwrap(), Change::AlreadyEnabled);
        assert_eq!(
            link_target(t.path(), "etc/runlevels/default/sshd"),
            Path::new("/etc/init.d/sshd")
        );
        svc.enable("sysctl").unwrap();
        assert!(
            t.path()
                .join("etc/runlevels/boot/sysctl")
                .symlink_metadata()
                .is_ok()
        );
        assert_eq!(svc.disable("sshd").unwrap(), Change::Disabled);
        assert_eq!(svc.disable("sshd").unwrap(), Change::AlreadyDisabled);
        assert!(!t.path().join("etc/runlevels/default/sshd").exists());
    }

    #[test]
    fn openrc_disable_removes_every_runlevel() {
        let t = TempDir::new();
        t.put("etc/init.d/foo", "");
        let svc = Svc::new(Init::OpenRc, t.path());
        for level in ["default", "nonetwork", "boot"] {
            fs::create_dir_all(t.path().join("etc/runlevels").join(level)).unwrap();
            symlink(
                "/etc/init.d/foo",
                t.path().join("etc/runlevels").join(level).join("foo"),
            )
            .unwrap();
        }
        assert_eq!(svc.disable("foo").unwrap(), Change::Disabled);
        for level in ["default", "nonetwork", "boot"] {
            assert!(
                t.path()
                    .join("etc/runlevels")
                    .join(level)
                    .join("foo")
                    .symlink_metadata()
                    .is_err()
            );
        }
    }

    #[test]
    fn runit_links_into_the_default_runsvdir() {
        let t = TempDir::new();
        fs::create_dir_all(t.path().join("etc/runit/sv/cupsd")).unwrap();
        let svc = Svc::new(Init::Runit, t.path());
        assert_eq!(svc.enable("cupsd").unwrap(), Change::Enabled);
        assert_eq!(
            link_target(t.path(), "etc/runit/runsvdir/default/cupsd"),
            Path::new("/etc/runit/sv/cupsd")
        );
        assert_eq!(svc.disable("cupsd").unwrap(), Change::Disabled);
    }

    #[test]
    fn dinit_links_relative_into_boot_d() {
        let t = TempDir::new();
        t.put("etc/dinit.d/NetworkManager", "type = process\n");
        let svc = Svc::new(Init::Dinit, t.path());
        assert_eq!(svc.enable("NetworkManager").unwrap(), Change::Enabled);
        assert_eq!(
            link_target(t.path(), "etc/dinit.d/boot.d/NetworkManager"),
            Path::new("../NetworkManager")
        );
        // The link resolves.
        assert!(t.path().join("etc/dinit.d/boot.d/NetworkManager").exists());
    }

    #[test]
    fn s6_uses_the_default_bundle_contents() {
        let t = TempDir::new();
        fs::create_dir_all(t.path().join("etc/s6/sv/sshd")).unwrap();
        let svc = Svc::new(Init::S6, t.path());
        assert_eq!(svc.enable("sshd").unwrap(), Change::Enabled);
        assert!(
            t.path()
                .join("etc/s6/adminsv/default/contents.d/sshd")
                .exists()
        );
        assert_eq!(svc.disable("sshd").unwrap(), Change::Disabled);
        assert_eq!(svc.disable("sshd").unwrap(), Change::AlreadyDisabled);
    }

    #[test]
    fn unknown_and_unsafe_names_are_refused() {
        let t = TempDir::new();
        let svc = Svc::new(Init::OpenRc, t.path());
        let missing = svc.enable("nothing").unwrap_err().to_string();
        assert!(
            missing.contains("no service `nothing` for openrc"),
            "{missing}"
        );
        assert!(matches!(svc.enable("../x"), Err(Error::Usage(_))));
        assert!(matches!(svc.disable("a/b"), Err(Error::Usage(_))));
        assert!(!t.path().join("etc/runlevels").exists());
    }

    #[test]
    fn list_combines_installed_and_enabled() {
        let t = TempDir::new();
        t.put("etc/init.d/a", "");
        t.put("etc/init.d/b", "");
        t.put("etc/init.d/.keep", "");
        let svc = Svc::new(Init::OpenRc, t.path());
        svc.enable("b").unwrap();
        assert_eq!(
            svc.list().unwrap(),
            vec![
                Service {
                    name: "a".into(),
                    enabled: false,
                    state: State::Unknown
                },
                Service {
                    name: "b".into(),
                    enabled: true,
                    state: State::Unknown
                },
            ]
        );

        let t = TempDir::new();
        fs::create_dir_all(t.path().join("etc/runit/sv/x")).unwrap();
        fs::create_dir_all(t.path().join("etc/runit/sv/y")).unwrap();
        let svc = Svc::new(Init::Runit, t.path());
        svc.enable("y").unwrap();
        let list = svc.list().unwrap();
        assert_eq!(
            list.iter()
                .map(|s| (s.name.as_str(), s.enabled))
                .collect::<Vec<_>>(),
            [("x", false), ("y", true)]
        );

        // dinit: boot.d is a directory of links, not a service.
        let t = TempDir::new();
        t.put("etc/dinit.d/sshd", "");
        let svc = Svc::new(Init::Dinit, t.path());
        svc.enable("sshd").unwrap();
        let list = svc.list().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].enabled);
    }

    #[test]
    fn control_commands_per_init() {
        let t = TempDir::new();
        t.put("etc/init.d/sshd", "");
        let svc = Svc::new(Init::OpenRc, t.path());
        assert_eq!(
            svc.control_command(Action::Start, "sshd").unwrap(),
            (
                "rc-service".to_string(),
                vec!["sshd".to_string(), "start".to_string()]
            )
        );

        let t = TempDir::new();
        t.put("etc/dinit.d/sshd", "");
        let svc = Svc::new(Init::Dinit, t.path());
        assert_eq!(
            svc.control_command(Action::Stop, "sshd").unwrap(),
            (
                "dinitctl".to_string(),
                vec!["stop".to_string(), "sshd".to_string()]
            )
        );

        let t = TempDir::new();
        fs::create_dir_all(t.path().join("etc/s6/sv/sshd")).unwrap();
        let svc = Svc::new(Init::S6, t.path());
        assert_eq!(
            svc.control_command(Action::Start, "sshd").unwrap().1,
            ["-u", "change", "sshd"]
        );
        assert_eq!(
            svc.control_command(Action::Stop, "sshd").unwrap().1,
            ["-d", "change", "sshd"]
        );

        // runit supervises only what is enabled.
        let t = TempDir::new();
        fs::create_dir_all(t.path().join("etc/runit/sv/sshd")).unwrap();
        let svc = Svc::new(Init::Runit, t.path());
        let e = svc
            .control_command(Action::Start, "sshd")
            .unwrap_err()
            .to_string();
        assert!(e.contains("seg svc enable sshd"), "{e}");
        svc.enable("sshd").unwrap();
        let (program, args) = svc.control_command(Action::Start, "sshd").unwrap();
        assert_eq!(program, "sv");
        assert_eq!(args[0], "start");
        assert!(args[1].ends_with("etc/runit/runsvdir/default/sshd"));
    }

    #[test]
    fn rc_status_lines() {
        let text = " acpid                              [  started  ]\n \
                    syslog-ng                          [  stopped  ]\n \
                    foo                                [  crashed  ]\n \
                    bar                                [  started 00:00:12 (0) ]\n\
                    Runlevel: default\n\
                    Dynamic Runlevel: hotplugged\n \
                    baz                                [  starting  ]\n";
        let m = parse_rc_status(text);
        assert_eq!(m["acpid"], State::Started);
        assert_eq!(m["syslog-ng"], State::Stopped);
        assert_eq!(m["foo"], State::Crashed);
        assert_eq!(m["bar"], State::Started);
        assert_eq!(m["baz"], State::Unknown);
        assert_eq!(m.len(), 5);
    }

    #[test]
    fn sv_status_lines() {
        let text = "run: /etc/runit/runsvdir/default/sshd: (pid 1234) 5s; run: log: (pid 1233) 5s\n\
                    down: /etc/runit/runsvdir/default/cupsd: 3s, normally up\n\
                    fail: /etc/runit/runsvdir/default/gone: unable to change to service directory: file does not exist\n\
                    warning: /etc/runit/runsvdir/default/new: unable to open supervise/ok: file does not exist\n";
        let m = parse_sv_status(text);
        assert_eq!(m["sshd"], State::Started);
        assert_eq!(m["cupsd"], State::Stopped);
        assert_eq!(m["gone"], State::Crashed);
        assert_eq!(m["new"], State::Stopped);
    }

    #[test]
    fn dinitctl_list_lines() {
        let text = "[[+]     ] boot\n\
                    [{+}     ] NetworkManager (pid: 450)\n\
                    [{ }     ] cupsd\n\
                    not a service line\n";
        let m = parse_dinit_list(text);
        assert_eq!(m["boot"], State::Started);
        assert_eq!(m["NetworkManager"], State::Started);
        assert_eq!(m["cupsd"], State::Stopped);
        assert_eq!(m.len(), 3);
    }
}
