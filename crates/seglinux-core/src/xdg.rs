//! XDG base directories (only the parts SegLinux uses).

use std::env;
use std::path::PathBuf;

fn home() -> PathBuf {
    env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn base(var: &str, fallback: &str) -> PathBuf {
    env::var_os(var)
        .map(PathBuf::from)
        // The spec says relative paths must be ignored.
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

pub fn config_home() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config")
}

pub fn data_home() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share")
}

pub fn state_home() -> PathBuf {
    base("XDG_STATE_HOME", ".local/state")
}

pub fn cache_home() -> PathBuf {
    base("XDG_CACHE_HOME", ".cache")
}

/// `~/.config/seglinux`
pub fn seglinux_config() -> PathBuf {
    config_home().join("seglinux")
}

/// `~/.local/share/seglinux`
pub fn seglinux_data() -> PathBuf {
    data_home().join("seglinux")
}

/// `~/.local/share/applications`
pub fn applications_dir() -> PathBuf {
    data_home().join("applications")
}
