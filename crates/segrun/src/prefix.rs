//! Wine prefixes: SegLinux's "C: drives".
//!
//! Prefixes live in `~/.local/share/seglinux/prefixes/<name>`. Each one
//! carries a small state file recording what segrun already set up in it,
//! so DXVK and friends are only installed once.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = ".seglinux-prefix.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrefixState {
    pub created_by: String,
    pub runner: String,
    pub dxvk: bool,
    pub vkd3d: bool,
    /// winetricks verbs installed through segrun.
    pub verbs: Vec<String>,
}

impl PrefixState {
    /// Record whether an optional setup step is in place.
    pub fn mark(&mut self, step: crate::launch::StepKind, done: bool) {
        use crate::launch::StepKind;
        match step {
            StepKind::Dxvk => self.dxvk = done,
            StepKind::Vkd3d => self.vkd3d = done,
            StepKind::InitPrefix => {}
        }
    }
}

pub fn root() -> PathBuf {
    seglinux_core::xdg::seglinux_data().join("prefixes")
}

pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        bail!("invalid prefix name `{name}`: use letters, digits, `-`, `_` and `.`");
    }
    Ok(())
}

pub fn dir(name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(root().join(name))
}

pub fn load_state(prefix_dir: &Path) -> Option<PrefixState> {
    let text = fs::read_to_string(prefix_dir.join(STATE_FILE)).ok()?;
    toml::from_str(&text).ok()
}

pub fn save_state(prefix_dir: &Path, state: &PrefixState) -> Result<()> {
    fs::create_dir_all(prefix_dir)?;
    let path = prefix_dir.join(STATE_FILE);
    fs::write(&path, toml::to_string(state)?)
        .with_context(|| format!("cannot write {}", path.display()))
}

/// A prefix counts as created once Wine wrote its registry.
pub fn is_initialized(wineprefix: &Path) -> bool {
    wineprefix.join("system.reg").exists()
}

/// All prefix names, sorted.
pub fn list() -> Result<Vec<String>> {
    let root = root();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = fs::read_dir(&root)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    Ok(names)
}

/// A prefix name derived from a program's file name, used when installers
/// are isolated: `Setup_v2.1 (x64).exe` becomes `setup-v2-1-x64`. Generic
/// installer names are qualified with their folder name.
pub fn slug_for(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let generic = matches!(
        stem.to_ascii_lowercase().as_str(),
        "setup" | "install" | "installer" | "autorun" | "launcher"
    );
    let base = match path.parent().and_then(|p| p.file_name()) {
        Some(parent) if generic => format!("{}-{stem}", parent.to_string_lossy()),
        _ => stem,
    };
    let mut slug = String::new();
    for c in base.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').chars().take(48).collect::<String>();
    if slug.is_empty() {
        "app".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(validate_name("default").is_ok());
        assert!(validate_name("my-game_2.0").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("../etc").is_err());
        assert!(validate_name(".hidden").is_err());
        assert!(validate_name("a/b").is_err());
    }

    #[test]
    fn slugs() {
        assert_eq!(
            slug_for(Path::new("/dl/Setup_v2.1 (x64).exe")),
            "setup-v2-1-x64"
        );
        assert_eq!(slug_for(Path::new("/dl/GIMP/setup.exe")), "gimp-setup");
        assert_eq!(slug_for(Path::new("/!!!.exe")), "app");
        assert!(validate_name(&slug_for(Path::new("/x/Ünïcødé app.exe"))).is_ok());
    }

    #[test]
    fn state_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_state(dir.path()), None);
        let state = PrefixState {
            created_by: "segrun".into(),
            runner: "wine".into(),
            dxvk: true,
            vkd3d: false,
            verbs: vec!["corefonts".into()],
        };
        save_state(dir.path(), &state).unwrap();
        assert_eq!(load_state(dir.path()), Some(state));
    }
}
