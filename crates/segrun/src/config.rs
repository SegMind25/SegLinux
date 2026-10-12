//! `segrun.toml`: defaults plus per-application overrides.
//!
//! Lookup order: `--config`, `~/.config/seglinux/segrun.toml`,
//! `/etc/seglinux/segrun.toml`, built-in defaults.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const SYSTEM_CONFIG: &str = "/etc/seglinux/segrun.toml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Toggle {
    /// Decide from the program's imports and the available drivers.
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub defaults: Defaults,
    /// Extra environment for every program.
    pub env: BTreeMap<String, String>,
    #[serde(rename = "app", skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<AppOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Defaults {
    /// `wine`, `wine:/path/to/wine`, `proton:/path/to/Proton`, `umu` or
    /// `umu:/path/to/proton`.
    pub runner: String,
    /// The shared prefix: SegLinux's "C: drive".
    pub prefix: String,
    /// Give every installer its own prefix instead of the shared one.
    pub isolate_installers: bool,
    /// Direct3D 9/10/11 through Vulkan.
    pub dxvk: Toggle,
    /// Direct3D 12 through Vulkan.
    pub vkd3d: Toggle,
    pub esync: bool,
    pub fsync: bool,
    /// Keep Wine's debug output (normally silenced with WINEDEBUG=-all).
    pub debug: bool,
    /// Performance overlay: MangoHud when installed, otherwise DXVK's HUD.
    pub hud: bool,
    /// Run games under Feral GameMode (CPU governor, I/O and GPU tuning
    /// while the game runs). `auto` = programs that use Direct3D, OpenGL
    /// or Vulkan.
    pub gamemode: Toggle,
    /// Run programs inside gamescope, Valve's gaming compositor (frame
    /// limiting, upscaling, fullscreen for windowed-only games).
    pub gamescope: bool,
    /// Arguments for gamescope, e.g. `["-W", "2560", "-H", "1440", "-r", "165"]`.
    pub gamescope_args: Vec<String>,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            runner: "wine".to_string(),
            prefix: "default".to_string(),
            isolate_installers: false,
            dxvk: Toggle::Auto,
            vkd3d: Toggle::Auto,
            esync: true,
            fsync: true,
            debug: false,
            hud: false,
            gamemode: Toggle::Auto,
            gamescope: false,
            gamescope_args: vec!["-f".into()],
        }
    }
}

/// Settings for programs whose file name matches `match`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppOverride {
    /// Case-insensitive glob on the file name, e.g. `"Photoshop*.exe"`.
    #[serde(rename = "match")]
    pub pattern: String,
    pub prefix: Option<String>,
    pub runner: Option<String>,
    pub dxvk: Option<Toggle>,
    pub vkd3d: Option<Toggle>,
    pub virtual_desktop: Option<String>,
    pub gamemode: Option<Toggle>,
    pub gamescope: Option<bool>,
    /// Replaces the default gamescope arguments when not empty.
    pub gamescope_args: Vec<String>,
    /// Arguments added before the ones given on the command line.
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// `WINEDLLOVERRIDES` entries, e.g. `"d3dcompiler_47=n,b"`.
    pub dll_overrides: Vec<String>,
}

impl Config {
    pub fn user_path() -> PathBuf {
        seglinux_core::xdg::seglinux_config().join("segrun.toml")
    }

    /// Load the first config file that exists.
    pub fn load(explicit: Option<&Path>) -> Result<(Config, Option<PathBuf>)> {
        let candidates: Vec<PathBuf> = match explicit {
            Some(p) => vec![p.to_path_buf()],
            None => vec![Self::user_path(), PathBuf::from(SYSTEM_CONFIG)],
        };
        for path in candidates {
            if path.exists() || explicit.is_some() {
                let text = fs::read_to_string(&path)
                    .with_context(|| format!("cannot read {}", path.display()))?;
                let config = Self::parse(&text)
                    .with_context(|| format!("invalid configuration in {}", path.display()))?;
                return Ok((config, Some(path)));
            }
        }
        Ok((Config::default(), None))
    }

    pub fn parse(text: &str) -> Result<Config> {
        Ok(toml::from_str(text)?)
    }

    /// The first override whose pattern matches `file_name`.
    pub fn app_for(&self, file_name: &str) -> Option<&AppOverride> {
        self.apps.iter().find(|a| glob_match(&a.pattern, file_name))
    }
}

/// Case-insensitive glob supporting `*` and `?`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let text = toml::to_string(&Config::default()).unwrap();
        assert_eq!(Config::parse(&text).unwrap(), Config::default());
    }

    #[test]
    fn parse_full_example() {
        let cfg = Config::parse(
            r#"
            [defaults]
            runner = "umu"
            dxvk = "never"
            hud = true

            [env]
            DXVK_ASYNC = "1"

            [[app]]
            match = "Photoshop*.exe"
            prefix = "adobe"
            dxvk = "always"
            virtual_desktop = "1920x1080"
            dll_overrides = ["d3dcompiler_47=n,b"]
            env = { MESA_GL_VERSION_OVERRIDE = "4.6" }
            "#,
        )
        .unwrap();
        assert_eq!(cfg.defaults.runner, "umu");
        assert_eq!(cfg.defaults.dxvk, Toggle::Never);
        assert_eq!(cfg.defaults.prefix, "default");
        assert_eq!(cfg.env["DXVK_ASYNC"], "1");
        let app = cfg.app_for("photoshop_2024.EXE").unwrap();
        assert_eq!(app.prefix.as_deref(), Some("adobe"));
        assert!(cfg.app_for("notepad++.exe").is_none());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(Config::parse("[defaults]\nrunnr = \"wine\"").is_err());
    }

    #[test]
    fn glob() {
        assert!(glob_match("*.exe", "Setup.EXE"));
        assert!(glob_match("setup?.exe", "setup2.exe"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("a*b*c", "aXXbYY"));
        assert!(!glob_match("setup.exe", "setup.exe.bak"));
    }
}
