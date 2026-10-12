//! A plain description of an external command.

use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    /// Data written to the command's standard input.
    pub stdin: Option<String>,
    /// Hide `stdin` from logs and dry runs (passwords).
    pub secret_stdin: bool,
}

impl Cmd {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            ..Default::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        self.env.retain(|(k, _)| *k != key);
        self.env.push((key, value.into()));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn stdin(mut self, data: impl Into<String>) -> Self {
        self.stdin = Some(data.into());
        self
    }

    pub fn secret_stdin(mut self, data: impl Into<String>) -> Self {
        self.stdin = Some(data.into());
        self.secret_stdin = true;
        self
    }

    /// Value of an environment variable set on this command.
    pub fn get_env(&self, key: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Build a `std::process::Command` (stdio is left to the caller).
    pub fn to_command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args);
        for (k, v) in &self.env {
            c.env(k, v);
        }
        if let Some(dir) = &self.cwd {
            c.current_dir(dir);
        }
        c
    }

    /// Run to completion with inherited stdout/stderr, failing on a non-zero
    /// exit status.
    pub fn run(&self) -> Result<()> {
        let status = self
            .spawn_with_stdin(Stdio::inherit(), Stdio::inherit())?
            .wait()?;
        if !status.success() {
            bail!("`{self}` failed with {status}");
        }
        Ok(())
    }

    /// Run to completion and return stdout. Stderr is included in the error
    /// on failure.
    pub fn output(&self) -> Result<String> {
        let out = self
            .spawn_with_stdin(Stdio::piped(), Stdio::piped())?
            .wait_with_output()?;
        if !out.status.success() {
            bail!(
                "`{self}` failed with {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run and capture stdout and stderr together, for logging.
    pub fn output_combined(&self) -> Result<(bool, String)> {
        let out = self
            .spawn_with_stdin(Stdio::piped(), Stdio::piped())?
            .wait_with_output()?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok((out.status.success(), text))
    }

    fn spawn_with_stdin(&self, stdout: Stdio, stderr: Stdio) -> Result<std::process::Child> {
        let mut c = self.to_command();
        c.stdout(stdout).stderr(stderr);
        c.stdin(if self.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = c
            .spawn()
            .with_context(|| format!("cannot start `{}`", self.program))?;
        if let Some(data) = &self.stdin {
            let mut pipe = child.stdin.take().expect("stdin was piped");
            pipe.write_all(data.as_bytes())?;
        }
        Ok(child)
    }
}

impl fmt::Display for Cmd {
    /// Shell-quoted command line, with environment assignments first, so it
    /// can be pasted into a terminal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| format!("{k}={}", shell_quote(v)))
            .collect();
        parts.push(shell_quote(&self.program));
        parts.extend(self.args.iter().map(|a| shell_quote(a)));
        f.write_str(&parts.join(" "))?;
        if let Some(data) = &self.stdin {
            if self.secret_stdin {
                f.write_str(" <<< '********'")?;
            } else {
                write!(f, " <<< {}", shell_quote(data.trim_end()))?;
            }
        }
        Ok(())
    }
}

/// Quote a string for POSIX shells, only when needed.
pub fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_quotes_only_when_needed() {
        let c = Cmd::new("wine")
            .arg("C:\\Program Files\\App\\app.exe")
            .arg("--flag")
            .env(
                "WINEPREFIX",
                "/home/me/.local/share/seglinux/prefixes/default",
            );
        assert_eq!(
            c.to_string(),
            "WINEPREFIX=/home/me/.local/share/seglinux/prefixes/default wine 'C:\\Program Files\\App\\app.exe' --flag"
        );
    }

    #[test]
    fn quote_single_quotes() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn env_overrides_replace_earlier_values() {
        let c = Cmd::new("x").env("A", "1").env("A", "2");
        assert_eq!(c.env, vec![("A".to_string(), "2".to_string())]);
        assert_eq!(c.get_env("A"), Some("2"));
    }

    #[test]
    fn secrets_are_masked() {
        let c = Cmd::new("chpasswd").secret_stdin("alice:hunter2\n");
        assert_eq!(c.to_string(), "chpasswd <<< '********'");
        assert!(!c.to_string().contains("hunter2"));
    }

    #[test]
    fn run_and_capture() {
        assert_eq!(Cmd::new("echo").arg("hello").output().unwrap(), "hello\n");
        assert_eq!(Cmd::new("cat").stdin("piped").output().unwrap(), "piped");
        assert!(Cmd::new("false").run().is_err());
        let (ok, text) = Cmd::new("sh")
            .args(["-c", "echo out; echo err >&2"])
            .output_combined()
            .unwrap();
        assert!(ok);
        assert_eq!(text, "out\nerr\n");
    }
}
