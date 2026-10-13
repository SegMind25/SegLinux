//! Carry out a plan, logging like Windows Setup does in `Panther\setupact.log`
//! (everything) and `setuperr.log` (errors only).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use seglinux_core::cmd::Cmd;
use seglinux_core::in_root;
use seglinux_core::init::{InitSystem, ServiceChange};

use crate::plan::{Action, Pass, Plan, Secret};

pub const LOG_DIR: &str = "/var/log/seglinux";

pub struct Log {
    dir: PathBuf,
    act: File,
    err: File,
    started: Instant,
}

impl Log {
    pub fn open(dir: &Path) -> Result<Log> {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let open = |name: &str| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(name))
                .with_context(|| format!("cannot open {}", dir.join(name).display()))
        };
        Ok(Log {
            dir: dir.to_path_buf(),
            act: open("setupact.log")?,
            err: open("setuperr.log")?,
            started: Instant::now(),
        })
    }

    fn stamp(&self) -> String {
        format!("[{:>8.1}s]", self.started.elapsed().as_secs_f64())
    }

    pub fn info(&mut self, msg: &str) {
        let line = format!("{} {msg}\n", self.stamp());
        let _ = self.act.write_all(line.as_bytes());
    }

    pub fn error(&mut self, msg: &str) {
        let line = format!("{} ERROR {msg}\n", self.stamp());
        let _ = self.act.write_all(line.as_bytes());
        let _ = self.err.write_all(line.as_bytes());
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub pass: Pass,
    /// Steps finished so far and in total.
    pub done: usize,
    pub total: usize,
    pub description: String,
}

impl Progress {
    pub fn percent(&self) -> u16 {
        (self.done * 100)
            .checked_div(self.total)
            .map_or(100, |p| p as u16)
    }
}

/// What `seglinux-setup install --unattend FILE --progress json` prints, one
/// JSON object per line. The graphical installer reads these.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    Progress {
        /// Windows configuration pass name (`windowsPE`, `specialize`, ...).
        pass: String,
        /// What the pass does, for people.
        title: String,
        done: usize,
        total: usize,
        description: String,
    },
    /// SegLinux is installed.
    Done,
    /// Setup stopped.
    Error { message: String },
}

impl From<&Progress> for Event {
    fn from(p: &Progress) -> Event {
        Event::Progress {
            pass: p.pass.windows_name().into(),
            title: p.pass.title().into(),
            done: p.done,
            total: p.total,
            description: p.description.clone(),
        }
    }
}

impl Event {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("events serialize")
    }

    /// Parses one line of the JSON progress output; other lines are `None`.
    pub fn parse(line: &str) -> Option<Event> {
        serde_json::from_str(line.trim()).ok()
    }
}

pub struct Executor<'a> {
    target: PathBuf,
    init: InitSystem,
    log: &'a mut Log,
}

impl<'a> Executor<'a> {
    pub fn new(target: &Path, init: InitSystem, log: &'a mut Log) -> Self {
        Self {
            target: target.to_path_buf(),
            init,
            log,
        }
    }

    pub fn run(&mut self, plan: &Plan, progress: &mut dyn FnMut(Progress)) -> Result<()> {
        let total = plan.step_count();
        let mut done = 0;
        self.log.info(&format!(
            "SegLinux Setup {} starting",
            env!("CARGO_PKG_VERSION")
        ));
        for line in plan
            .to_string()
            .lines()
            .take_while(|l| !l.starts_with("=="))
        {
            self.log.info(line);
        }
        for phase in &plan.phases {
            self.log.info(&format!(
                "== Pass {}: {}",
                phase.pass.windows_name(),
                phase.pass.title()
            ));
            for step in &phase.steps {
                progress(Progress {
                    pass: phase.pass,
                    done,
                    total,
                    description: step.description.clone(),
                });
                self.log.info(&format!("* {}", step.description));
                for action in &step.actions {
                    if let Err(e) = self.apply(action) {
                        self.log.error(&format!("{}: {e:#}", step.description));
                        return Err(e.context(step.description.clone()));
                    }
                }
                done += 1;
            }
        }
        progress(Progress {
            pass: Pass::SetupComplete,
            done: total,
            total,
            description: "Installation complete".into(),
        });
        self.log.info("SegLinux Setup finished successfully");
        Ok(())
    }

    fn path(&self, rel: &str) -> PathBuf {
        in_root(&self.target, rel)
    }

    /// `cmd` run inside the new system. Like arch-chroot, it gets its own
    /// PID namespace, so whatever it leaves running ends with it: pacman-key
    /// starts gpg-agent, which kept /dev/null open and made the final
    /// unmount fail with "target is busy".
    fn chroot_cmd(&self, cmd: &Cmd) -> Cmd {
        let mut c = cmd.clone();
        c.program = "unshare".into();
        c.args = ["--fork", "--pid", "--kill-child", "--", "chroot"]
            .into_iter()
            .map(String::from)
            .chain(std::iter::once(self.target.to_string_lossy().into_owned()))
            .chain(std::iter::once(cmd.program.clone()))
            .chain(cmd.args.iter().cloned())
            .collect();
        c
    }

    fn exec(&mut self, cmd: &Cmd, may_fail: bool) -> Result<()> {
        self.log.info(&format!("$ {cmd}"));
        let (ok, output) = cmd.output_combined()?;
        for line in output.lines() {
            self.log.info(&format!("  {line}"));
        }
        if !ok {
            if may_fail {
                self.log.info("  (failed; not required, continuing)");
            } else {
                let tail: Vec<&str> = output
                    .lines()
                    .rev()
                    .take(15)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                bail!("`{}` failed:\n{}", cmd, tail.join("\n"));
            }
        }
        Ok(())
    }

    pub fn apply(&mut self, action: &Action) -> Result<()> {
        match action {
            Action::Host { cmd, may_fail } => self.exec(cmd, *may_fail),
            Action::Chroot { cmd, may_fail } => {
                let c = self.chroot_cmd(cmd);
                self.exec(&c, *may_fail)
            }
            Action::WriteFile {
                path,
                contents,
                mode,
            } => {
                self.log.info(&format!("write {path}"));
                let p = self.path(path);
                if let Some(parent) = p.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&p, contents).with_context(|| format!("cannot write {path}"))?;
                fs::set_permissions(&p, fs::Permissions::from_mode(*mode))?;
                Ok(())
            }
            Action::Symlink { target, link } => {
                self.log.info(&format!("link {link} -> {target}"));
                let p = self.path(link);
                if p.symlink_metadata().is_ok() {
                    fs::remove_file(&p)?;
                }
                if let Some(parent) = p.parent() {
                    fs::create_dir_all(parent)?;
                }
                symlink(target, &p).with_context(|| format!("cannot create {link}"))
            }
            Action::SetKey { path, key, value } => {
                self.log.info(&format!("set {key}={value} in {path}"));
                let p = self.path(path);
                let old = fs::read_to_string(&p).unwrap_or_default();
                if let Some(parent) = p.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&p, set_key(&old, key, value))
                    .with_context(|| format!("cannot write {path}"))
            }
            Action::EnableService(name) => {
                let change = self.init.enable_service(&self.target, name)?;
                self.log
                    .info(&format!("enable {name} ({}): {change:?}", self.init));
                Ok(())
            }
            Action::DisableService(name) => {
                let change = self.init.disable_service(&self.target, name)?;
                if change == ServiceChange::Missing {
                    self.log.info(&format!("disable {name}: not installed"));
                } else {
                    self.log
                        .info(&format!("disable {name} ({}): {change:?}", self.init));
                }
                Ok(())
            }
            Action::AddUser {
                name,
                full_name,
                groups,
            } => {
                let existing = existing_groups(&self.path("/etc/group"));
                let (keep, skipped): (Vec<&String>, Vec<&String>) =
                    groups.iter().partition(|g| existing.contains(*g));
                if !skipped.is_empty() {
                    self.log.info(&format!(
                        "groups not present in the new system, skipped: {}",
                        skipped
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
                let mut cmd = Cmd::new("useradd").args(["-m", "-s", "/bin/bash"]);
                if !full_name.is_empty() {
                    cmd = cmd.args(["-c", full_name]);
                }
                if !keep.is_empty() {
                    cmd = cmd.arg("-G").arg(
                        keep.iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                }
                let c = self.chroot_cmd(&cmd.arg(name));
                self.exec(&c, false)
            }
            Action::SetPassword { user, secret } => {
                let cmd = match secret {
                    Secret::Plain(p) => Cmd::new("chpasswd").secret_stdin(format!("{user}:{p}\n")),
                    Secret::Hash(h) => Cmd::new("chpasswd")
                        .arg("-e")
                        .secret_stdin(format!("{user}:{h}\n")),
                    Secret::Lock => Cmd::new("passwd").args(["-l", user]),
                };
                let c = self.chroot_cmd(&cmd);
                self.exec(&c, false)
            }
            Action::RunScripts { dir } => {
                let host_dir = self.path(dir);
                let mut scripts: Vec<PathBuf> = fs::read_dir(&host_dir)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .filter(|p| {
                                p.metadata().is_ok_and(|m| {
                                    m.is_file() && m.permissions().mode() & 0o111 != 0
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                scripts.sort();
                if scripts.is_empty() {
                    self.log.info(&format!("no scripts in {dir}"));
                }
                for script in scripts {
                    let inside = format!(
                        "{dir}/{}",
                        script.file_name().unwrap_or_default().to_string_lossy()
                    );
                    let c = self.chroot_cmd(&Cmd::new(inside));
                    // Like SetupComplete.cmd, a failing script does not fail Setup.
                    self.exec(&c, true)?;
                }
                Ok(())
            }
            Action::SaveLogs { dir } => {
                let dest = self.path(dir);
                fs::create_dir_all(&dest)?;
                self.log.info(&format!("copying logs to {dir}"));
                for name in ["setupact.log", "setuperr.log"] {
                    let src = self.log.dir().join(name);
                    if src.exists() {
                        fs::copy(&src, dest.join(name))?;
                    }
                }
                Ok(())
            }
        }
    }
}

fn existing_groups(group_file: &Path) -> Vec<String> {
    fs::read_to_string(group_file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split(':').next())
        .filter(|g| !g.is_empty())
        .map(str::to_string)
        .collect()
}

/// Set `key=value` in a shell-style configuration file. An existing active
/// assignment is replaced; otherwise a commented-out one (`#key=...`) is
/// replaced; otherwise the line is appended.
pub fn set_key(text: &str, key: &str, value: &str) -> String {
    let assignment = format!("{key}={value}");
    let is_active = |l: &str| l.trim_start().starts_with(&format!("{key}="));
    let is_commented = |l: &str| {
        let t = l.trim_start();
        t.starts_with('#')
            && t.trim_start_matches('#')
                .trim_start()
                .starts_with(&format!("{key}="))
    };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if let Some(i) = lines.iter().position(|l| is_active(l)) {
        lines[i] = assignment;
    } else if let Some(i) = lines.iter().position(|l| is_commented(l)) {
        lines[i] = assignment;
    } else {
        lines.push(assignment);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod event_tests {
    use super::*;

    #[test]
    fn events_round_trip_as_json_lines() {
        let p = Progress {
            pass: Pass::Specialize,
            done: 3,
            total: 40,
            description: "Setting the time zone to \"UTC\"".into(),
        };
        let line = Event::from(&p).to_json();
        assert!(!line.contains('\n'));
        assert!(line.starts_with(r#"{"event":"progress","pass":"specialize""#));
        assert_eq!(Event::parse(&line), Some(Event::from(&p)));
        assert_eq!(Event::parse(r#"{"event":"done"}"#), Some(Event::Done));
        assert_eq!(
            Event::parse(r#" {"event":"error","message":"no disk"} "#),
            Some(Event::Error {
                message: "no disk".into()
            })
        );
        assert_eq!(Event::parse("Using answer file /tmp/a.toml"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Phase, Step, Summary};

    #[test]
    fn set_key_variants() {
        let grub = "GRUB_TIMEOUT=5\nGRUB_DISTRIBUTOR=\"Artix\"\n#GRUB_DISABLE_OS_PROBER=false\n";
        let out = set_key(grub, "GRUB_DISTRIBUTOR", "\"SegLinux\"");
        assert_eq!(
            out,
            "GRUB_TIMEOUT=5\nGRUB_DISTRIBUTOR=\"SegLinux\"\n#GRUB_DISABLE_OS_PROBER=false\n"
        );
        let out = set_key(&out, "GRUB_DISABLE_OS_PROBER", "false");
        assert!(out.contains("\nGRUB_DISABLE_OS_PROBER=false\n"));
        assert!(!out.contains('#'));
        assert_eq!(set_key("", "keymap", "\"fr\""), "keymap=\"fr\"\n");
        // Similar prefixes are not confused.
        assert_eq!(
            set_key("clock_hctosys=\"YES\"\n", "clock", "\"UTC\""),
            "clock_hctosys=\"YES\"\nclock=\"UTC\"\n"
        );
    }

    #[test]
    fn group_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("group");
        fs::write(&f, "root:x:0:\nwheel:x:998:\naudio:x:995:\n").unwrap();
        assert_eq!(existing_groups(&f), vec!["root", "wheel", "audio"]);
        assert!(existing_groups(&dir.path().join("missing")).is_empty());
    }

    /// Runs file-system actions for real against a temporary "target".
    #[test]
    fn executes_file_actions_in_target() {
        let target = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let root = target.path();
        fs::create_dir_all(root.join("etc/init.d")).unwrap();
        fs::write(root.join("etc/init.d/sddm"), "").unwrap();
        fs::create_dir_all(root.join("etc/default")).unwrap();
        fs::write(
            root.join("etc/default/grub"),
            "GRUB_DISTRIBUTOR=\"Artix\"\n",
        )
        .unwrap();

        let plan = Plan {
            target: root.to_path_buf(),
            summary: Summary {
                disk: crate::disk::Disk {
                    path: "/dev/null".into(),
                    size: 0,
                    model: String::new(),
                    removable: false,
                    read_only: false,
                    transport: String::new(),
                },
                layout: crate::disk::Layout::new(
                    "/dev/null",
                    crate::answer::Filesystem::Ext4,
                    0,
                    None,
                    &mut crate::ids::Sequential::default(),
                ),
                hostname: "test".into(),
                image: "/dev/null".into(),
                firmware: crate::facts::Firmware::Uefi,
                init: InitSystem::OpenRc,
                users: vec![],
                online_packages: 0,
            },
            phases: vec![Phase {
                pass: Pass::Specialize,
                steps: vec![Step {
                    description: "configure".into(),
                    actions: vec![
                        Action::WriteFile {
                            path: "/etc/sudoers.d/10-seglinux".into(),
                            contents: "%wheel ALL=(ALL:ALL) ALL\n".into(),
                            mode: 0o440,
                        },
                        Action::Symlink {
                            target: "/usr/share/zoneinfo/UTC".into(),
                            link: "/etc/localtime".into(),
                        },
                        Action::SetKey {
                            path: "/etc/default/grub".into(),
                            key: "GRUB_DISTRIBUTOR".into(),
                            value: "\"SegLinux\"".into(),
                        },
                        Action::EnableService("sddm".into()),
                        Action::EnableService("not-installed".into()),
                        Action::Host {
                            cmd: Cmd::new("true"),
                            may_fail: false,
                        },
                        Action::Host {
                            cmd: Cmd::new("false"),
                            may_fail: true,
                        },
                        Action::SaveLogs {
                            dir: "/var/log/seglinux".into(),
                        },
                    ],
                }],
            }],
        };
        let mut log = Log::open(logs.path()).unwrap();
        let mut seen = Vec::new();
        Executor::new(root, InitSystem::OpenRc, &mut log)
            .run(&plan, &mut |p| seen.push(p.percent()))
            .unwrap();
        assert_eq!(seen, vec![0, 100]);

        let sudoers = root.join("etc/sudoers.d/10-seglinux");
        assert_eq!(
            sudoers.metadata().unwrap().permissions().mode() & 0o777,
            0o440
        );
        assert_eq!(
            fs::read_link(root.join("etc/localtime")).unwrap(),
            Path::new("/usr/share/zoneinfo/UTC")
        );
        assert_eq!(
            fs::read_to_string(root.join("etc/default/grub")).unwrap(),
            "GRUB_DISTRIBUTOR=\"SegLinux\"\n"
        );
        assert!(
            root.join("etc/runlevels/default/sddm")
                .symlink_metadata()
                .is_ok()
        );
        let act = fs::read_to_string(root.join("var/log/seglinux/setupact.log")).unwrap();
        assert!(act.contains("* configure"));
        assert!(act.contains("enable not-installed (openrc): Missing"));
        assert!(act.contains("not required, continuing"));
    }

    #[test]
    fn failing_required_command_stops_and_logs_error() {
        let target = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let mut log = Log::open(logs.path()).unwrap();
        let mut ex = Executor::new(target.path(), InitSystem::OpenRc, &mut log);
        let err = ex
            .apply(&Action::Host {
                cmd: Cmd::new("sh").args(["-c", "echo boom >&2; exit 3"]),
                may_fail: false,
            })
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn chroot_wrapping() {
        let logs = tempfile::tempdir().unwrap();
        let mut log = Log::open(logs.path()).unwrap();
        let ex = Executor::new(Path::new("/mnt/seglinux"), InitSystem::OpenRc, &mut log);
        let c = ex.chroot_cmd(&Cmd::new("chpasswd").arg("-e").secret_stdin("a:b\n"));
        assert_eq!(
            c.to_string(),
            "unshare --fork --pid --kill-child -- chroot /mnt/seglinux chpasswd -e <<< '********'"
        );
    }
}
