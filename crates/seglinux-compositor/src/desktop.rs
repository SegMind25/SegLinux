//! The SegLinux Settings app's choices (`~/.config/seglinux/shell.toml`,
//! written by seglinux-shell): keyboard layout and repeat, mouse and
//! touchpad, focus, workspaces. The same file drives the labwc session, so
//! one Settings app serves both. Read leniently: only the keys used here,
//! anything else is ignored.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Input {
    pub keyboard_layout: String,
    pub layout_switch: String,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
    /// -100 (slowest) to 100.
    pub pointer_speed: i32,
    pub mouse_acceleration: bool,
    pub natural_scroll_mouse: bool,
    pub natural_scroll_touchpad: bool,
    pub tap_to_click: bool,
    pub disable_touchpad_while_typing: bool,
    pub left_handed: bool,
}

impl Default for Input {
    /// The Settings app's own defaults.
    fn default() -> Self {
        Input {
            keyboard_layout: String::new(),
            layout_switch: "alt-shift".into(),
            repeat_rate: 25,
            repeat_delay: 600,
            pointer_speed: 0,
            mouse_acceleration: true,
            natural_scroll_mouse: false,
            natural_scroll_touchpad: false,
            tap_to_click: true,
            disable_touchpad_while_typing: true,
            left_handed: false,
        }
    }
}

impl Input {
    /// The xkb option switching between layouts, when there are several.
    pub fn xkb_options(&self) -> Option<String> {
        if !self.keyboard_layout.contains(',') {
            return None;
        }
        match self.layout_switch.as_str() {
            "alt-shift" => Some("grp:alt_shift_toggle".into()),
            "ctrl-shift" => Some("grp:ctrl_shift_toggle".into()),
            "caps-lock" => Some("grp:caps_toggle".into()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Windows {
    pub focus_follows_mouse: bool,
    pub workspaces: u32,
}

impl Default for Windows {
    fn default() -> Self {
        Windows {
            focus_follows_mouse: false,
            workspaces: 1,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Desktop {
    pub input: Input,
    pub windows: Windows,
}

impl Desktop {
    pub fn parse(text: &str) -> Desktop {
        toml::from_str(text).unwrap_or_else(|e| {
            tracing::warn!("shell.toml: {e}; using the defaults for keyboard and mouse");
            Desktop::default()
        })
    }

    pub fn path() -> Option<PathBuf> {
        Some(crate::config::Config::default_path()?.with_file_name("shell.toml"))
    }

    pub fn load() -> Desktop {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Desktop::parse(&t))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_the_settings_app_writes_and_ignores_the_rest() {
        let d = Desktop::parse(
            r#"
wallpaper = "/usr/share/backgrounds/x.jpg"
pinned = ["steam"]

[windows]
focus_follows_mouse = true
gap = 4
workspaces = 4

[input]
keyboard_layout = "us,ara"
layout_switch = "ctrl-shift"
pointer_speed = -30
tap_to_click = false
"#,
        );
        assert!(d.windows.focus_follows_mouse);
        assert_eq!(d.windows.workspaces, 4);
        assert_eq!(d.input.pointer_speed, -30);
        assert!(!d.input.tap_to_click);
        assert_eq!(
            d.input.xkb_options().as_deref(),
            Some("grp:ctrl_shift_toggle")
        );
        // Untouched keys keep the Settings app's defaults.
        assert_eq!(d.input.repeat_rate, 25);
        assert!(d.input.disable_touchpad_while_typing);
    }

    #[test]
    fn one_layout_needs_no_switch_and_a_broken_file_gives_the_defaults() {
        let d = Desktop::parse("[input]\nkeyboard_layout = 'fr'\n");
        assert_eq!(d.input.xkb_options(), None);
        assert_eq!(Desktop::parse("[input\n"), Desktop::default());
    }
}
