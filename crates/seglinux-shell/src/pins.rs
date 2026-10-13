//! Pinning applications to the dock from the dock and the Start menu, as
//! with Plasma's "Pin to Task Manager". The pins are shell.toml's `pinned`
//! list, which the running desktop reloads as soon as it is saved.
use crate::common;
use crate::config::Config;
use gtk::{gio, prelude::*};

/// What a menu item or a drag does to the pinned list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Pin,
    Unpin,
    /// Moves the pin to where the pin named here is; that one moves aside,
    /// as when an icon is dropped onto another.
    TakePlaceOf(String),
}

/// The desktop file id of an application: `firefox` for firefox.desktop.
pub fn desktop_id(app: &impl IsA<gio::AppInfo>) -> Option<String> {
    app.id()
        .map(|id| id.trim_end_matches(".desktop").to_string())
        .filter(|id| !id.is_empty())
}

/// Applies `change` to `pinned` for the application `id`; returns whether
/// the list changed.
pub fn apply(pinned: &mut Vec<String>, id: &str, change: &Change) -> bool {
    let from = pinned.iter().position(|p| p == id);
    match (change, from) {
        (Change::Pin, None) => {
            pinned.push(id.to_string());
            true
        }
        (Change::Unpin, Some(from)) => {
            pinned.remove(from);
            true
        }
        (Change::TakePlaceOf(other), Some(from)) => match pinned.iter().position(|p| p == other) {
            Some(to) if to != from => {
                let moved = pinned.remove(from);
                pinned.insert(to, moved);
                true
            }
            _ => false,
        },
        _ => false,
    }
}

/// Saves `change` for the application `id`; the desktop picks it up.
pub fn update(id: &str, change: Change) {
    if let Err(error) = save(id, &change) {
        common::show_error(&error);
    }
}

fn save(id: &str, change: &Change) -> Result<(), String> {
    let path = Config::path();
    // The file rather than the desktop's copy: preferences saved meanwhile
    // are kept, and a file being edited by hand is left alone.
    let mut config = Config::try_load().map_err(|error| {
        format!(
            "{} cannot be read, so the dock was not changed: {error}",
            path.display()
        )
    })?;
    if !apply(&mut config.pinned, id, change) {
        return Ok(());
    }
    config.validate()?;
    crate::settings::save_text(&path, &config.to_toml())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pin_and_unpin() {
        let mut pinned = list(&["firefox"]);
        assert!(apply(&mut pinned, "steam", &Change::Pin));
        assert_eq!(pinned, ["firefox", "steam"]);
        assert!(!apply(&mut pinned, "steam", &Change::Pin), "no duplicates");
        assert!(apply(&mut pinned, "firefox", &Change::Unpin));
        assert_eq!(pinned, ["steam"]);
        assert!(!apply(&mut pinned, "firefox", &Change::Unpin));
    }

    #[test]
    fn moving_takes_the_other_pins_place() {
        let mut pinned = list(&["a", "b", "c", "d"]);
        // Dragged to the right: lands after the one it was dropped on.
        assert!(apply(&mut pinned, "a", &Change::TakePlaceOf("c".into())));
        assert_eq!(pinned, ["b", "c", "a", "d"]);
        // Dragged to the left: lands before it.
        assert!(apply(&mut pinned, "d", &Change::TakePlaceOf("b".into())));
        assert_eq!(pinned, ["d", "b", "c", "a"]);
        // Onto itself, or with something that is not pinned: nothing moves.
        assert!(!apply(&mut pinned, "b", &Change::TakePlaceOf("b".into())));
        assert!(!apply(&mut pinned, "b", &Change::TakePlaceOf("x".into())));
        assert!(!apply(&mut pinned, "x", &Change::TakePlaceOf("b".into())));
        assert_eq!(pinned, ["d", "b", "c", "a"]);
    }

    #[test]
    fn moving_past_pins_that_are_not_installed() {
        // The dock hides "gone"; moving "b" left past "a" skips it.
        let mut pinned = list(&["a", "gone", "b"]);
        assert!(apply(&mut pinned, "b", &Change::TakePlaceOf("a".into())));
        assert_eq!(pinned, ["b", "a", "gone"]);
    }
}
