//! `seg game`: what a game needs from the system, and the commands around it.

pub mod apply;
pub mod audio;
pub mod boost;
pub mod check;
pub mod dbus;
pub mod hw;
pub mod proc;
pub mod profile;
pub mod run;
pub mod setup;
pub mod toml;
pub mod unsafe_boost;

use crate::error::{Error, Result};
use crate::sys;

pub const HELP: &str = "\
Usage: seg game <command>

  run [options] -- COMMAND   run a game with the computer set up for it
                             (Steam launch option: seg game run -- %command%)
  check                      is this computer ready for games? PASS/FAIL with fixes
  setup [--yes]              the graphics driver packages this computer needs
  profiles [FILE]            check and list the per-game profiles
  apply                      apply the gaming sysctls and udev rules (seg-game service)
  audio on|off|status        low-latency PipeWire settings for games
  unsafe-boost on|off|status mitigations=off: faster, less safe (read the warning)

`seg game <command> --help` tells more.
";

/// Dispatch `seg game ...`. Returns the exit status.
pub fn run(parser: &mut lexopt::Parser) -> Result<u8> {
    use lexopt::prelude::*;
    let command = match parser.next()? {
        None => {
            out!("{HELP}");
            return Ok(0);
        }
        Some(Short('h') | Long("help")) => {
            out!("{HELP}");
            return Ok(0);
        }
        Some(Value(v)) => v.string()?,
        Some(other) => return Err(other.unexpected().into()),
    };
    match command.as_str() {
        "run" => run::run(run::parse_args(parser)?),
        "check" => {
            match parser.next()? {
                None => {}
                Some(lexopt::Arg::Short('h') | lexopt::Arg::Long("help")) => {
                    out!("{}", check::HELP);
                    return Ok(0);
                }
                Some(other) => return Err(other.unexpected().into()),
            }
            check::run()
        }
        "setup" => setup::run(setup::parse_args(parser)?),
        "apply" => {
            match parser.next()? {
                None => {}
                Some(lexopt::Arg::Short('h') | lexopt::Arg::Long("help")) => {
                    out!("{}", apply::HELP);
                    return Ok(0);
                }
                Some(other) => return Err(other.unexpected().into()),
            }
            apply::run()
        }
        "audio" => audio::run(audio::parse_args(parser)?),
        "unsafe-boost" => unsafe_boost::run(unsafe_boost::parse_args(parser)?),
        "boost" => boost_command(parser),
        "profiles" => profiles_command(parser),
        other => Err(Error::Usage(format!("unknown game command `{other}`"))),
    }
}

/// The root helper `seg game run` calls through sudo:
/// `boost on PID [GOVERNOR]`, `boost off PID`, `boost status`. Nothing the
/// caller says reaches a file name; the governor must be one the CPU lists.
fn boost_command(parser: &mut lexopt::Parser) -> Result<u8> {
    use lexopt::prelude::*;
    let mut words: Vec<String> = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Value(v) => words.push(v.string()?),
            Short('h') | Long("help") => {
                out!(
                    "Usage: seg game boost on PID [GOVERNOR] | off PID | status\n\nThe root helper behind `seg game run`; use that instead.\n"
                );
                return Ok(0);
            }
            other => return Err(other.unexpected().into()),
        }
    }
    let pid = |w: Option<&String>| -> Result<u32> {
        w.and_then(|s| s.parse::<u32>().ok())
            .filter(|p| *p > 1)
            .ok_or_else(|| Error::Usage("a process id is needed".into()))
    };
    let valid_governor = |g: &str| {
        !g.is_empty()
            && g.len() <= 32
            && g.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    };
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    sys::require_root("`seg game boost`")?;
    let boost = boost::Boost::new("/");
    let report = match words.as_slice() {
        ["on", pid_word] => boost.on(pid(Some(&pid_word.to_string()))?, "performance")?,
        ["on", pid_word, governor] if valid_governor(governor) => {
            boost.on(pid(Some(&pid_word.to_string()))?, governor)?
        }
        ["off", pid_word] => boost.off(pid(Some(&pid_word.to_string()))?)?,
        ["status"] => {
            out!("{}", boost.status());
            return Ok(0);
        }
        _ => {
            return Err(Error::Usage(
                "seg game boost on PID [GOVERNOR] | off PID | status".into(),
            ));
        }
    };
    for line in &report.lines {
        outln!("{line}");
    }
    for w in &report.warnings {
        sys::emit_err(format_args!("seg: {w}"));
    }
    Ok(0)
}

fn profiles_command(parser: &mut lexopt::Parser) -> Result<u8> {
    use lexopt::prelude::*;
    let mut file: Option<std::path::PathBuf> = None;
    while let Some(arg) = parser.next()? {
        match arg {
            Value(v) if file.is_none() => file = Some(v.into()),
            Short('h') | Long("help") => {
                out!(
                    "Usage: seg game profiles [FILE]\n\nChecks ~/.config/seg/games.toml (or FILE) and lists what it says.\n"
                );
                return Ok(0);
            }
            other => return Err(other.unexpected().into()),
        }
    }
    let path = file
        .or_else(profile::default_path)
        .ok_or_else(|| Error::other("cannot tell where the profiles are (no HOME)"))?;
    let profiles = profile::load(&path)?;
    if profiles.is_empty() {
        outln!(
            "{}: no profiles (it is read when a game starts)",
            path.display()
        );
        return Ok(0);
    }
    outln!(
        "{}: {} profile{}",
        path.display(),
        profiles.len(),
        if profiles.len() == 1 { "" } else { "s" }
    );
    for p in &profiles {
        let governor = match &p.governor {
            profile::Governor::Performance => "performance".to_string(),
            profile::Governor::Keep => "keep".to_string(),
            profile::Governor::Named(n) => n.clone(),
        };
        let pin = match &p.pin {
            profile::Pin::Off => "off".to_string(),
            profile::Pin::VCache => "vcache".to_string(),
            profile::Pin::Cpus(c) => hw::format_cpulist(c),
        };
        outln!(
            "  {:<24} governor {governor}, pin {pin}, mangohud {}, gamescope {}",
            p.matcher,
            if p.mangohud { "on" } else { "off" },
            if p.gamescope.is_empty() {
                "off".to_string()
            } else {
                p.gamescope.join(" ")
            }
        );
    }
    Ok(0)
}
