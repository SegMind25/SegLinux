//! Matching windows to applications, so the dock shows one icon per app
//! whether it is pinned, running, or both.

use gtk::{gio, prelude::*};

/// Windows whose app id stands for the pinned id on the left: terminals
/// for the `wincraft-terminal` launcher.
const ALIASES: &[(&str, &[&str])] = &[(
    "wincraft-terminal",
    &[
        "foot",
        "footclient",
        "org.kde.konsole",
        "konsole",
        "alacritty",
        "xterm",
    ],
)];

/// Desktop file ids (without `.desktop`) to try for a window's app id,
/// most specific first: `org.kde.dolphin`, then `dolphin`.
pub fn candidates(app_id: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    push(app_id.to_string());
    push(app_id.to_lowercase());
    if let Some(last) = app_id.rsplit('.').next() {
        push(last.to_lowercase());
    }
    out
}

/// Whether a window with `app_id` belongs to the pinned launcher `pin`.
pub fn belongs_to(pin: &str, app_id: &str) -> bool {
    if app_id.is_empty() {
        return false;
    }
    if ALIASES.iter().any(|(p, ids)| {
        p.eq_ignore_ascii_case(pin) && ids.iter().any(|i| i.eq_ignore_ascii_case(app_id))
    }) {
        return true;
    }
    let pin_names = candidates(pin);
    candidates(app_id)
        .iter()
        .any(|c| pin_names.iter().any(|p| p.eq_ignore_ascii_case(c)))
}

/// The installed application for a desktop file id, if any.
pub fn lookup(id: &str) -> Option<gio::DesktopAppInfo> {
    gio::DesktopAppInfo::new(&format!("{id}.desktop"))
}

/// The installed application a window belongs to, if one can be found.
pub fn for_window(app_id: &str) -> Option<gio::DesktopAppInfo> {
    candidates(app_id).iter().find_map(|c| lookup(c))
}

/// An icon for an application, or a generic one.
pub fn icon(app: Option<&gio::DesktopAppInfo>) -> gio::Icon {
    app.and_then(|a| a.icon())
        .unwrap_or_else(|| gio::ThemedIcon::new("application-x-executable").upcast())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_ids() {
        assert_eq!(
            candidates("org.kde.dolphin"),
            ["org.kde.dolphin", "dolphin"]
        );
        assert_eq!(candidates("Steam"), ["Steam", "steam"]);
        assert_eq!(candidates("firefox"), ["firefox"]);
        assert!(candidates("").is_empty());
    }

    #[test]
    fn windows_belong_to_pins() {
        assert!(belongs_to("org.kde.dolphin", "org.kde.dolphin"));
        assert!(belongs_to("firefox", "firefox"));
        assert!(belongs_to("steam", "Steam"));
        assert!(belongs_to("net.lutris.Lutris", "net.lutris.Lutris"));
        assert!(belongs_to("wincraft-terminal", "foot"));
        assert!(belongs_to("wincraft-terminal", "org.kde.konsole"));
        assert!(!belongs_to("firefox", "org.kde.dolphin"));
        assert!(!belongs_to("steam", ""));
    }
}
