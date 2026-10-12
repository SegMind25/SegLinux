//! binfmt_misc: let the kernel start `.exe` files through segrun, so
//! `./setup.exe` works in a terminal just like on Windows.
//!
//! The persistent registration is shipped as `/etc/binfmt.d/wine.conf`,
//! which replaces the `wine.conf` installed by the wine package (files in
//! /etc override files of the same name in /usr/lib). This module handles
//! the live state in `/proc/sys/fs/binfmt_misc`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub const BINFMT_DIR: &str = "/proc/sys/fs/binfmt_misc";
pub const ENTRY_NAME: &str = "segrun";

/// Registration string for `register` / binfmt.d.
pub fn rule(interpreter: &str) -> String {
    format!(":{ENTRY_NAME}:M::MZ::{interpreter}:")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub enabled: bool,
    pub interpreter: String,
    pub magic: String,
}

/// Parse one `/proc/sys/fs/binfmt_misc/<name>` file.
pub fn parse_entry(name: &str, text: &str) -> Entry {
    let mut e = Entry {
        name: name.to_string(),
        enabled: false,
        interpreter: String::new(),
        magic: String::new(),
    };
    for line in text.lines() {
        if line == "enabled" {
            e.enabled = true;
        } else if let Some(v) = line.strip_prefix("interpreter ") {
            e.interpreter = v.to_string();
        } else if let Some(v) = line.strip_prefix("magic ") {
            e.magic = v.to_string();
        }
    }
    e
}

/// Registered handlers for files starting with `MZ` (hex `4d5a`).
pub fn mz_handlers(dir: &Path) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(out);
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "register" || name == "status" {
            continue;
        }
        if let Ok(text) = fs::read_to_string(entry.path()) {
            let e = parse_entry(&name, &text);
            if e.magic.starts_with("4d5a") {
                out.push(e);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn register(interpreter: &Path) -> Result<()> {
    let dir = PathBuf::from(BINFMT_DIR);
    if !dir.join("register").exists() {
        bail!("binfmt_misc is not mounted at {BINFMT_DIR}");
    }
    if dir.join(ENTRY_NAME).exists() {
        return Ok(());
    }
    fs::write(dir.join("register"), rule(&interpreter.to_string_lossy()))
        .context("cannot register the handler (are you root?)")
}

pub fn unregister() -> Result<()> {
    let path = PathBuf::from(BINFMT_DIR).join(ENTRY_NAME);
    if path.exists() {
        fs::write(&path, "-1").context("cannot remove the handler (are you root?)")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_format() {
        assert_eq!(rule("/usr/bin/segrun"), ":segrun:M::MZ::/usr/bin/segrun:");
    }

    #[test]
    fn parses_proc_entry_and_finds_mz_handlers() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("register"), "").unwrap();
        fs::write(
            dir.path().join("segrun"),
            "enabled\ninterpreter /usr/bin/segrun\nflags: \noffset 0\nmagic 4d5a\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("python"),
            "enabled\ninterpreter /usr/bin/python\nflags: \noffset 0\nmagic 0d0a\n",
        )
        .unwrap();
        let found = mz_handlers(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].interpreter, "/usr/bin/segrun");
        assert!(found[0].enabled);
    }
}
