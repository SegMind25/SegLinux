//! Small helpers over /proc, /sys and child processes.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Context, Error, Result};

/// The effective user id, from `Uid: real effective saved fs` in
/// /proc/self/status (std has no geteuid, and this needs no libc crate).
pub fn parse_euid(status: &str) -> Option<u32> {
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

pub fn euid() -> Result<u32> {
    let status = fs::read_to_string("/proc/self/status").context("reading /proc/self/status")?;
    parse_euid(&status)
        .ok_or_else(|| Error::other("cannot read the user id from /proc/self/status"))
}

/// Fails with a clear message unless running as root. `what` names the
/// command in the message, for example "`seg zram on`".
pub fn require_root(what: &str) -> Result<()> {
    if euid()? == 0 {
        Ok(())
    } else {
        Err(Error::NeedRoot(what.to_string()))
    }
}

/// A file's trimmed contents, or None when it cannot be read.
pub fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn read(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    fs::read_to_string(path).context(format!("reading {}", path.display()))
}

pub fn write(path: impl AsRef<Path>, contents: &str) -> Result<()> {
    let path = path.as_ref();
    fs::write(path, contents).context(format!("writing {}", path.display()))
}

/// Replace a file in one step, so a crash never leaves half a configuration.
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context(format!("creating {}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".seg-tmp");
    let tmp = PathBuf::from(tmp);
    write(&tmp, contents)?;
    fs::rename(&tmp, path).context(format!("replacing {}", path.display()))
}

pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run a program and capture its output. Only a program that cannot be
/// started is an error; a failing exit status is reported in `ok`.
pub fn run(program: &str, args: &[&str]) -> Result<Output> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::Command {
            program: program.to_string(),
            detail: format!("cannot start it: {e}"),
        })?;
    Ok(Output {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Like [`run`] but a failing exit status is an error carrying stderr.
pub fn run_ok(program: &str, args: &[&str]) -> Result<String> {
    let out = run(program, args)?;
    if out.ok {
        Ok(out.stdout)
    } else {
        Err(Error::Command {
            program: format!("{program} {}", args.join(" ")),
            detail: out.stderr.trim().to_string(),
        })
    }
}

/// Run a program with the terminal attached, so its output and prompts show.
pub fn run_inherit(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| Error::Command {
            program: program.to_string(),
            detail: format!("cannot start it: {e}"),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Command {
            program: format!("{program} {}", args.join(" ")),
            detail: format!("failed with {status}"),
        })
    }
}

pub fn which(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// 1536 KiB -> "1.5 MiB".
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    match UNITS.get(unit) {
        Some(&"B") | None => format!("{bytes} B"),
        Some(u) => format!("{value:.1} {u}"),
    }
}

/// 8100 s -> "2h 15m".
pub fn human_duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{secs}s"),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// Home directory of `user` from passwd-formatted text.
pub fn parse_passwd_home(passwd: &str, user: &str) -> Option<PathBuf> {
    passwd.lines().find_map(|line| {
        let mut f = line.split(':');
        (f.next()? == user).then(|| f.nth(4).map(PathBuf::from))?
    })
}

/// The home that `seg` reports on: the user who ran `sudo`, else the caller's.
pub fn invoking_home() -> Option<PathBuf> {
    if let Some(user) = std::env::var_os("SUDO_USER").and_then(|u| u.into_string().ok())
        && user != "root"
        && let Ok(passwd) = fs::read_to_string("/etc/passwd")
        && let Some(home) = parse_passwd_home(&passwd, &user)
    {
        return Some(home);
    }
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Total size of the files under `path`, not following links. Unreadable
/// entries count as zero: this is for reports, not for accounting.
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries.flatten().map(|e| dir_size(&e.path())).sum()
}

/// Write to stdout; a closed pipe ends the program quietly, as other tools do.
pub fn emit(args: std::fmt::Arguments, newline: bool) {
    use std::io::{ErrorKind, Write};
    let mut out = std::io::stdout().lock();
    let result = out.write_fmt(args).and_then(|()| {
        if newline {
            out.write_all(b"\n")
        } else {
            Ok(())
        }
    });
    if let Err(e) = result
        && e.kind() == ErrorKind::BrokenPipe
    {
        std::process::exit(0);
    }
}

/// Write a line to stderr, ignoring a stderr that cannot be written.
pub fn emit_err(args: std::fmt::Arguments) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{args}");
}

/// Colors only on a terminal, and not when NO_COLOR is set.
pub fn use_color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub fn bold(text: &str) -> String {
    if use_color() {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
pub mod testutil {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A directory under the system temp dir, removed on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "seg-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }

        /// Write `contents` to `rel` below the directory, creating parents.
        pub fn put(&self, rel: &str, contents: &str) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, contents).unwrap();
            p
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::TempDir;
    use super::*;

    #[test]
    fn euid_is_the_second_uid_field() {
        let status = "Name:\tsh\nUid:\t1000\t0\t0\t0\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_euid(status), Some(0));
        assert_eq!(parse_euid("Uid:\t1000\t1000\t1000\t1000\n"), Some(1000));
        assert_eq!(parse_euid("Name:\tsh\n"), None);
        assert_eq!(parse_euid("Uid:\tx\ty\n"), None);
    }

    #[test]
    fn sizes_and_durations() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
        assert_eq!(human_duration(42), "42s");
        assert_eq!(human_duration(600), "10m");
        assert_eq!(human_duration(8100), "2h 15m");
        assert_eq!(human_duration(90_000), "1d 1h");
    }

    #[test]
    fn home_from_passwd() {
        let passwd =
            "root:x:0:0:root:/root:/bin/bash\nalice:x:1000:1000:Alice:/home/alice:/bin/zsh\n";
        assert_eq!(
            parse_passwd_home(passwd, "alice"),
            Some(PathBuf::from("/home/alice"))
        );
        assert_eq!(parse_passwd_home(passwd, "bob"), None);
        assert_eq!(parse_passwd_home("broken", "broken"), None);
    }

    #[test]
    fn dir_size_adds_files_and_ignores_links() {
        let t = TempDir::new();
        t.put("a/one", "12345");
        t.put("a/b/two", "123");
        std::os::unix::fs::symlink(t.path().join("a"), t.path().join("link")).unwrap();
        assert_eq!(dir_size(&t.path().join("a")), 8);
        assert_eq!(dir_size(t.path()), 8);
        assert_eq!(dir_size(&t.path().join("missing")), 0);
    }

    #[test]
    fn atomic_write_replaces_and_creates_parents() {
        let t = TempDir::new();
        let p = t.path().join("etc/seg/tune.conf");
        write_atomic(&p, "mode = light\n").unwrap();
        write_atomic(&p, "mode = balanced\n").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "mode = balanced\n");
        assert!(!t.path().join("etc/seg/tune.conf.seg-tmp").exists());
    }

    #[test]
    fn missing_program_is_an_error_not_a_panic() {
        let e = run("definitely-not-a-program-seg", &[]).err().unwrap();
        assert!(matches!(e, Error::Command { .. }));
        assert!(run("true", &[]).unwrap().ok);
        assert!(!run("false", &[]).unwrap().ok);
        assert!(run_ok("false", &[]).is_err());
    }
}
