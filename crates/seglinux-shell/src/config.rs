use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
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

pub const CLOCK_24H: &str = "%x  %H:%M";
pub const CLOCK_12H: &str = "%x  %I:%M %p";
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.accent.len() != 7
            || !self.accent.starts_with('#')
            || !self.accent[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Accent must be a six-digit hex color, such as #3daee9.".into());
        }
        if !(10..=24).contains(&self.font_size)
            || !(20..=64).contains(&self.dock_icon_size)
            || !(24..=96).contains(&self.desktop_icon_size)
        {
            return Err("Font or icon size is outside the supported range.".into());
        }
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
        Ok(())
    }
    pub fn css(&self) -> String {
        let mut css = include_str!("../../../data/desktop/shell.css")
            .replace("#3daee9", &self.accent)
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
            css = css
                .replace("#202326", "#f3f4f6")
                .replace("#eff0f1", "#202326");
        }
        css.push_str(&format!(
            "\nwindow {{ font-size: {}px; }}\n.dock .start image, .dock .app image {{ -gtk-icon-size: {}px; }}\n",
            self.font_size, self.dock_icon_size
        ));
        css
    }
    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
        home.map(|p| p.join("seglinux/shell.toml"))
    }
    pub fn load() -> Self {
        let path = Self::path();
        match path.filter(|p| p.exists()) {
            Some(path) => match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| {
                    toml::from_str::<Config>(&s)
                        .map_err(|e| e.to_string())
                        .and_then(|c| {
                            c.validate()?;
                            Ok(c)
                        })
                }) {
                Ok(config) => config,
                Err(error) => {
                    eprintln!("{}: {error}; using desktop defaults", path.display());
                    Self::default()
                }
            },
            None => Self::default(),
        }
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
        let c: Config = toml::from_str("pinned = ['steam']\ndesktop_icons = false").unwrap();
        assert_eq!(c.pinned, ["steam"]);
        assert!(!c.desktop_icons);
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
    }
    #[test]
    fn saved_config_round_trips() {
        let c = Config::default();
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.pinned, c.pinned);
        assert_eq!(back.clock_format, c.clock_format);
    }
}
