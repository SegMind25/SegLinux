//! The compositor's settings: `~/.config/seglinux/compositor.toml`, read at
//! start and again on Super+Shift+R (or SIGHUP is not needed: the key is
//! enough). Every key is optional; a missing file means the defaults, which
//! match the SegLinux session on labwc (data/desktop/labwc/rc.xml).
//!
//! ```toml
//! autostart = ["sh /usr/share/seglinux/labwc/autostart", "nm-applet"]
//! background = "#0b1220"
//! focus_follows_mouse = false
//!
//! [keyboard]
//! layout = "us,ara"
//! options = "grp:alt_shift_toggle"
//! repeat_delay = 300
//! repeat_rate = 30
//!
//! [binds]
//! "Super+Return" = "spawn seglinux-terminal"
//! "Super+Q" = "close"
//! "Alt+F4" = "close"
//! ```

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a key combination does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Run a command line (through `sh -c`).
    Spawn(String),
    Close,
    Maximize,
    Fullscreen,
    Minimize,
    /// Focus the next window (Alt+Tab).
    CycleWindows,
    /// Half of the screen, as labwc's SnapToEdge.
    SnapLeft,
    SnapRight,
    ShowDesktop,
    ReloadConfig,
    Quit,
    /// Switch to virtual terminal N (Ctrl+Alt+F1..F12).
    SwitchVt(i32),
}

impl Action {
    pub fn parse(text: &str) -> Result<Action, String> {
        let text = text.trim();
        let (word, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        let rest = rest.trim();
        let simple = |a: Action| {
            if rest.is_empty() {
                Ok(a)
            } else {
                Err(format!("`{word}` takes no argument"))
            }
        };
        match word {
            "spawn" if !rest.is_empty() => Ok(Action::Spawn(rest.to_string())),
            "spawn" => Err("`spawn` needs a command".into()),
            "close" => simple(Action::Close),
            "maximize" => simple(Action::Maximize),
            "fullscreen" => simple(Action::Fullscreen),
            "minimize" => simple(Action::Minimize),
            "cycle-windows" => simple(Action::CycleWindows),
            "snap-left" => simple(Action::SnapLeft),
            "snap-right" => simple(Action::SnapRight),
            "show-desktop" => simple(Action::ShowDesktop),
            "reload-config" => simple(Action::ReloadConfig),
            "quit" => simple(Action::Quit),
            "switch-vt" => rest
                .parse()
                .ok()
                .filter(|n| (1..=12).contains(n))
                .map(Action::SwitchVt)
                .ok_or_else(|| "`switch-vt` needs a number from 1 to 12".into()),
            other => Err(format!("unknown action `{other}`")),
        }
    }
}

/// Modifier keys of a key combination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

/// `Super+Shift+S`: modifiers and a key name (an xkb keysym name, case
/// does not matter for letters).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Combo {
    pub mods: Mods,
    /// Lower case for letters: `s`, `Return`, `F4`, `XF86AudioMute`.
    pub key: String,
}

impl Combo {
    pub fn parse(text: &str) -> Result<Combo, String> {
        let mut mods = Mods::default();
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let Some((key, modifiers)) = parts.split_last() else {
            return Err(format!("empty key combination `{text}`"));
        };
        if key.is_empty() {
            return Err(format!("no key in `{text}`"));
        }
        for m in modifiers {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" => mods.alt = true,
                "shift" => mods.shift = true,
                "super" | "logo" | "win" | "meta" => mods.logo = true,
                other => return Err(format!("unknown modifier `{other}` in `{text}`")),
            }
        }
        Ok(Combo {
            mods,
            key: normalize_key(key),
        })
    }
}

/// Single letters compare without case (`S` and `s` are the same key).
pub fn normalize_key(key: &str) -> String {
    if key.chars().count() == 1 {
        key.to_lowercase()
    } else {
        key.to_string()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    pub layout: String,
    pub variant: String,
    pub options: Option<String>,
    pub repeat_delay: Option<i32>,
    pub repeat_rate: Option<i32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct File {
    autostart: Option<Vec<String>>,
    background: Option<String>,
    focus_follows_mouse: Option<bool>,
    keyboard: Keyboard,
    binds: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub autostart: Vec<String>,
    /// Behind every window, where no wallpaper (layer surface) covers it.
    pub background: [f32; 4],
    pub focus_follows_mouse: bool,
    pub keyboard: Keyboard,
    pub binds: BTreeMap<Combo, Action>,
}

/// The shell's D-Bus actions, as labwc's rc.xml calls them.
fn shell(action: &str) -> String {
    format!("spawn gapplication action org.seglinux.Shell {action}")
}

/// The keys of the SegLinux session, the same as on labwc.
pub fn default_binds() -> Vec<(&'static str, String)> {
    vec![
        ("Super+space", shell("menu")),
        ("Super+e", "spawn dolphin".into()),
        ("Super+Return", "spawn seglinux-terminal".into()),
        ("Super+i", "spawn seglinux-shell --settings".into()),
        ("Super+l", "spawn swaylock -c 090f1d".into()),
        ("Super+d", "show-desktop".into()),
        ("Super+Left", "snap-left".into()),
        ("Super+Right", "snap-right".into()),
        ("Super+Up", "maximize".into()),
        ("Super+Down", "minimize".into()),
        ("Alt+F4", "close".into()),
        ("Alt+Tab", "cycle-windows".into()),
        ("Super+F11", "fullscreen".into()),
        ("Super+Shift+r", "reload-config".into()),
        ("Ctrl+Alt+BackSpace", "quit".into()),
        ("Print", shell("screenshot \"'screen'\"")),
        ("Shift+Print", shell("screenshot \"'area'\"")),
        ("Super+Shift+s", shell("screenshot \"'area'\"")),
        ("Super+period", shell("emoji")),
        ("XF86AudioRaiseVolume", shell("volume \"'up'\"")),
        ("XF86AudioLowerVolume", shell("volume \"'down'\"")),
        ("XF86AudioMute", shell("volume \"'mute'\"")),
        ("XF86AudioMicMute", shell("volume \"'mic-mute'\"")),
        ("XF86MonBrightnessUp", shell("brightness \"'up'\"")),
        ("XF86MonBrightnessDown", shell("brightness \"'down'\"")),
    ]
}

impl Default for Config {
    fn default() -> Self {
        let mut binds = BTreeMap::new();
        for (combo, action) in default_binds() {
            binds.insert(
                Combo::parse(combo).expect("default key combination"),
                Action::parse(&action).expect("default action"),
            );
        }
        for vt in 1..=12 {
            binds.insert(
                Combo::parse(&format!("Ctrl+Alt+XF86Switch_VT_{vt}")).expect("vt combo"),
                Action::SwitchVt(vt),
            );
        }
        Config {
            // The same start-up as the labwc session: environment for D-Bus
            // services, sound, the password prompt agent and the desktop.
            autostart: vec!["sh /usr/share/seglinux/labwc/autostart".into()],
            background: [0.043, 0.071, 0.125, 1.0],
            focus_follows_mouse: false,
            keyboard: Keyboard::default(),
            binds,
        }
    }
}

/// `#rrggbb` or `#rrggbbaa` -> RGBA from 0 to 1.
pub fn parse_color(text: &str) -> Option<[f32; 4]> {
    let hex = text.strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    let a = if hex.len() == 8 { byte(6)? } else { 255 };
    Some([byte(0)?, byte(2)?, byte(4)?, a].map(|b| b as f32 / 255.0))
}

impl Config {
    /// The settings in `text`, on top of the defaults. Mistakes in single
    /// entries are reported and skipped, so a typo never leaves the user
    /// without a desktop.
    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut config = Config::default();
        let mut problems = Vec::new();
        let file: File = match toml::from_str(text) {
            Ok(file) => file,
            Err(e) => {
                problems.push(format!("{e}; using the defaults"));
                return (config, problems);
            }
        };
        if let Some(autostart) = file.autostart {
            config.autostart = autostart;
        }
        if let Some(color) = file.background {
            match parse_color(&color) {
                Some(c) => config.background = c,
                None => problems.push(format!("background: `{color}` is not #rrggbb")),
            }
        }
        if let Some(f) = file.focus_follows_mouse {
            config.focus_follows_mouse = f;
        }
        config.keyboard = file.keyboard;
        for (combo, action) in file.binds {
            let combo = match Combo::parse(&combo) {
                Ok(c) => c,
                Err(e) => {
                    problems.push(format!("binds: {e}"));
                    continue;
                }
            };
            // "none" frees a default key for the applications.
            if action.trim() == "none" {
                config.binds.remove(&combo);
                continue;
            }
            match Action::parse(&action) {
                Ok(a) => {
                    config.binds.insert(combo, a);
                }
                Err(e) => problems.push(format!("binds: {e}")),
            }
        }
        (config, problems)
    }

    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("seglinux/compositor.toml"))
    }

    /// The file, or the defaults when there is none.
    pub fn load(path: Option<&Path>) -> (Config, Vec<String>) {
        let Some(path) = path else {
            return (Config::default(), Vec::new());
        };
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let (config, problems) = Config::parse(&text);
                let problems = problems
                    .into_iter()
                    .map(|p| format!("{}: {p}", path.display()))
                    .collect();
                (config, problems)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), Vec::new()),
            Err(e) => (
                Config::default(),
                vec![format!("{}: {e}; using the defaults", path.display())],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos() {
        let c = Combo::parse("Super+Shift+S").unwrap();
        assert!(c.mods.logo && c.mods.shift && !c.mods.ctrl && !c.mods.alt);
        assert_eq!(c.key, "s");
        assert_eq!(Combo::parse("ctrl+alt+BackSpace").unwrap().key, "BackSpace");
        assert_eq!(Combo::parse("Print").unwrap().mods, Mods::default());
        assert!(Combo::parse("Hyper+x").is_err());
        assert!(Combo::parse("Super+").is_err());
        assert_eq!(Combo::parse("Win+e"), Combo::parse("Super+E"));
    }

    #[test]
    fn actions() {
        assert_eq!(
            Action::parse("spawn foot -e htop"),
            Ok(Action::Spawn("foot -e htop".into()))
        );
        assert_eq!(Action::parse(" close "), Ok(Action::Close));
        assert!(Action::parse("spawn").is_err());
        assert!(Action::parse("close now").is_err());
        assert!(Action::parse("dance").is_err());
        assert_eq!(Action::parse("switch-vt 2"), Ok(Action::SwitchVt(2)));
        assert!(Action::parse("switch-vt 13").is_err());
    }

    #[test]
    fn defaults_match_the_labwc_session() {
        let c = Config::default();
        let get = |k: &str| c.binds.get(&Combo::parse(k).unwrap()).cloned();
        assert_eq!(
            get("Super+Return"),
            Some(Action::Spawn("seglinux-terminal".into()))
        );
        assert_eq!(get("Alt+F4"), Some(Action::Close));
        assert_eq!(get("Alt+Tab"), Some(Action::CycleWindows));
        assert_eq!(get("Ctrl+Alt+XF86Switch_VT_3"), Some(Action::SwitchVt(3)));
        // Every key of labwc's rc.xml has its counterpart.
        let rc = include_str!("../../../data/desktop/labwc/rc.xml");
        for (labwc_key, ours) in [
            ("W-space", "Super+space"),
            ("W-e", "Super+e"),
            ("W-Return", "Super+Return"),
            ("W-i", "Super+i"),
            ("W-l", "Super+l"),
            ("W-d", "Super+d"),
            ("W-Left", "Super+Left"),
            ("W-Right", "Super+Right"),
            ("Print", "Print"),
            ("S-Print", "Shift+Print"),
            ("W-S-s", "Super+Shift+s"),
            ("W-period", "Super+period"),
            ("XF86AudioMute", "XF86AudioMute"),
            ("XF86MonBrightnessUp", "XF86MonBrightnessUp"),
        ] {
            assert!(rc.contains(&format!("key=\"{labwc_key}\"")), "{labwc_key}");
            assert!(get(ours).is_some(), "{ours}");
        }
    }

    #[test]
    fn a_file_changes_and_removes_keys_and_reports_mistakes() {
        let (c, problems) = Config::parse(
            r##"
autostart = ["seglinux-shell", "nm-applet"]
background = "#112233"
focus_follows_mouse = true

[keyboard]
layout = "us,ara"
options = "grp:alt_shift_toggle"

[binds]
"Super+Return" = "spawn foot"
"Alt+Tab" = "none"
"Super+x" = "explode"
"Bogus+y" = "close"
"##,
        );
        assert_eq!(c.autostart, ["seglinux-shell", "nm-applet"]);
        assert_eq!(c.background[3], 1.0);
        assert!((c.background[0] - 0x11 as f32 / 255.0).abs() < 1e-6);
        assert!(c.focus_follows_mouse);
        assert_eq!(c.keyboard.layout, "us,ara");
        assert_eq!(
            c.binds.get(&Combo::parse("Super+Return").unwrap()),
            Some(&Action::Spawn("foot".into()))
        );
        assert!(!c.binds.contains_key(&Combo::parse("Alt+Tab").unwrap()));
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    #[test]
    fn a_broken_file_gives_the_defaults() {
        let (c, problems) = Config::parse("binds = [");
        assert_eq!(c, Config::default());
        assert_eq!(problems.len(), 1);
        let (c, problems) = Config::parse("unknown_key = 1");
        assert_eq!(c, Config::default());
        assert_eq!(problems.len(), 1);
        assert_eq!(
            parse_color("#ff000080"),
            Some([1.0, 0.0, 0.0, 128.0 / 255.0])
        );
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("#ééé"), None);
    }
}
