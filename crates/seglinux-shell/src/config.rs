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
        }
    }
}

pub const CLOCK_24H: &str = "%x  %H:%M";
pub const CLOCK_12H: &str = "%x  %I:%M %p";
impl Config {
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
                .and_then(|s| toml::from_str(&s).map_err(|e| e.to_string()))
            {
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
    fn saved_config_round_trips() {
        let c = Config::default();
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.pinned, c.pinned);
        assert_eq!(back.clock_format, c.clock_format);
    }
}
