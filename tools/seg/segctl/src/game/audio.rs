//! `seg game audio on|off|status`: PipeWire's low-latency settings for
//! games. The two drop-in files the segctl package ships in
//! /usr/share/seg/pipewire are copied into the user's own PipeWire
//! configuration (no root), and removed again by `off`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Context, Error, Result};

pub const HELP: &str = "\
Usage: seg game audio on|off|status

on      lower PipeWire's buffer (256 samples at 48 kHz, about 5 ms) for you
off     back to PipeWire's defaults
status  say which it is

Lower latency means less delay between what happens in a game and what you
hear, at the price of more CPU work for the sound server and, on a weak or busy
computer, crackling. If sound crackles, run `seg game audio off`. It applies
the next time PipeWire starts: log out and in, or restart pipewire,
wireplumber and pipewire-pulse.
";

/// (file in the package, folder below ~/.config/pipewire, file name there)
const FILES: [(&str, &str); 2] = [
    ("pipewire.conf.d/99-seg-lowlatency.conf", "pipewire.conf.d"),
    (
        "pipewire-pulse.conf.d/99-seg-lowlatency.conf",
        "pipewire-pulse.conf.d",
    ),
];

pub const SHARE: &str = "/usr/share/seg/pipewire";

pub enum Action {
    On,
    Off,
    Status,
}

pub fn parse_args(parser: &mut lexopt::Parser) -> Result<Action> {
    use lexopt::prelude::*;
    let mut word: Option<String> = None;
    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => {
                out!("{HELP}");
                std::process::exit(0);
            }
            Value(v) if word.is_none() => word = Some(v.string()?),
            other => return Err(other.unexpected().into()),
        }
    }
    match word.as_deref() {
        Some("on") => Ok(Action::On),
        Some("off") => Ok(Action::Off),
        None | Some("status") => Ok(Action::Status),
        Some(other) => Err(Error::Usage(format!(
            "unknown audio command `{other}` (on, off or status)"
        ))),
    }
}

fn targets(config: &Path) -> Vec<(PathBuf, PathBuf)> {
    FILES
        .iter()
        .map(|(src, dir)| {
            (
                PathBuf::from(src),
                config
                    .join("pipewire")
                    .join(dir)
                    .join("99-seg-lowlatency.conf"),
            )
        })
        .collect()
}

pub fn is_on(config: &Path) -> bool {
    targets(config).iter().all(|(_, t)| t.exists())
}

pub fn enable(share: &Path, config: &Path) -> Result<()> {
    for (src, target) in targets(config) {
        let from = share.join(src);
        let text = fs::read_to_string(&from).context(format!("reading {}", from.display()))?;
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir).context(format!("creating {}", dir.display()))?;
        }
        fs::write(&target, text).context(format!("writing {}", target.display()))?;
    }
    Ok(())
}

pub fn disable(config: &Path) -> Result<bool> {
    let mut removed = false;
    for (_, target) in targets(config) {
        match fs::remove_file(&target) {
            Ok(()) => removed = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(format!("removing {}", target.display()), e)),
        }
    }
    Ok(removed)
}

pub fn run(action: Action) -> Result<u8> {
    let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => crate::sys::invoking_home()
            .ok_or_else(|| Error::other("cannot tell where your configuration is (no HOME)"))?
            .join(".config"),
    };
    match action {
        Action::Status => outln!(
            "low-latency audio: {}",
            if is_on(&config) { "on" } else { "off" }
        ),
        Action::On => {
            enable(Path::new(SHARE), &config)?;
            outln!(
                "low-latency audio is on. Log out and in (or restart pipewire, wireplumber and pipewire-pulse). If sound crackles: seg game audio off"
            );
        }
        Action::Off => {
            outln!(
                "{}",
                if disable(&config)? {
                    "low-latency audio is off. Log out and in (or restart pipewire) to go back to the defaults."
                } else {
                    "low-latency audio was not on."
                }
            );
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    fn share() -> TempDir {
        let t = TempDir::new();
        t.put(
            "pipewire.conf.d/99-seg-lowlatency.conf",
            "context.properties = {}\n",
        );
        t.put(
            "pipewire-pulse.conf.d/99-seg-lowlatency.conf",
            "pulse.properties = {}\n",
        );
        t
    }

    #[test]
    fn on_copies_both_files_and_off_removes_them() {
        let (s, c) = (share(), TempDir::new());
        assert!(!is_on(c.path()));
        enable(s.path(), c.path()).unwrap();
        assert!(is_on(c.path()));
        assert_eq!(
            fs::read_to_string(
                c.path()
                    .join("pipewire/pipewire-pulse.conf.d/99-seg-lowlatency.conf")
            )
            .unwrap(),
            "pulse.properties = {}\n"
        );
        // Twice is fine.
        enable(s.path(), c.path()).unwrap();
        assert!(disable(c.path()).unwrap());
        assert!(!is_on(c.path()));
        assert!(!disable(c.path()).unwrap(), "nothing left to remove");
    }

    #[test]
    fn a_missing_package_file_is_an_error() {
        let c = TempDir::new();
        let e = enable(Path::new("/nowhere"), c.path())
            .unwrap_err()
            .to_string();
        assert!(e.contains("reading /nowhere/pipewire.conf.d"), "{e}");
    }

    #[test]
    fn other_files_in_the_config_folders_are_left_alone() {
        let (s, c) = (share(), TempDir::new());
        let mine = c.put("pipewire/pipewire.conf.d/50-mine.conf", "x\n");
        enable(s.path(), c.path()).unwrap();
        disable(c.path()).unwrap();
        assert!(mine.exists());
    }

    #[test]
    fn the_shipped_files_say_what_the_help_says() {
        let pw = include_str!("../../../dist/pipewire/pipewire.conf.d/99-seg-lowlatency.conf");
        assert!(pw.contains("default.clock.quantum"));
        assert!(pw.contains("256"));
        let pulse =
            include_str!("../../../dist/pipewire/pipewire-pulse.conf.d/99-seg-lowlatency.conf");
        assert!(pulse.contains("pulse.min.req"));
        assert!(pulse.contains("256/48000"));
    }
}
