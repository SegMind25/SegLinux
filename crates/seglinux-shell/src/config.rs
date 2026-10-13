//! `~/.config/seglinux/shell.toml`: the desktop's preferences, from the
//! wallpaper and dock to window behaviour, input devices and gaming.
//!
//! Every field has a default, so older files keep working and a file may
//! name only what it changes. Unknown keys are rejected, which catches typos.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub wallpaper: PathBuf,
    /// strftime-style format of the dock clock (`%x` is the local date).
    pub clock_format: String,
    /// Applications pinned to the dock, as desktop file ids without
    /// `.desktop`; ones that are not installed are skipped.
    pub pinned: Vec<String>,
    /// Show the files in ~/Desktop as icons on the desktop.
    pub desktop_icons: bool,
    pub theme: Theme,
    pub accent: String,
    pub font_size: u32,
    pub animations: bool,
    pub dock_position: DockPosition,
    pub dock_icon_size: u32,
    pub desktop_icon_size: u32,
    pub wallpaper_fit: WallpaperFit,
    /// Opacity of the dock and menus' background, in percent.
    pub dock_opacity: u32,
    /// The "Show desktop" button next to Start.
    pub show_desktop_button: bool,
    /// The battery level as a number next to its icon, on laptops.
    pub battery_percentage: bool,
    pub windows: Windows,
    pub input: Input,
    pub overlay: Overlay,
    pub gaming_mode: GamingMode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wallpaper: "/usr/share/backgrounds/seglinux/seglinux-dark.jpg".into(),
            clock_format: CLOCK_24H.into(),
            pinned: [
                "org.kde.dolphin",
                "firefox",
                "seglinux-terminal",
                "steam",
                "net.lutris.Lutris",
            ]
            .map(String::from)
            .to_vec(),
            desktop_icons: true,
            theme: Theme::Dark,
            accent: "#3daee9".into(),
            font_size: 13,
            animations: true,
            dock_position: DockPosition::Bottom,
            dock_icon_size: 26,
            desktop_icon_size: 48,
            wallpaper_fit: WallpaperFit::Cover,
            dock_opacity: 86,
            show_desktop_button: true,
            battery_percentage: true,
            windows: Windows::default(),
            input: Input::default(),
            overlay: Overlay::default(),
            gaming_mode: GamingMode::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DockPosition {
    #[default]
    Bottom,
    Top,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperFit {
    #[default]
    Cover,
    Contain,
    Fill,
}

/// Window management, applied by the labwc compositor.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Windows {
    /// Focus the window under the mouse pointer.
    pub focus_follows_mouse: bool,
    /// Bring a window to the front when it gets the focus.
    pub raise_on_focus: bool,
    /// Space around snapped and maximized windows, in pixels.
    pub gap: u32,
    /// Roundness of the title bars' top corners, in pixels.
    pub corner_radius: u32,
    /// Virtual desktops (Ctrl+Super+Left/Right switches between them).
    pub workspaces: u32,
    /// Variable refresh rate (FreeSync / G-Sync).
    pub adaptive_sync: AdaptiveSync,
    /// Tearing, for the lowest input latency in games.
    pub allow_tearing: Tearing,
}
impl Default for Windows {
    fn default() -> Self {
        Self {
            focus_follows_mouse: false,
            raise_on_focus: false,
            gap: 8,
            corner_radius: 8,
            workspaces: 1,
            adaptive_sync: AdaptiveSync::Fullscreen,
            allow_tearing: Tearing::Yes,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AdaptiveSync {
    No,
    /// Only while a window is fullscreen: games, not the desktop.
    #[default]
    Fullscreen,
    Yes,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tearing {
    No,
    /// When the active window asks for it, as games do.
    #[default]
    Yes,
    /// For every fullscreen window.
    Fullscreen,
}

/// Keyboard, mouse and touchpad.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    /// XKB layouts separated by commas, such as `fr` or `us,ara`; empty
    /// for the layout chosen in Setup.
    pub keyboard_layout: String,
    /// The keys that switch between several layouts.
    pub layout_switch: LayoutSwitch,
    /// Repeated key presses per second while a key is held.
    pub repeat_rate: u32,
    /// Milliseconds before a held key repeats.
    pub repeat_delay: u32,
    pub numlock: NumLock,
    /// From -100 (slowest) to 100 (fastest).
    pub pointer_speed: i32,
    /// Pointer acceleration; off moves the pointer exactly with the mouse,
    /// as many players prefer.
    pub mouse_acceleration: bool,
    pub natural_scroll_mouse: bool,
    pub natural_scroll_touchpad: bool,
    pub tap_to_click: bool,
    pub disable_touchpad_while_typing: bool,
    pub left_handed: bool,
    /// Mouse pointer size, in pixels.
    pub cursor_size: u32,
}
impl Default for Input {
    fn default() -> Self {
        Self {
            keyboard_layout: String::new(),
            layout_switch: LayoutSwitch::AltShift,
            repeat_rate: 25,
            repeat_delay: 600,
            numlock: NumLock::Unchanged,
            pointer_speed: 0,
            mouse_acceleration: true,
            natural_scroll_mouse: false,
            natural_scroll_touchpad: false,
            tap_to_click: true,
            disable_touchpad_while_typing: true,
            left_handed: false,
            cursor_size: 24,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutSwitch {
    #[default]
    AltShift,
    CtrlShift,
    CapsLock,
    None,
}
impl LayoutSwitch {
    /// The XKB option, for `XKB_DEFAULT_OPTIONS`.
    pub fn xkb_option(self) -> Option<&'static str> {
        match self {
            Self::AltShift => Some("grp:alt_shift_toggle"),
            Self::CtrlShift => Some("grp:ctrl_shift_toggle"),
            Self::CapsLock => Some("grp:caps_toggle"),
            Self::None => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NumLock {
    /// Leave Num Lock as the keyboard reports it.
    #[default]
    Unchanged,
    On,
    Off,
}

/// The MangoHud performance overlay segrun shows when `hud` is on.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Overlay {
    pub position: OverlayPosition,
    pub detail: OverlayDetail,
    /// Frame rate limit for every game, 0 for none.
    pub fps_limit: u32,
}
impl Default for Overlay {
    fn default() -> Self {
        Self {
            position: OverlayPosition::TopLeft,
            detail: OverlayDetail::Standard,
            fps_limit: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayPosition {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayDetail {
    /// The frame rate alone.
    FpsOnly,
    /// One line across the top of the screen.
    Horizontal,
    /// MangoHud's own default: frame rate, frame time, CPU and GPU load.
    #[default]
    Standard,
    /// Temperatures, clocks, memory and more.
    Detailed,
}

/// The "SegLinux Gaming Mode" session: Steam Big Picture in gamescope.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GamingMode {
    /// Output resolution; 0 picks the display's own.
    pub width: u32,
    pub height: u32,
    /// Refresh rate in Hz; 0 picks the display's own.
    pub refresh_rate: u32,
    pub adaptive_sync: bool,
    pub hdr: bool,
    /// The MangoHud overlay inside gamescope.
    pub performance_overlay: bool,
}
impl Default for GamingMode {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            refresh_rate: 0,
            adaptive_sync: true,
            hdr: false,
            performance_overlay: false,
        }
    }
}
impl GamingMode {
    /// gamescope's options for the Gaming Mode session.
    pub fn gamescope_args(&self) -> Vec<String> {
        let mut args = vec!["-e".to_string()];
        for (flag, value) in [
            ("-W", self.width),
            ("-H", self.height),
            ("-r", self.refresh_rate),
        ] {
            if value > 0 {
                args.extend([flag.to_string(), value.to_string()]);
            }
        }
        for (flag, on) in [
            ("--adaptive-sync", self.adaptive_sync),
            ("--hdr-enabled", self.hdr),
            ("--mangoapp", self.performance_overlay),
        ] {
            if on {
                args.push(flag.to_string());
            }
        }
        args
    }
}

pub const CLOCK_24H: &str = "%x  %H:%M";
pub const CLOCK_12H: &str = "%x  %I:%M %p";
/// Clock formats offered as presets: label and strftime format.
pub const CLOCK_PRESETS: &[(&str, &str)] = &[
    ("Date and 24-hour time", CLOCK_24H),
    ("Date and 12-hour time", CLOCK_12H),
    ("24-hour time", "%H:%M"),
    ("12-hour time", "%I:%M %p"),
    ("24-hour time with seconds", "%H:%M:%S"),
    ("Weekday and time", "%a %H:%M"),
];
/// Accent colors offered as swatches: name and color.
pub const ACCENTS: &[(&str, &str)] = &[
    ("Blue", "#3daee9"),
    ("Indigo", "#6d6bff"),
    ("Purple", "#9b59d0"),
    ("Pink", "#e93d8f"),
    ("Red", "#da4453"),
    ("Orange", "#f67400"),
    ("Yellow", "#fdbc4b"),
    ("Green", "#27ae60"),
    ("Teal", "#1abc9c"),
    ("Graphite", "#7f8c8d"),
];

fn in_range(what: &str, value: u32, range: std::ops::RangeInclusive<u32>) -> Result<(), String> {
    if range.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{what} must be between {} and {}.",
            range.start(),
            range.end()
        ))
    }
}

/// Whether `layout` is a list of XKB layouts such as `us`, `fr,ara` or
/// `us(intl)`: it ends up in the compositor's environment file.
pub fn valid_keyboard_layout(layout: &str) -> bool {
    layout.is_empty()
        || (layout.len() <= 64
            && layout.split(',').count() <= 4
            && layout.split(',').all(|part| {
                let (name, variant) = match part.split_once('(') {
                    Some((name, rest)) => match rest.strip_suffix(')') {
                        Some(variant) => (name, Some(variant)),
                        None => return false,
                    },
                    None => (part, None),
                };
                let word = |s: &str| {
                    !s.is_empty()
                        && s.bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                };
                word(name) && variant.is_none_or(word)
            }))
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.accent.len() != 7
            || !self.accent.starts_with('#')
            || !self.accent[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Accent must be a six-digit hex color, such as #3daee9.".into());
        }
        in_range("Text size", self.font_size, 10..=24)?;
        in_range("Dock icon size", self.dock_icon_size, 20..=64)?;
        in_range("Desktop icon size", self.desktop_icon_size, 24..=96)?;
        in_range("Dock opacity", self.dock_opacity, 30..=100)?;
        if self.clock_format.trim().is_empty() || self.clock_format.len() > 128 {
            return Err("Enter a clock format between 1 and 128 characters.".into());
        }
        if self.pinned.len() > 30
            || self.pinned.iter().any(|id| {
                id.is_empty()
                    || !id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            })
        {
            return Err("Use up to 30 desktop application IDs containing letters, numbers, dots, hyphens or underscores.".into());
        }
        let w = &self.windows;
        in_range("The gap between windows", w.gap, 0..=48)?;
        in_range("Title bar roundness", w.corner_radius, 0..=20)?;
        in_range("The number of workspaces", w.workspaces, 1..=12)?;
        let i = &self.input;
        if !valid_keyboard_layout(&i.keyboard_layout) {
            return Err("Keyboard layouts are XKB names separated by commas, such as fr or us,ara (up to four).".into());
        }
        in_range("Key repeat rate", i.repeat_rate, 1..=100)?;
        in_range("Key repeat delay", i.repeat_delay, 100..=2000)?;
        if !(-100..=100).contains(&i.pointer_speed) {
            return Err("Pointer speed must be between -100 and 100.".into());
        }
        in_range("Pointer size", i.cursor_size, 16..=96)?;
        in_range("The frame rate limit", self.overlay.fps_limit, 0..=1000)?;
        let g = &self.gaming_mode;
        in_range("Gaming Mode width", g.width, 0..=16384)?;
        in_range("Gaming Mode height", g.height, 0..=16384)?;
        in_range("Gaming Mode refresh rate", g.refresh_rate, 0..=1000)?;
        if (g.width == 0) != (g.height == 0) {
            return Err("Set both the Gaming Mode width and height, or neither.".into());
        }
        Ok(())
    }

    /// The shell's style sheet, with this configuration's colors and sizes.
    pub fn css(&self) -> String {
        let alpha = f64::from(self.dock_opacity) / 100.0;
        let mut css = include_str!("../../../data/desktop/shell.css")
            .replace("#3daee9", &self.accent)
            .replace(
                "alpha(#202326, 0.86)",
                &format!("alpha(#202326, {alpha:.2})"),
            )
            .replace(
                "font-size: 13px",
                &format!("font-size: {}px", self.font_size),
            )
            .replace(
                "font-size: 12px",
                &format!("font-size: {}px", self.font_size.saturating_sub(1)),
            )
            .replace(
                "font-size: 17px",
                &format!("font-size: {}px", self.font_size + 4),
            );
        if self.theme == Theme::Light {
            // Dark text on light surfaces; lines and hover shades in black.
            css = css
                .replace("#202326", "#f3f4f6")
                .replace("#eff0f1", "#202326")
                .replace("alpha(#ffffff,", "alpha(#000000,");
        }
        css.push_str(&format!(
            "\nwindow {{ font-size: {}px; }}\n.dock .start image, .dock .app image {{ -gtk-icon-size: {}px; }}\n",
            self.font_size, self.dock_icon_size
        ));
        css
    }

    pub fn path() -> PathBuf {
        seglinux_core::xdg::seglinux_config().join("shell.toml")
    }

    /// The configuration in `text`, checked.
    pub fn parse(text: &str) -> Result<Self, String> {
        let config = toml::from_str::<Config>(text).map_err(|e| e.to_string())?;
        config.validate()?;
        Ok(config)
    }

    /// The user's configuration, the defaults when there is none, or why
    /// it cannot be used.
    pub fn try_load() -> Result<Self, String> {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// The user's configuration; the defaults when there is none, or with a
    /// warning when it cannot be used.
    pub fn load() -> Self {
        Self::try_load().unwrap_or_else(|error| {
            eprintln!(
                "{}: {error}; using desktop defaults",
                Self::path().display()
            );
            Self::default()
        })
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("the configuration serializes")
    }
}

pub fn matches_query(name: &str, description: &str, query: &str) -> bool {
    let haystack = format!("{name} {description}").to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn searches_all_words_without_case_sensitivity() {
        assert!(matches_query(
            "File Explorer",
            "Browse files",
            "FILE browse"
        ));
        assert!(!matches_query(
            "File Explorer",
            "Browse files",
            "file music"
        ));
        assert!(matches_query("Terminal", "", ""));
    }
    #[test]
    fn config_defaults_and_typos() {
        let c: Config = toml::from_str("clock_format = '%I:%M %p'").unwrap();
        assert_eq!(c.clock_format, "%I:%M %p");
        assert!(c.wallpaper.is_absolute());
        assert!(c.desktop_icons);
        assert!(c.pinned.iter().any(|p| p == "firefox"));
        assert!(toml::from_str::<Config>("wallpapre = 'oops'").is_err());
        assert!(toml::from_str::<Config>("[windows]\ngapp = 3").is_err());
        let c: Config = toml::from_str("pinned = ['steam']\ndesktop_icons = false").unwrap();
        assert_eq!(c.pinned, ["steam"]);
        assert!(!c.desktop_icons);
    }

    #[test]
    fn the_example_file_shows_the_defaults() {
        let example = Config::parse(include_str!("../../../data/desktop/shell.toml")).unwrap();
        assert_eq!(example, Config::default());
    }

    #[test]
    fn older_files_keep_working() {
        // A file from before the windows, input and gaming sections.
        let old = "wallpaper = \"/usr/share/backgrounds/seglinux/seglinux-flow.jpg\"\n\
                   clock_format = \"%x  %I:%M %p\"\npinned = [\"firefox\"]\ndesktop_icons = true\n\
                   theme = \"light\"\naccent = \"#3daee9\"\nfont_size = 13\nanimations = true\n\
                   wallpaper_fit = \"cover\"\ndock_position = \"bottom\"\ndock_icon_size = 26\n\
                   desktop_icon_size = 48\n";
        let c = Config::parse(old).unwrap();
        assert_eq!(c.theme, Theme::Light);
        assert_eq!(c.windows, Windows::default());
        assert_eq!(c.input, Input::default());
        assert_eq!(c.dock_opacity, 86);
        let partial = Config::parse("[input]\nkeyboard_layout = 'fr'\n").unwrap();
        assert_eq!(partial.input.keyboard_layout, "fr");
        assert_eq!(partial.input.repeat_rate, 25);
    }

    #[test]
    fn validates_customization_and_rejects_css_injection() {
        let mut c = Config::default();
        assert!(c.validate().is_ok());
        c.accent = "red; } window { color: red".into();
        assert!(c.validate().is_err());
        c.accent = "#abcdef".into();
        c.dock_icon_size = 1000;
        assert!(c.validate().is_err());
        c.dock_icon_size = 32;
        c.theme = Theme::Light;
        assert!(c.validate().is_ok());
        assert!(c.css().contains("#abcdef"));
        assert!(c.css().contains("#f3f4f6"));
        assert!(!c.css().contains("alpha(#ffffff,"));
        c.dock_opacity = 10;
        assert!(c.validate().is_err());
        c.dock_opacity = 50;
        assert!(c.css().contains("alpha(#f3f4f6, 0.50)"));
    }

    #[test]
    fn every_css_token_exists_in_the_style_sheet() {
        // css() replaces these literals; a renamed color would silently
        // stop following the settings.
        let sheet = include_str!("../../../data/desktop/shell.css");
        for token in [
            "#3daee9",
            "alpha(#202326, 0.86)",
            "font-size: 13px",
            "font-size: 12px",
            "font-size: 17px",
            "#eff0f1",
            "alpha(#ffffff,",
        ] {
            assert!(sheet.contains(token), "{token}");
        }
    }

    #[test]
    fn rejects_out_of_range_windows_input_and_gaming() {
        let check = |f: fn(&mut Config)| {
            let mut c = Config::default();
            f(&mut c);
            c.validate()
        };
        assert!(check(|c| c.windows.workspaces = 0).is_err());
        assert!(check(|c| c.windows.gap = 49).is_err());
        assert!(check(|c| c.input.pointer_speed = -101).is_err());
        assert!(check(|c| c.input.keyboard_layout = "fr\nXKB=evil".into()).is_err());
        assert!(check(|c| c.input.keyboard_layout = "us,ara".into()).is_ok());
        assert!(check(|c| c.input.keyboard_layout = "us(intl),fr".into()).is_ok());
        assert!(check(|c| c.input.keyboard_layout = "us(intl".into()).is_err());
        assert!(check(|c| c.gaming_mode.width = 1920).is_err());
        assert!(
            check(|c| {
                c.gaming_mode.width = 1920;
                c.gaming_mode.height = 1080;
            })
            .is_ok()
        );
    }

    #[test]
    fn gaming_mode_arguments() {
        assert_eq!(
            GamingMode::default().gamescope_args(),
            ["-e", "--adaptive-sync"]
        );
        let g = GamingMode {
            width: 2560,
            height: 1440,
            refresh_rate: 165,
            adaptive_sync: false,
            hdr: true,
            performance_overlay: true,
        };
        assert_eq!(
            g.gamescope_args(),
            [
                "-e",
                "-W",
                "2560",
                "-H",
                "1440",
                "-r",
                "165",
                "--hdr-enabled",
                "--mangoapp"
            ]
        );
    }

    #[test]
    fn saved_config_round_trips() {
        let mut c = Config::default();
        c.windows.workspaces = 4;
        c.input.layout_switch = LayoutSwitch::CapsLock;
        c.overlay.detail = OverlayDetail::FpsOnly;
        let text = c.to_toml();
        assert_eq!(Config::parse(&text).unwrap(), c);
    }
}
