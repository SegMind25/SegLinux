//! Per-game profiles: ~/.config/seg/games.toml, one `[[game]]` per program,
//! matched by the name of the executable.
//!
//! ```toml
//! [[game]]
//! match = "eldenring.exe"        # glob; compared with the file names in the command
//! governor = "performance"      # or "keep" to leave the CPU alone, or another governor
//! pin = "vcache"                # "vcache", "off" or a CPU list such as "0-7,16-23"
//! mangohud = true
//! gamescope = ["-W", "2560", "-H", "1440", "-r", "165", "--adaptive-sync"]
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use super::hw::parse_cpulist;
use super::toml::{self, Table, Value};
use crate::error::{Error, Result};

/// What the CPU governor does while the game runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Governor {
    /// `performance`.
    #[default]
    Performance,
    /// Leave it as it is: no boost for this game.
    Keep,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Pin {
    #[default]
    Off,
    /// The complex with the biggest L3 cache, on an X3D part with two.
    VCache,
    Cpus(Vec<usize>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profile {
    pub matcher: String,
    pub governor: Governor,
    pub pin: Pin,
    pub mangohud: bool,
    /// A MangoHud config file; `preset` means the one SegLinux ships.
    pub mangohud_config: Option<String>,
    pub gamescope: Vec<String>,
    pub nice: Option<i32>,
    /// Line of the `[[game]]` header, for messages.
    pub line: usize,
}

pub const KNOWN_KEYS: [&str; 7] = [
    "match",
    "governor",
    "pin",
    "mangohud",
    "mangohud_config",
    "gamescope",
    "nice",
];

pub fn parse_pin(text: &str) -> std::result::Result<Pin, String> {
    match text.trim() {
        "off" | "none" | "" => Ok(Pin::Off),
        "vcache" | "v-cache" | "auto" => Ok(Pin::VCache),
        list => parse_cpulist(list).map(Pin::Cpus).ok_or_else(|| {
            format!("`{list}` is not `vcache`, `off` or a CPU list such as 0-7,16-23")
        }),
    }
}

fn valid_governor(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn one(table: &Table) -> std::result::Result<Profile, String> {
    for (key, _, line) in &table.entries {
        if !KNOWN_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "line {line}: unknown setting `{key}` (known: {})",
                KNOWN_KEYS.join(", ")
            ));
        }
    }
    let text = |key: &str| -> std::result::Result<Option<String>, String> {
        match table.get(key) {
            None => Ok(None),
            Some(Value::Str(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!(
                "`{key}` must be a string (table at line {})",
                table.line
            )),
        }
    };
    let matcher = text("match")?
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| format!("the [[game]] at line {} has no `match`", table.line))?;
    let governor = match text("governor")?.as_deref() {
        None | Some("performance") => Governor::Performance,
        Some("keep") => Governor::Keep,
        Some(name) if valid_governor(name) => Governor::Named(name.to_string()),
        Some(other) => {
            return Err(format!(
                "`{other}` is not a governor (table at line {})",
                table.line
            ));
        }
    };
    let pin = match text("pin")? {
        None => Pin::Off,
        Some(p) => parse_pin(&p).map_err(|e| format!("pin: {e} (table at line {})", table.line))?,
    };
    let mangohud = match table.get("mangohud") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(format!(
                "`mangohud` must be true or false (table at line {})",
                table.line
            ));
        }
    };
    let gamescope = match table.get("gamescope") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::Str(s) => Ok(s.clone()),
                Value::Int(n) => Ok(n.to_string()),
                _ => Err(format!(
                    "`gamescope` must list strings (table at line {})",
                    table.line
                )),
            })
            .collect::<std::result::Result<_, _>>()?,
        Some(_) => {
            return Err(format!(
                "`gamescope` must be a list of strings (table at line {})",
                table.line
            ));
        }
    };
    let nice = match table.get("nice") {
        None => None,
        Some(Value::Int(n)) if (-20..=19).contains(n) => Some(*n as i32),
        Some(_) => {
            return Err(format!(
                "`nice` must be a number from -20 to 19 (table at line {})",
                table.line
            ));
        }
    };
    Ok(Profile {
        matcher,
        governor,
        pin,
        mangohud,
        mangohud_config: text("mangohud_config")?,
        gamescope,
        nice,
        line: table.line,
    })
}

pub fn parse(text: &str) -> std::result::Result<Vec<Profile>, String> {
    let tables = toml::parse(text).map_err(|e| e.to_string())?;
    tables
        .iter()
        .map(|t| {
            if t.name == "game" {
                one(t)
            } else {
                Err(format!(
                    "line {}: unknown table [[{}]] (only [[game]])",
                    t.line, t.name
                ))
            }
        })
        .collect()
}

/// Where the profiles are: `$XDG_CONFIG_HOME/seg/games.toml`, else
/// `~/.config/seg/games.toml` of the user who ran `sudo`, if any.
pub fn default_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => crate::sys::invoking_home()?.join(".config"),
    };
    Some(base.join("seg/games.toml"))
}

/// The profiles of a file; a missing file has none.
pub fn load(path: &Path) -> Result<Vec<Profile>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(format!("reading {}", path.display()), e)),
    };
    parse(&text).map_err(|e| Error::other(format!("{}: {e}", path.display())))
}

/// `*` and `?` globbing, without regard to case (Windows programs are
/// written both ways).
pub fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ni));
                pi += 1;
            }
            Some(&c) if c == '?' || Some(&c) == n.get(ni) => {
                pi += 1;
                ni += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    pi = sp + 1;
                    ni = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    while p.get(pi) == Some(&'*') {
        pi += 1;
    }
    pi == p.len()
}

/// The last component of a path, or of a Windows path.
pub fn file_name(arg: &str) -> &str {
    arg.rsplit(['/', '\\']).next().unwrap_or(arg)
}

/// The first profile whose `match` fits the name of any program in the
/// command: `seg game run -- %command%` becomes a Proton command line
/// (`.../proton waitforexitandrun /games/Game/game.exe`), so every argument
/// counts, not just the first.
pub fn find<'a>(profiles: &'a [Profile], command: &[String]) -> Option<&'a Profile> {
    profiles.iter().find(|p| {
        command
            .iter()
            .any(|arg| glob(&p.matcher, file_name(arg)) || glob(&p.matcher, arg))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_full_profile() {
        let p = parse(
            r#"
[[game]]
match = "eldenring.exe"
governor = "performance"
pin = "vcache"
mangohud = true
mangohud_config = "preset"
gamescope = ["-W", "2560", "-H", 1440, "--adaptive-sync"]
nice = -8
"#,
        )
        .unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].matcher, "eldenring.exe");
        assert_eq!(p[0].governor, Governor::Performance);
        assert_eq!(p[0].pin, Pin::VCache);
        assert!(p[0].mangohud);
        assert_eq!(p[0].mangohud_config.as_deref(), Some("preset"));
        assert_eq!(
            p[0].gamescope,
            ["-W", "2560", "-H", "1440", "--adaptive-sync"]
        );
        assert_eq!(p[0].nice, Some(-8));
        assert_eq!(p[0].line, 2);
    }

    #[test]
    fn defaults_are_a_plain_boost() {
        let p = parse("[[game]]\nmatch = \"x\"\n").unwrap();
        assert_eq!(p[0].governor, Governor::Performance);
        assert_eq!(p[0].pin, Pin::Off);
        assert!(!p[0].mangohud);
        assert!(p[0].gamescope.is_empty());
        assert_eq!(p[0].nice, None);
    }

    #[test]
    fn governors_and_pins() {
        let p = parse("[[game]]\nmatch=\"a\"\ngovernor=\"keep\"\npin=\"0-7,16-23\"\n[[game]]\nmatch=\"b\"\ngovernor=\"schedutil\"\npin=\"off\"\n").unwrap();
        assert_eq!(p[0].governor, Governor::Keep);
        assert_eq!(p[0].pin, Pin::Cpus((0..8).chain(16..24).collect()));
        assert_eq!(p[1].governor, Governor::Named("schedutil".into()));
        assert_eq!(p[1].pin, Pin::Off);
        assert_eq!(parse_pin("auto"), Ok(Pin::VCache));
        assert!(parse_pin("3-1").is_err());
    }

    #[test]
    fn mistakes_are_reported_not_ignored() {
        let cases = [
            ("[[game]]\ngovernor = \"keep\"\n", "no `match`"),
            ("[[game]]\nmatch = \"\"\n", "no `match`"),
            (
                "[[game]]\nmatch = \"x\"\ngovenor = \"keep\"\n",
                "unknown setting `govenor`",
            ),
            (
                "[[game]]\nmatch = \"x\"\nmangohud = \"yes\"\n",
                "must be true or false",
            ),
            ("[[game]]\nmatch = \"x\"\nnice = 40\n", "from -20 to 19"),
            (
                "[[game]]\nmatch = \"x\"\ngovernor = \"Per formance\"\n",
                "is not a governor",
            ),
            ("[[game]]\nmatch = \"x\"\npin = \"left\"\n", "pin:"),
            (
                "[[game]]\nmatch = \"x\"\ngamescope = \"-W 100\"\n",
                "list of strings",
            ),
            ("[[other]]\nmatch = \"x\"\n", "unknown table [[other]]"),
            ("[[game]]\nmatch = x\n", "line 2"),
        ];
        for (doc, fragment) in cases {
            let e = parse(doc).unwrap_err();
            assert!(e.contains(fragment), "{doc:?} -> {e}");
        }
    }

    #[test]
    fn globs() {
        for (pat, name, want) in [
            ("eldenring.exe", "EldenRing.exe", true),
            ("*.exe", "game.EXE", true),
            ("cs2*", "cs2", true),
            ("cs2*", "cs2.sh", true),
            ("g?me", "game", true),
            ("g?me", "gme", false),
            ("*launcher*", "My Launcher v2.exe", true),
            ("a*b*c", "aXXbYYc", true),
            ("a*b*c", "aXXbYY", false),
            ("*", "", true),
            ("", "", true),
            ("x", "", false),
        ] {
            assert_eq!(glob(pat, name), want, "{pat} vs {name}");
        }
    }

    #[test]
    fn a_proton_command_line_matches_its_exe() {
        let profiles =
            parse("[[game]]\nmatch = \"eldenring.exe\"\n[[game]]\nmatch = \"cs2\"\nnice = 5\n")
                .unwrap();
        let proton = cmd(&[
            "/home/u/.steam/root/compatibilitytools.d/GE-Proton/proton",
            "waitforexitandrun",
            "/games/ELDEN RING/Game/eldenring.exe",
        ]);
        assert_eq!(find(&profiles, &proton).map(|p| p.line), Some(1));
        // A Windows-style path (an installer script passes one) too.
        assert!(
            find(
                &profiles,
                &cmd(&["wine", "C:\\Games\\EldenRing\\eldenring.exe"])
            )
            .is_some()
        );
        let native = cmd(&[
            "/home/u/.steam/steamapps/common/CS2/game/bin/linuxsteamrt64/cs2",
            "-novid",
        ]);
        assert_eq!(find(&profiles, &native).and_then(|p| p.nice), Some(5));
        assert!(find(&profiles, &cmd(&["glxgears"])).is_none());
        assert!(find(&[], &native).is_none());
    }

    #[test]
    fn the_first_matching_profile_wins() {
        let p =
            parse("[[game]]\nmatch = \"*.exe\"\nnice = 1\n[[game]]\nmatch = \"a.exe\"\nnice = 2\n")
                .unwrap();
        assert_eq!(find(&p, &cmd(&["a.exe"])).and_then(|x| x.nice), Some(1));
    }

    #[test]
    fn a_missing_file_has_no_profiles_and_a_broken_one_names_itself() {
        let t = crate::sys::testutil::TempDir::new();
        assert_eq!(load(&t.path().join("none.toml")).unwrap(), vec![]);
        let broken = t.put("games.toml", "[[game]]\nmatch = oops\n");
        let e = load(&broken).unwrap_err().to_string();
        assert!(e.contains("games.toml: line 2"), "{e}");
    }
}
