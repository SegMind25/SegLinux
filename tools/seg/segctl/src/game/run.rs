//! `seg game run -- COMMAND`: a lightweight GameMode for the length of one
//! game. Steam launch option: `seg game run -- %command%`.
//!
//! While the game runs: CPU governor and energy preference `performance`,
//! an AMD GPU at its high power level (through the root helper, see
//! boost.rs), a better CPU and I/O priority for the game, the screen saver
//! held off, and, if asked, the game on the V-Cache cores of an X3D CPU.
//! When it ends, or seg is interrupted, or seg itself fails, every setting
//! goes back. Nothing is left changed.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use super::boost::Boost;
use super::dbus;
use super::hw;
use super::proc;
use super::profile::{self, Governor, Pin, Profile};
use crate::error::{Error, Result};
use crate::sys;

pub const HELP: &str = "\
Usage: seg game run [options] -- COMMAND [ARGS...]

Runs COMMAND with the computer set up for a game, and puts everything back
when it ends (also on Ctrl+C, SIGTERM and a crash of seg). In Steam, set the
game's launch options to:  seg game run -- %command%

While the game runs:
  - CPU governor and energy preference: performance; an AMD GPU: high power
    level (needs the gamemode group, or root)
  - the game's CPU priority is raised a little and its I/O priority is the
    best of its class
  - the screen saver and idle blanking are held off (session bus)
  - with --pin vcache, or pin = \"vcache\" in the game's profile: only the
    cores of the V-Cache complex of an X3D CPU with two

Options:
  --no-boost        do not change the governor or the GPU
  --pin WHERE       vcache, off, or a CPU list such as 0-7,16-23
  --nice N          CPU priority of the game, -20..19 (default -4)
  --profiles FILE   game profiles (default ~/.config/seg/games.toml)
  --dry-run         show what would be done and stop
";

const DEFAULT_NICE: i32 = -4;
const SEG: &str = "/usr/bin/seg";

#[derive(Debug, Default)]
pub struct Options {
    pub no_boost: bool,
    pub pin: Option<Pin>,
    pub nice: Option<i32>,
    pub profiles: Option<PathBuf>,
    pub dry_run: bool,
    pub command: Vec<String>,
}

pub fn parse_args(parser: &mut lexopt::Parser) -> Result<Options> {
    use lexopt::prelude::*;
    let mut o = Options::default();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("no-boost") => o.no_boost = true,
            Long("dry-run") => o.dry_run = true,
            Long("pin") => {
                o.pin = Some(profile::parse_pin(&parser.value()?.string()?).map_err(Error::Usage)?);
            }
            Long("nice") => {
                let n: i32 = parser.value()?.parse()?;
                if !(-20..=19).contains(&n) {
                    return Err(Error::Usage("--nice must be from -20 to 19".into()));
                }
                o.nice = Some(n);
            }
            Long("profiles") => o.profiles = Some(parser.value()?.into()),
            Short('h') | Long("help") => {
                out!("{HELP}");
                std::process::exit(0);
            }
            // Everything after `--` is the command, whatever it looks like.
            Value(first) => {
                o.command.push(first.string()?);
                o.command
                    .extend(parser.raw_args()?.map(|a| a.to_string_lossy().into_owned()));
                break;
            }
            other => return Err(other.unexpected().into()),
        }
    }
    if o.command.is_empty() {
        return Err(Error::Usage(
            "seg game run needs a command: seg game run -- COMMAND".into(),
        ));
    }
    Ok(o)
}

/// What the machine offers, for planning.
#[derive(Debug, Default, Clone)]
pub struct Machine {
    /// CPUs of the V-Cache complex, when there is one to choose.
    pub vcache: Option<Vec<usize>>,
    pub mangohud: bool,
    pub gamescope: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// The governor for the boost; None: no boost.
    pub governor: Option<String>,
    pub cpus: Option<Vec<usize>>,
    pub nice: i32,
    /// The command to start, with gamescope and MangoHud wrapped around it.
    pub argv: Vec<String>,
    /// Environment the game gets unless it is already set.
    pub env: Vec<(String, String)>,
    pub notes: Vec<String>,
}

pub const PRESET: &str = "/usr/share/seg/mangohud-gaming.conf";

/// Decide what to do. `is_set(name)` tells whether the user already set an
/// environment variable (theirs wins).
pub fn plan(
    o: &Options,
    profile: Option<&Profile>,
    machine: &Machine,
    is_set: &dyn Fn(&str) -> bool,
) -> Plan {
    let mut notes = Vec::new();
    let governor = if o.no_boost {
        None
    } else {
        match profile.map(|p| &p.governor) {
            None | Some(Governor::Performance) => Some("performance".to_string()),
            Some(Governor::Keep) => None,
            Some(Governor::Named(n)) => Some(n.clone()),
        }
    };
    let wanted_pin = o
        .pin
        .clone()
        .or_else(|| profile.map(|p| p.pin.clone()))
        .unwrap_or_default();
    let cpus = match wanted_pin {
        Pin::Off => None,
        Pin::Cpus(list) => Some(list),
        Pin::VCache => match &machine.vcache {
            Some(cpus) => Some(cpus.clone()),
            None => {
                notes.push(
                    "no V-Cache complex to pin to (not an X3D CPU with two); not pinning".into(),
                );
                None
            }
        },
    };
    let mut argv = o.command.clone();
    let mut env: Vec<(String, String)> = Vec::new();
    if let Some(p) = profile {
        if p.mangohud {
            if machine.mangohud {
                argv.insert(0, "mangohud".into());
                let config = match p.mangohud_config.as_deref() {
                    Some("preset") => Some(PRESET.to_string()),
                    Some(path) => Some(path.to_string()),
                    None => None,
                };
                if let Some(config) = config
                    && !is_set("MANGOHUD_CONFIGFILE")
                {
                    env.push(("MANGOHUD_CONFIGFILE".into(), config));
                }
            } else {
                notes.push("MangoHud is not installed; running without it".into());
            }
        }
        if !p.gamescope.is_empty() {
            if machine.gamescope {
                let mut wrapped = vec!["gamescope".to_string()];
                wrapped.extend(p.gamescope.iter().cloned());
                wrapped.push("--".into());
                wrapped.extend(argv);
                argv = wrapped;
            } else {
                notes.push("gamescope is not installed; running without it".into());
            }
        }
    }
    // Shader caches: room for the compiled shaders of many games (the 1 GB
    // defaults make games recompile them and stutter).
    for (name, value) in [
        ("MESA_SHADER_CACHE_MAX_SIZE", "10G"),
        ("__GL_SHADER_DISK_CACHE_SIZE", "10737418240"),
        ("__GL_SHADER_DISK_CACHE_SKIP_CLEANUP", "1"),
    ] {
        if !is_set(name) {
            env.push((name.into(), value.into()));
        }
    }
    Plan {
        governor,
        cpus,
        nice: o
            .nice
            .or(profile.and_then(|p| p.nice))
            .unwrap_or(DEFAULT_NICE),
        argv,
        env,
        notes,
    }
}

/// The root helper, as seen from the process that asks for the boost.
static BOOST_HELD: AtomicBool = AtomicBool::new(false);

fn as_root() -> bool {
    sys::euid().is_ok_and(|u| u == 0)
}

/// Switch the boost on for this process. Failure is a warning: the game runs.
fn boost_on(governor: &str) {
    let pid = std::process::id();
    let failure: Option<String> = if as_root() {
        match Boost::new("/").on(pid, governor) {
            Ok(report) => report.warnings.into_iter().next(),
            Err(e) => Some(e.to_string()),
        }
    } else {
        match sys::run(
            "sudo",
            &["-n", SEG, "game", "boost", "on", &pid.to_string(), governor],
        ) {
            Ok(out) if out.ok => None,
            Ok(out) => Some(out.stderr.trim().to_string()),
            Err(e) => Some(e.to_string()),
        }
    };
    match failure {
        None => BOOST_HELD.store(true, Ordering::SeqCst),
        Some(why) => {
            // A partial boost is still a boost to give back.
            BOOST_HELD.store(true, Ordering::SeqCst);
            sys::emit_err(format_args!(
                "seg: the performance boost is not (fully) on: {}\nseg: run `seg game check` for the fix",
                if why.is_empty() {
                    "the helper failed"
                } else {
                    &why
                }
            ));
        }
    }
}

/// Put everything back. Safe to call twice, and from the panic hook.
pub fn boost_off() {
    if !BOOST_HELD.swap(false, Ordering::SeqCst) {
        return;
    }
    let pid = std::process::id();
    if as_root() {
        let _ = Boost::new("/").off(pid);
    } else {
        let _ = sys::run(
            "sudo",
            &["-n", SEG, "game", "boost", "off", &pid.to_string()],
        );
    }
}

/// Puts the system back when it goes out of scope.
struct Session {
    _idle: Option<dbus::Inhibitor>,
}

impl Drop for Session {
    fn drop(&mut self) {
        boost_off();
    }
}

/// Can `program` be started? A path must be a file you may run; a bare name
/// must be found in PATH.
fn program_exists(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if program.contains('/') {
        std::fs::metadata(program).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    } else {
        sys::which(program).is_some()
    }
}

fn describe(plan: &Plan, profile: Option<&Profile>) -> String {
    let mut text = String::new();
    match profile {
        Some(p) => text.push_str(&format!(
            "profile:   match \"{}\" (line {})\n",
            p.matcher, p.line
        )),
        None => text.push_str("profile:   none matched\n"),
    }
    text.push_str(&format!(
        "boost:     {}\n",
        plan.governor
            .as_deref()
            .map_or("off".to_string(), |g| format!(
                "governor {g}, energy preference and GPU level"
            ))
    ));
    text.push_str(&format!(
        "pin:       {}\n",
        plan.cpus
            .as_deref()
            .map_or("no".to_string(), hw::format_cpulist)
    ));
    text.push_str(&format!("nice:      {}\n", plan.nice));
    for (k, v) in &plan.env {
        text.push_str(&format!("env:       {k}={v}\n"));
    }
    text.push_str(&format!("command:   {}\n", plan.argv.join(" ")));
    for n in &plan.notes {
        text.push_str(&format!("note:      {n}\n"));
    }
    text
}

pub fn run(o: Options) -> Result<u8> {
    let profiles = match o.profiles.clone().or_else(profile::default_path) {
        Some(path) => profile::load(&path).unwrap_or_else(|e| {
            // A typo in games.toml must not keep anyone from playing.
            sys::emit_err(format_args!("seg: {e}\nseg: playing without profiles"));
            Vec::new()
        }),
        None => Vec::new(),
    };
    let found = profile::find(&profiles, &o.command);
    let machine = Machine {
        vcache: hw::vcache_group(&hw::l3_groups(std::path::Path::new("/"))).map(|g| g.cpus.clone()),
        mangohud: sys::which("mangohud").is_some(),
        gamescope: sys::which("gamescope").is_some(),
    };
    let plan = plan(&o, found, &machine, &|name| {
        std::env::var_os(name).is_some()
    });
    if o.dry_run {
        out!("{}", describe(&plan, found));
        return Ok(0);
    }
    for n in &plan.notes {
        sys::emit_err(format_args!("seg: {n}"));
    }
    // A program that is not there fails before anything is changed, with the
    // status a shell gives.
    if let Some(program) = plan.argv.first()
        && !program_exists(program)
    {
        sys::emit_err(format_args!("seg: `{program}`: command not found"));
        return Ok(127);
    }

    // From here on, everything changed is changed back: when the game ends,
    // on a signal (the wait returns) and on a crash of seg (the hook).
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        boost_off();
        previous(info);
    }));
    if let Err(e) = proc::install_signal_handlers() {
        sys::emit_err(format_args!(
            "seg: cannot catch signals ({e}); Ctrl+C may leave the boost on"
        ));
    }
    let session = Session {
        _idle: dbus::inhibit_screensaver("seg", "playing a game").ok(),
    };
    if let Some(governor) = &plan.governor {
        boost_on(governor);
    }

    let Some((program, args)) = plan.argv.split_first() else {
        return Err(Error::Usage("seg game run needs a command".into()));
    };
    let mut command = Command::new(program);
    command.args(args);
    for (k, v) in &plan.env {
        command.env(k, v);
    }
    let child = proc::spawn_prepared(&mut command, Some(plan.nice), true, plan.cpus.clone());
    let code = match child {
        Ok(child) => {
            proc::set_child(child.id());
            proc::wait_for(child.id()).unwrap_or(1)
        }
        Err(e) => {
            drop(session);
            return Err(Error::Command {
                program: program.clone(),
                detail: format!("cannot start it: {e}"),
            });
        }
    };
    drop(session);
    Ok((code & 0xff) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::profile::parse;

    fn opts(cmd: &[&str]) -> Options {
        Options {
            command: cmd.iter().map(|s| s.to_string()).collect(),
            ..Options::default()
        }
    }

    fn machine() -> Machine {
        Machine {
            vcache: Some(vec![0, 1, 2, 3]),
            mangohud: true,
            gamescope: true,
        }
    }

    fn none_set(_: &str) -> bool {
        false
    }

    #[test]
    fn a_plain_game_gets_the_boost_and_shader_caches() {
        let p = plan(
            &opts(&["./game", "-fullscreen"]),
            None,
            &machine(),
            &none_set,
        );
        assert_eq!(p.governor.as_deref(), Some("performance"));
        assert_eq!(p.cpus, None);
        assert_eq!(p.nice, -4);
        assert_eq!(p.argv, ["./game", "-fullscreen"]);
        let env: Vec<_> = p.env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            env,
            [
                "MESA_SHADER_CACHE_MAX_SIZE",
                "__GL_SHADER_DISK_CACHE_SIZE",
                "__GL_SHADER_DISK_CACHE_SKIP_CLEANUP"
            ]
        );
        assert!(p.notes.is_empty());
    }

    #[test]
    fn what_the_user_set_is_not_overridden() {
        let set = |n: &str| n == "MESA_SHADER_CACHE_MAX_SIZE";
        let p = plan(&opts(&["g"]), None, &machine(), &set);
        assert!(!p.env.iter().any(|(k, _)| k == "MESA_SHADER_CACHE_MAX_SIZE"));
        assert!(
            p.env
                .iter()
                .any(|(k, _)| k == "__GL_SHADER_DISK_CACHE_SIZE")
        );
    }

    #[test]
    fn no_boost_and_keep_leave_the_governor_alone() {
        let mut o = opts(&["g"]);
        o.no_boost = true;
        assert_eq!(plan(&o, None, &machine(), &none_set).governor, None);
        let profiles = parse("[[game]]\nmatch = \"g\"\ngovernor = \"keep\"\n").unwrap();
        assert_eq!(
            plan(&opts(&["g"]), profiles.first(), &machine(), &none_set).governor,
            None
        );
        let profiles = parse("[[game]]\nmatch = \"g\"\ngovernor = \"schedutil\"\n").unwrap();
        assert_eq!(
            plan(&opts(&["g"]), profiles.first(), &machine(), &none_set)
                .governor
                .as_deref(),
            Some("schedutil")
        );
    }

    #[test]
    fn pinning_to_the_vcache_complex() {
        let profiles = parse("[[game]]\nmatch = \"g\"\npin = \"vcache\"\n").unwrap();
        let p = plan(&opts(&["g"]), profiles.first(), &machine(), &none_set);
        assert_eq!(p.cpus, Some(vec![0, 1, 2, 3]));
        // No such complex: a note, no pin.
        let none = Machine {
            vcache: None,
            ..machine()
        };
        let p = plan(&opts(&["g"]), profiles.first(), &none, &none_set);
        assert_eq!(p.cpus, None);
        assert!(p.notes[0].contains("no V-Cache"), "{:?}", p.notes);
        // The command line beats the profile.
        let mut o = opts(&["g"]);
        o.pin = Some(Pin::Off);
        assert_eq!(plan(&o, profiles.first(), &machine(), &none_set).cpus, None);
        o.pin = Some(Pin::Cpus(vec![8, 9]));
        assert_eq!(plan(&o, None, &machine(), &none_set).cpus, Some(vec![8, 9]));
    }

    #[test]
    fn mangohud_and_gamescope_wrap_the_game() {
        let profiles = parse("[[game]]\nmatch = \"g\"\nmangohud = true\nmangohud_config = \"preset\"\ngamescope = [\"-W\", \"2560\", \"--adaptive-sync\"]\n").unwrap();
        let p = plan(&opts(&["g", "-x"]), profiles.first(), &machine(), &none_set);
        assert_eq!(
            p.argv,
            [
                "gamescope",
                "-W",
                "2560",
                "--adaptive-sync",
                "--",
                "mangohud",
                "g",
                "-x"
            ]
        );
        assert!(
            p.env
                .contains(&("MANGOHUD_CONFIGFILE".into(), PRESET.into()))
        );
        // Missing tools are said, not fatal.
        let bare = Machine {
            mangohud: false,
            gamescope: false,
            ..machine()
        };
        let p = plan(&opts(&["g"]), profiles.first(), &bare, &none_set);
        assert_eq!(p.argv, ["g"]);
        assert_eq!(p.notes.len(), 2);
        // The user's own MangoHud config wins.
        let set = |n: &str| n == "MANGOHUD_CONFIGFILE";
        let p = plan(&opts(&["g"]), profiles.first(), &machine(), &set);
        assert!(!p.env.iter().any(|(k, _)| k == "MANGOHUD_CONFIGFILE"));
    }

    #[test]
    fn nice_comes_from_the_flag_then_the_profile_then_the_default() {
        let profiles = parse("[[game]]\nmatch = \"g\"\nnice = -9\n").unwrap();
        assert_eq!(
            plan(&opts(&["g"]), profiles.first(), &machine(), &none_set).nice,
            -9
        );
        let mut o = opts(&["g"]);
        o.nice = Some(2);
        assert_eq!(plan(&o, profiles.first(), &machine(), &none_set).nice, 2);
        assert_eq!(plan(&opts(&["g"]), None, &machine(), &none_set).nice, -4);
    }

    #[test]
    fn the_command_after_dashes_is_taken_whole() {
        let mut p =
            lexopt::Parser::from_args(["--no-boost", "--", "proton", "run", "--weird", "-x"]);
        let o = parse_args(&mut p).unwrap();
        assert!(o.no_boost);
        assert_eq!(o.command, ["proton", "run", "--weird", "-x"]);
        let mut p = lexopt::Parser::from_args(["--pin", "0-3", "game"]);
        let o = parse_args(&mut p).unwrap();
        assert_eq!(o.pin, Some(Pin::Cpus(vec![0, 1, 2, 3])));
        assert_eq!(o.command, ["game"]);
    }

    #[test]
    fn usage_mistakes() {
        for args in [
            vec![],
            vec!["--no-boost"],
            vec!["--nice", "99", "--", "g"],
            vec!["--pin", "left", "--", "g"],
            vec!["--bogus", "--", "g"],
        ] {
            let mut p = lexopt::Parser::from_args(args.clone());
            assert!(
                matches!(parse_args(&mut p), Err(Error::Usage(_))),
                "{args:?}"
            );
        }
    }

    #[test]
    fn programs_are_found_before_anything_is_changed() {
        assert!(program_exists("sh"));
        assert!(program_exists("/bin/sh"));
        assert!(!program_exists("definitely-not-a-game-seg"));
        assert!(!program_exists("/no/such/game"));
        // A file that cannot be run.
        assert!(!program_exists("/etc/hostname"));
    }

    #[test]
    fn the_plan_reads_well() {
        let profiles = parse("[[game]]\nmatch = \"g\"\npin = \"vcache\"\n").unwrap();
        let p = plan(&opts(&["g"]), profiles.first(), &machine(), &none_set);
        let text = describe(&p, profiles.first());
        assert!(text.contains("profile:   match \"g\" (line 1)"), "{text}");
        assert!(text.contains("pin:       0-3"), "{text}");
        assert!(text.contains("command:   g"), "{text}");
    }
}
