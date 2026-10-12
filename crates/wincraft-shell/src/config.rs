use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub wallpaper: PathBuf,
    pub clock_format: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            wallpaper: "/usr/share/backgrounds/wincraft/wincraft-dark.png".into(),
            clock_format: "%H:%M\n%a, %d %b".into(),
        }
    }
}
impl Config {
    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
        home.map(|p| p.join("wincraft/shell.toml"))
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
        assert!(toml::from_str::<Config>("wallpapre = 'oops'").is_err());
    }
}
