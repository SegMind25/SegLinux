//! Counts and progress on dock icons, as Plasma's task manager shows them.
//! Applications send them with the Unity launcher API
//! (`com.canonical.Unity.LauncherEntry`): unread messages from Telegram,
//! Discord or Thunderbird, downloads in Brave and Chromium, file copies in
//! KDE applications.
use gtk::{gio, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// What an application asked to show on its icon, as last sent.
#[derive(Clone, Debug, Default, PartialEq)]
struct Entry {
    count: i64,
    count_visible: bool,
    progress: f64,
    progress_visible: bool,
    urgent: bool,
    /// The connection that sent it: when it closes (the application quit or
    /// crashed), the badge goes.
    sender: String,
}

/// What the dock shows on an icon.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Badge {
    pub count: Option<i64>,
    /// 0 to 1.
    pub progress: Option<f64>,
    pub urgent: bool,
}

impl Entry {
    fn badge(&self) -> Badge {
        Badge {
            count: (self.count_visible && self.count > 0).then_some(self.count),
            progress: self
                .progress_visible
                .then_some(self.progress.clamp(0.0, 1.0)),
            urgent: self.urgent,
        }
    }

    /// Applies an update: applications send only what changed.
    fn apply(&mut self, properties: &HashMap<String, glib::Variant>) {
        for (key, value) in properties {
            match key.as_str() {
                "count" => {
                    if let Some(count) = value
                        .get::<i64>()
                        .or_else(|| value.get::<i32>().map(i64::from))
                    {
                        self.count = count;
                    }
                }
                "count-visible" => self.count_visible = value.get().unwrap_or(false),
                "progress" => {
                    if let Some(progress) = value.get::<f64>() {
                        self.progress = progress;
                    }
                }
                "progress-visible" => self.progress_visible = value.get().unwrap_or(false),
                "urgent" => self.urgent = value.get().unwrap_or(false),
                _ => {}
            }
        }
    }
}

/// The desktop file id an update is about: `application://brave-browser.desktop`
/// gives `brave-browser.desktop`.
pub fn desktop_id(app_uri: &str) -> Option<String> {
    let id = app_uri.strip_prefix("application://")?;
    (!id.is_empty() && !id.contains('/')).then(|| {
        if id.ends_with(".desktop") {
            id.to_string()
        } else {
            format!("{id}.desktop")
        }
    })
}

struct Inner {
    entries: RefCell<HashMap<String, Entry>>,
    listeners: RefCell<Vec<Box<dyn Fn() -> bool>>>,
    subscriptions: RefCell<Vec<gio::SignalSubscription>>,
}

/// The badges of the session; cheap to clone.
#[derive(Clone)]
pub struct Badges(Rc<Inner>);

impl Badges {
    pub fn start() -> Badges {
        let badges = Badges(Rc::new(Inner {
            entries: RefCell::default(),
            listeners: RefCell::default(),
            subscriptions: RefCell::default(),
        }));
        let Ok(connection) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        else {
            return badges;
        };
        let weak = Rc::downgrade(&badges.0);
        let updates = connection.subscribe_to_signal(
            None,
            Some("com.canonical.Unity.LauncherEntry"),
            Some("Update"),
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some(badges) = weak.upgrade().map(Badges) else {
                    return;
                };
                let Some((uri, properties)) = signal
                    .parameters
                    .get::<(String, HashMap<String, glib::Variant>)>()
                else {
                    return;
                };
                if let Some(id) = desktop_id(&uri) {
                    badges.update(&id, signal.sender_name, &properties);
                }
            },
        );
        let weak = Rc::downgrade(&badges.0);
        let gone = connection.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some(badges) = weak.upgrade().map(Badges) else {
                    return;
                };
                if let Some((name, _, new_owner)) =
                    signal.parameters.get::<(String, String, String)>()
                    && new_owner.is_empty()
                {
                    badges.forget(&name);
                }
            },
        );
        badges.0.subscriptions.replace(vec![updates, gone]);
        badges
    }

    fn update(&self, id: &str, sender: &str, properties: &HashMap<String, glib::Variant>) {
        {
            let mut entries = self.0.entries.borrow_mut();
            let entry = entries.entry(id.to_string()).or_default();
            entry.apply(properties);
            entry.sender = sender.to_string();
        }
        self.changed();
    }

    /// Removes what a connection that closed had sent.
    fn forget(&self, sender: &str) {
        let removed = {
            let mut entries = self.0.entries.borrow_mut();
            let before = entries.len();
            entries.retain(|_, e| e.sender != sender);
            entries.len() != before
        };
        if removed {
            self.changed();
        }
    }

    /// The badge of a desktop file id (`brave-browser.desktop`).
    pub fn get(&self, id: &str) -> Badge {
        self.0
            .entries
            .borrow()
            .get(id)
            .map(Entry::badge)
            .unwrap_or_default()
    }

    /// Calls `listener` after each change, until it returns false (its
    /// widgets are gone).
    pub fn subscribe(&self, listener: impl Fn() -> bool + 'static) {
        self.0.listeners.borrow_mut().push(Box::new(listener));
    }

    fn changed(&self) {
        let listeners: Vec<_> = self.0.listeners.borrow_mut().drain(..).collect();
        let kept: Vec<_> = listeners.into_iter().filter(|l| l()).collect();
        let mut current = self.0.listeners.borrow_mut();
        let added: Vec<_> = current.drain(..).collect();
        current.extend(kept);
        current.extend(added);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::prelude::*;

    fn props(pairs: &[(&str, glib::Variant)]) -> HashMap<String, glib::Variant> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn app_uris() {
        assert_eq!(
            desktop_id("application://brave-browser.desktop").as_deref(),
            Some("brave-browser.desktop")
        );
        assert_eq!(
            desktop_id("application://org.telegram.desktop.desktop").as_deref(),
            Some("org.telegram.desktop.desktop")
        );
        assert_eq!(
            desktop_id("application://discord").as_deref(),
            Some("discord.desktop")
        );
        assert_eq!(desktop_id("file:///tmp/x.desktop"), None);
        assert_eq!(desktop_id("application://"), None);
    }

    #[test]
    fn updates_build_up() {
        let mut entry = Entry::default();
        entry.apply(&props(&[
            ("count", 3i64.to_variant()),
            ("count-visible", true.to_variant()),
        ]));
        assert_eq!(entry.badge().count, Some(3));
        // Progress alone keeps the count.
        entry.apply(&props(&[
            ("progress", 0.25f64.to_variant()),
            ("progress-visible", true.to_variant()),
        ]));
        assert_eq!(
            entry.badge(),
            Badge {
                count: Some(3),
                progress: Some(0.25),
                urgent: false
            }
        );
        entry.apply(&props(&[
            ("count-visible", false.to_variant()),
            ("progress", 7.0f64.to_variant()),
        ]));
        assert_eq!(entry.badge().count, None);
        assert_eq!(entry.badge().progress, Some(1.0), "clamped");
        entry.apply(&props(&[
            ("count", 0i64.to_variant()),
            ("count-visible", true.to_variant()),
        ]));
        assert_eq!(entry.badge().count, None, "no badge for zero");
        entry.apply(&props(&[("urgent", true.to_variant())]));
        assert!(entry.badge().urgent);
    }
}
