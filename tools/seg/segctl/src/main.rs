//! seg: system information, services, cleaning, tuning, zram swap and
//! health checks for SegLinux, over runit, OpenRC, dinit and s6.

// No unwrap or panic on paths a user can reach: errors are values.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

/// `print!` and `println!` that end quietly when the reader goes away
/// (`seg clean | head`) instead of panicking on the broken pipe.
macro_rules! out {
    ($($arg:tt)*) => { $crate::sys::emit(format_args!($($arg)*), false) };
}
macro_rules! outln {
    ($($arg:tt)*) => { $crate::sys::emit(format_args!($($arg)*), true) };
}

mod clean;
mod doctor;
mod error;
mod game;
mod info;
mod init;
mod svc;
mod sys;
mod tune;
mod vercmp;
mod zram;

use std::process::ExitCode;

use error::{Error, Result};

const HELP: &str = "\
seg - look after a SegLinux system

Usage: seg <command> [options]

Commands:
  info                         distro, kernel, init, uptime, memory, packages, disk
  svc list|enable|disable|start|stop [name]
                               services, on whichever init is running
  clean [--apply]              orphans, old cached packages, old logs (dry run by default)
  tune light|balanced|gaming|performance|apply|status
                               memory, CPU and disk settings
  zram on|off|status           compressed swap in memory
  game run|check|setup|...     play: boost while a game runs, readiness check, drivers
  doctor                       look for problems and print their fixes

Options:
  -h, --help                   show this help (also: seg <command> --help)
  -V, --version                show the version
";

const SVC_HELP: &str = "\
Usage: seg svc <command> [name]

  list [--enabled]   installed services: enabled at boot, and running or not
  enable <name>      start at boot
  disable <name>     do not start at boot
  start <name>       start now
  stop <name>        stop now

Works on OpenRC, runit, dinit and s6 (detected). enable, disable, start and
stop need root; list does not.

--offline  pick the init from what is installed here, not from the running
           PID 1: for package scripts that run in a chroot
";

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            sys::emit_err(format_args!("seg: {e}"));
            ExitCode::from(e.exit_code())
        }
    }
}

/// The exit status: 0, 1 when `doctor` found problems, or the game's own.
fn run() -> Result<u8> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_env();
    let command = match parser.next()? {
        None => {
            out!("{HELP}");
            return Ok(0);
        }
        Some(Short('h') | Long("help")) => {
            out!("{HELP}");
            return Ok(0);
        }
        Some(Short('V') | Long("version")) => {
            outln!("seg {}", env!("CARGO_PKG_VERSION"));
            return Ok(0);
        }
        Some(Value(v)) => v.string()?,
        Some(other) => return Err(other.unexpected().into()),
    };
    match command.as_str() {
        "info" => {
            no_more_args(&mut parser, "info")?;
            info::run()?;
        }
        "svc" => svc_command(&mut parser)?,
        "clean" => clean::run(clean::parse_args(&mut parser)?)?,
        "tune" => tune::run(tune::parse_args(&mut parser)?)?,
        "zram" => zram::run(zram::parse_args(&mut parser)?)?,
        "game" => return game::run(&mut parser),
        "doctor" => {
            no_more_args(&mut parser, "doctor")?;
            return Ok(u8::from(!doctor::run()?));
        }
        other => {
            return Err(Error::Usage(format!("unknown command `{other}`")));
        }
    }
    Ok(0)
}

/// Commands without options of their own: `--help` or nothing.
fn no_more_args(parser: &mut lexopt::Parser, name: &str) -> Result<()> {
    use lexopt::prelude::*;
    match parser.next()? {
        None => Ok(()),
        Some(Short('h') | Long("help")) => {
            match name {
                "doctor" => out!("{}", doctor::HELP),
                _ => outln!("Usage: seg {name}\n\nPrints a short summary of this system."),
            }
            std::process::exit(0);
        }
        Some(other) => Err(other.unexpected().into()),
    }
}

fn svc_command(parser: &mut lexopt::Parser) -> Result<()> {
    use lexopt::prelude::*;
    let mut action: Option<String> = None;
    let mut name: Option<String> = None;
    let mut only_enabled = false;
    let mut offline = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => {
                out!("{SVC_HELP}");
                return Ok(());
            }
            Long("enabled") => only_enabled = true,
            Long("offline") => offline = true,
            Value(v) if action.is_none() => action = Some(v.string()?),
            Value(v) if name.is_none() => name = Some(v.string()?),
            other => return Err(other.unexpected().into()),
        }
    }
    let action = action.ok_or_else(|| {
        Error::Usage("seg svc needs a command (list, enable, disable, start or stop)".into())
    })?;
    let detected = if offline {
        init::detect(std::path::Path::new("/"), None)
    } else {
        init::detect_running()
    };
    let init = detected.ok_or_else(|| {
        Error::Unsupported("no supported init system found (OpenRC, runit, dinit or s6)".into())
    })?;
    let svc = svc::Svc::new(init, "/");
    let need_name = || {
        name.clone()
            .ok_or_else(|| Error::Usage(format!("seg svc {action} needs a service name")))
    };
    match action.as_str() {
        "list" => {
            outln!("{:<28}{:<9}STATE   ({init})", "SERVICE", "BOOT");
            for s in svc
                .list()?
                .into_iter()
                .filter(|s| !only_enabled || s.enabled)
            {
                outln!(
                    "{:<28}{:<9}{}",
                    s.name,
                    if s.enabled { "enabled" } else { "-" },
                    s.state.label()
                );
            }
        }
        "enable" => {
            sys::require_root("`seg svc enable`")?;
            let name = need_name()?;
            match svc.enable(&name)? {
                svc::Change::Enabled => {
                    outln!("{name} will start at boot. Start it now with: seg svc start {name}");
                }
                _ => outln!("{name} already starts at boot"),
            }
        }
        "disable" => {
            sys::require_root("`seg svc disable`")?;
            let name = need_name()?;
            match svc.disable(&name)? {
                svc::Change::Disabled => {
                    outln!("{name} will not start at boot. Stop it now with: seg svc stop {name}");
                }
                _ => outln!("{name} already does not start at boot"),
            }
        }
        "start" | "stop" => {
            sys::require_root(if action == "start" {
                "`seg svc start`"
            } else {
                "`seg svc stop`"
            })?;
            let name = need_name()?;
            let act = if action == "start" {
                svc::Action::Start
            } else {
                svc::Action::Stop
            };
            svc.control(act, &name)?;
        }
        other => {
            return Err(Error::Usage(format!(
                "unknown svc command `{other}` (list, enable, disable, start or stop)"
            )));
        }
    }
    Ok(())
}
