//! The labwc compositor's configuration, generated from the window and
//! input settings.
//!
//! The session starts labwc with `~/.config/seglinux/labwc` as its
//! configuration folder. Until the user changes a window or input setting,
//! its files are links to the distribution's defaults in
//! `/usr/share/seglinux/labwc`, so updates reach them; afterwards `rc.xml`
//! and `environment` are written here, marked as managed by SegLinux
//! Settings. Files the user wrote by hand (without the mark) are left alone.
use crate::config::{AdaptiveSync, Config, Input, NumLock, Tearing, Windows};
use crate::managed::{self, MARK, Outcome, Wanted};
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

pub const SYSTEM_DIR: &str = "/usr/share/seglinux/labwc";

pub fn user_dir() -> PathBuf {
    seglinux_core::xdg::seglinux_config().join("labwc")
}

fn yes(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Keyboard shortcuts of the SegLinux desktop.
const KEYBINDS: &[(&str, &str)] = &[
    (
        "W-space",
        r#"<action name="Execute" command="gapplication action org.seglinux.Shell menu" />"#,
    ),
    ("W-e", r#"<action name="Execute" command="dolphin" />"#),
    (
        "W-Return",
        r#"<action name="Execute" command="seglinux-terminal" />"#,
    ),
    (
        "W-i",
        r#"<action name="Execute" command="seglinux-shell --settings" />"#,
    ),
    (
        "W-l",
        r#"<action name="Execute" command="swaylock -c 090f1d" />"#,
    ),
    ("W-d", r#"<action name="ToggleShowDesktop" />"#),
    ("W-Left", r#"<action name="SnapToEdge" direction="left" />"#),
    (
        "W-Right",
        r#"<action name="SnapToEdge" direction="right" />"#,
    ),
    (
        "C-W-Left",
        r#"<action name="GoToDesktop" to="left" wrap="yes" />"#,
    ),
    (
        "C-W-Right",
        r#"<action name="GoToDesktop" to="right" wrap="yes" />"#,
    ),
    (
        "C-S-W-Left",
        r#"<action name="SendToDesktop" to="left" wrap="yes" />"#,
    ),
    (
        "C-S-W-Right",
        r#"<action name="SendToDesktop" to="right" wrap="yes" />"#,
    ),
    (
        "XF86AudioRaiseVolume",
        r#"<action name="Execute" command="wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+" />"#,
    ),
    (
        "XF86AudioLowerVolume",
        r#"<action name="Execute" command="wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-" />"#,
    ),
    (
        "XF86AudioMute",
        r#"<action name="Execute" command="wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle" />"#,
    ),
];

/// labwc's `rc.xml` for these settings. The defaults give the file the
/// distribution installs (`data/desktop/labwc/rc.xml`).
pub fn rc_xml(windows: &Windows, input: &Input) -> String {
    let defaults = *windows == Windows::default() && *input == Input::default();
    let mut x = String::from("<?xml version=\"1.0\"?>\n");
    if defaults {
        x.push_str("<!-- SegLinux desktop: window behaviour, keyboard, mouse and touchpad.\n     Change them in SegLinux Settings (Super+I). -->\n");
    } else {
        let _ = writeln!(
            x,
            "<!-- {MARK}: changes made here are replaced when settings are saved.\n     Remove this comment to keep your own version. -->"
        );
    }
    x.push_str("<labwc_config>\n");
    let adaptive = match windows.adaptive_sync {
        AdaptiveSync::No => "no",
        AdaptiveSync::Fullscreen => "fullscreen",
        AdaptiveSync::Yes => "yes",
    };
    let tearing = match windows.allow_tearing {
        Tearing::No => "no",
        Tearing::Yes => "yes",
        Tearing::Fullscreen => "fullscreen",
    };
    let _ = write!(
        x,
        "  <core>
    <decoration>server</decoration><gap>{gap}</gap>
    <!-- Gaming: variable refresh rate (FreeSync/G-Sync) for fullscreen
         windows, and tearing when a game asks for it, for the lowest
         input latency. -->
    <adaptiveSync>{adaptive}</adaptiveSync>
    <allowTearing>{tearing}</allowTearing>
  </core>
  <focus>
    <followMouse>{follow}</followMouse>
    <raiseOnFocus>{raise}</raiseOnFocus>
  </focus>
  <desktops number=\"{workspaces}\" />
  <theme>
    <name>SegLinux</name><cornerRadius>{radius}</cornerRadius>
    <font place=\"ActiveWindow\"><name>Noto Sans</name><size>10</size></font>
    <titlebar><layout>icon:iconify,max,close</layout></titlebar>
  </theme>
  <keyboard>
",
        gap = windows.gap,
        follow = yes(windows.focus_follows_mouse),
        raise = yes(windows.raise_on_focus),
        workspaces = windows.workspaces,
        radius = windows.corner_radius,
    );
    match input.numlock {
        NumLock::Unchanged => {}
        NumLock::On => x.push_str("    <numlock>on</numlock>\n"),
        NumLock::Off => x.push_str("    <numlock>off</numlock>\n"),
    }
    let _ = writeln!(
        x,
        "    <repeatRate>{}</repeatRate>\n    <repeatDelay>{}</repeatDelay>\n    <default />",
        input.repeat_rate, input.repeat_delay
    );
    for (key, action) in KEYBINDS {
        let _ = writeln!(x, "    <keybind key=\"{key}\">{action}</keybind>");
    }
    let speed = f64::from(input.pointer_speed) / 100.0;
    let profile = if input.mouse_acceleration {
        "adaptive"
    } else {
        "flat"
    };
    let _ = write!(
        x,
        "  </keyboard>
  <mouse><default /></mouse>
  <libinput>
    <device category=\"non-touch\">
      <naturalScroll>{mouse_natural}</naturalScroll>
      <leftHanded>{left}</leftHanded>
      <pointerSpeed>{speed:.2}</pointerSpeed>
      <accelProfile>{profile}</accelProfile>
    </device>
    <device category=\"touchpad\">
      <naturalScroll>{pad_natural}</naturalScroll>
      <leftHanded>{left}</leftHanded>
      <pointerSpeed>{speed:.2}</pointerSpeed>
      <accelProfile>{profile}</accelProfile>
      <tap>{tap}</tap>
      <disableWhileTyping>{dwt}</disableWhileTyping>
    </device>
  </libinput>
  <windowSwitcher preview=\"yes\"><osd show=\"yes\" /></windowSwitcher>
</labwc_config>
",
        mouse_natural = yes(input.natural_scroll_mouse),
        pad_natural = yes(input.natural_scroll_touchpad),
        left = yes(input.left_handed),
        tap = yes(input.tap_to_click),
        dwt = yes(input.disable_touchpad_while_typing),
    );
    x
}

/// labwc's `environment` file (keyboard layout, pointer size), or `None`
/// when the defaults leave nothing to set.
pub fn environment(input: &Input) -> Option<String> {
    let mut lines = Vec::new();
    if !input.keyboard_layout.is_empty() {
        lines.push(format!("XKB_DEFAULT_LAYOUT={}", input.keyboard_layout));
        if input.keyboard_layout.contains(',')
            && let Some(option) = input.layout_switch.xkb_option()
        {
            lines.push(format!("XKB_DEFAULT_OPTIONS={option}"));
        }
    }
    if input.cursor_size != Input::default().cursor_size {
        lines.push(format!("XCURSOR_SIZE={}", input.cursor_size));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "# {MARK}: changes made here are replaced when settings are saved.\n{}\n",
        lines.join("\n")
    ))
}

/// The result of bringing the compositor's files up to date.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Synced {
    /// Files were changed: labwc should reload its configuration.
    pub changed: bool,
    /// Files edited by hand, which were not replaced.
    pub kept: Vec<PathBuf>,
}

/// Writes the compositor's configuration for `config` into `dir`, linking
/// to `system` for everything left at its default.
pub fn sync_into(config: &Config, dir: &Path, system: &Path) -> io::Result<Synced> {
    std::fs::create_dir_all(dir)?;
    let defaults = config.windows == Windows::default() && config.input == Input::default();
    let files = [
        (
            "rc.xml",
            if defaults {
                Wanted::Link(system.join("rc.xml"))
            } else {
                Wanted::Text(rc_xml(&config.windows, &config.input))
            },
        ),
        (
            "environment",
            environment(&config.input).map_or(Wanted::Absent, Wanted::Text),
        ),
        ("autostart", Wanted::Link(system.join("autostart"))),
        (
            "themerc-override",
            Wanted::Link(system.join("themerc-override")),
        ),
    ];
    let mut synced = Synced::default();
    for (name, wanted) in files {
        let path = dir.join(name);
        // The session's own files only need to exist: whatever the user put
        // in place of autostart or the theme stays.
        if matches!(name, "autostart" | "themerc-override")
            && std::fs::symlink_metadata(&path).is_ok()
        {
            continue;
        }
        match managed::update(&path, &wanted)? {
            Outcome::Unchanged => {}
            Outcome::Written => synced.changed = true,
            Outcome::Kept => synced.kept.push(path),
        }
    }
    Ok(synced)
}

/// Brings the user's compositor configuration up to date and, when it
/// changed inside a running labwc session, makes labwc reload it.
pub fn sync(config: &Config) -> io::Result<Synced> {
    let synced = sync_into(config, &user_dir(), Path::new(SYSTEM_DIR))?;
    if synced.changed && std::env::var_os("LABWC_PID").is_some() {
        // SIGHUP to $LABWC_PID: rc.xml and environment are read again.
        let _ = std::process::Command::new("labwc")
            .arg("--reconfigure")
            .status();
    }
    Ok(synced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LayoutSwitch;

    /// After changing `rc_xml`, regenerate the installed file with
    /// `SEGLINUX_BLESS=1 cargo test -p seglinux-shell defaults_are`.
    #[test]
    fn defaults_are_the_installed_file() {
        let rendered = rc_xml(&Windows::default(), &Input::default());
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/desktop/labwc/rc.xml"
        );
        if std::env::var_os("SEGLINUX_BLESS").is_some() {
            std::fs::write(path, &rendered).unwrap();
        }
        assert_eq!(
            rendered,
            std::fs::read_to_string(path).unwrap(),
            "data/desktop/labwc/rc.xml must be rc_xml() of the defaults"
        );
    }

    #[test]
    fn renders_window_and_input_settings() {
        let windows = Windows {
            focus_follows_mouse: true,
            gap: 0,
            workspaces: 4,
            adaptive_sync: AdaptiveSync::No,
            allow_tearing: Tearing::Fullscreen,
            ..Windows::default()
        };
        let input = Input {
            repeat_rate: 40,
            numlock: NumLock::On,
            pointer_speed: -35,
            mouse_acceleration: false,
            natural_scroll_touchpad: true,
            ..Input::default()
        };
        let x = rc_xml(&windows, &input);
        for expected in [
            MARK,
            "<followMouse>yes</followMouse>",
            "<gap>0</gap>",
            "<desktops number=\"4\" />",
            "<adaptiveSync>no</adaptiveSync>",
            "<allowTearing>fullscreen</allowTearing>",
            "<numlock>on</numlock>",
            "<repeatRate>40</repeatRate>",
            "<pointerSpeed>-0.35</pointerSpeed>",
            "<accelProfile>flat</accelProfile>",
        ] {
            assert!(x.contains(expected), "{expected}");
        }
        // The touchpad has natural scrolling, the mouse does not.
        let pad = x.split("category=\"touchpad\"").nth(1).unwrap();
        assert!(pad.contains("<naturalScroll>yes</naturalScroll>"));
        let mouse = x.split("category=\"non-touch\"").nth(1).unwrap();
        assert!(mouse.starts_with(">\n      <naturalScroll>no</naturalScroll>"));
    }

    #[test]
    fn environment_only_for_changed_settings() {
        assert_eq!(environment(&Input::default()), None);
        let one = Input {
            keyboard_layout: "fr".into(),
            ..Input::default()
        };
        let text = environment(&one).unwrap();
        assert!(text.contains("XKB_DEFAULT_LAYOUT=fr\n"));
        assert!(!text.contains("XKB_DEFAULT_OPTIONS"));
        let two = Input {
            keyboard_layout: "us,ara".into(),
            layout_switch: LayoutSwitch::CapsLock,
            cursor_size: 32,
            ..Input::default()
        };
        let text = environment(&two).unwrap();
        assert!(text.contains("XKB_DEFAULT_OPTIONS=grp:caps_toggle\n"));
        assert!(text.contains("XCURSOR_SIZE=32"));
    }

    #[test]
    fn sync_links_defaults_writes_changes_and_keeps_hand_edits() {
        let temp = tempfile::tempdir().unwrap();
        let (dir, system) = (temp.path().join("user"), temp.path().join("system"));
        std::fs::create_dir_all(&system).unwrap();
        let mut config = Config::default();

        let first = sync_into(&config, &dir, &system).unwrap();
        assert!(first.changed);
        assert_eq!(
            std::fs::read_link(dir.join("rc.xml")).unwrap(),
            system.join("rc.xml")
        );
        assert_eq!(
            std::fs::read_link(dir.join("autostart")).unwrap(),
            system.join("autostart")
        );
        assert!(!dir.join("environment").exists());
        // Nothing to do the second time: no needless compositor reload.
        assert_eq!(
            sync_into(&config, &dir, &system).unwrap(),
            Synced::default()
        );

        config.windows.workspaces = 3;
        config.input.keyboard_layout = "de".into();
        assert!(sync_into(&config, &dir, &system).unwrap().changed);
        let rc = std::fs::read_to_string(dir.join("rc.xml")).unwrap();
        assert!(rc.contains("<desktops number=\"3\" />"));
        assert!(
            !std::fs::symlink_metadata(dir.join("rc.xml"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            std::fs::read_to_string(dir.join("environment"))
                .unwrap()
                .contains("=de")
        );

        // Back to the defaults: the link returns, the environment goes.
        let reset = sync_into(&Config::default(), &dir, &system).unwrap();
        assert!(reset.changed);
        assert!(
            std::fs::symlink_metadata(dir.join("rc.xml"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!dir.join("environment").exists());

        // A hand-written rc.xml and autostart are never replaced.
        std::fs::remove_file(dir.join("rc.xml")).unwrap();
        std::fs::write(dir.join("rc.xml"), "<labwc_config/>").unwrap();
        std::fs::remove_file(dir.join("autostart")).unwrap();
        std::fs::write(dir.join("autostart"), "#!/bin/sh\n").unwrap();
        let kept = sync_into(&config, &dir, &system).unwrap();
        assert_eq!(kept.kept, [dir.join("rc.xml")]);
        assert_eq!(
            std::fs::read_to_string(dir.join("rc.xml")).unwrap(),
            "<labwc_config/>"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("autostart")).unwrap(),
            "#!/bin/sh\n"
        );
    }
}
