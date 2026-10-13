//! What the Start menu knows about the installed applications: their
//! categories, grouped as in Plasma's Kickoff, and which ones are new,
//! installed after the menu first listed the applications and not opened
//! since.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The Start menu's sidebar, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    /// The applications pinned to the dock.
    Favorites,
    All,
    Games,
    Internet,
    Multimedia,
    Graphics,
    Office,
    Development,
    Education,
    Science,
    Settings,
    System,
    Utilities,
    /// Applications that name no main category.
    Other,
}

impl Category {
    pub const ALL: [Category; 14] = [
        Category::Favorites,
        Category::All,
        Category::Games,
        Category::Internet,
        Category::Multimedia,
        Category::Graphics,
        Category::Office,
        Category::Development,
        Category::Education,
        Category::Science,
        Category::Settings,
        Category::System,
        Category::Utilities,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Favorites => "Favorites",
            Category::All => "All applications",
            Category::Games => "Games",
            Category::Internet => "Internet",
            Category::Multimedia => "Multimedia",
            Category::Graphics => "Graphics",
            Category::Office => "Office",
            Category::Development => "Development",
            Category::Education => "Education",
            Category::Science => "Science",
            Category::Settings => "Settings",
            Category::System => "System",
            Category::Utilities => "Utilities",
            Category::Other => "Other",
        }
    }

    /// Icon names to try, most specific first (see common::themed_icon).
    pub fn icons(self) -> &'static [&'static str] {
        match self {
            Category::Favorites => &["bookmarks", "starred-symbolic"],
            Category::All => &["applications-all", "view-app-grid-symbolic"],
            Category::Games => &["applications-games", "applications-games-symbolic"],
            Category::Internet => &["applications-internet", "network-workgroup-symbolic"],
            Category::Multimedia => &[
                "applications-multimedia",
                "applications-multimedia-symbolic",
            ],
            Category::Graphics => &["applications-graphics", "applications-graphics-symbolic"],
            Category::Office => &["applications-office", "x-office-document-symbolic"],
            Category::Development => &[
                "applications-development",
                "applications-engineering-symbolic",
            ],
            Category::Education => &["applications-education", "applications-science-symbolic"],
            Category::Science => &["applications-science", "applications-science-symbolic"],
            Category::Settings => &["preferences-system", "preferences-system-symbolic"],
            Category::System => &["applications-system", "applications-system-symbolic"],
            Category::Utilities => &["applications-utilities", "applications-utilities-symbolic"],
            Category::Other => &["applications-other", "applications-other-symbolic"],
        }
    }
}

/// The categories of a desktop file's `Categories` list
/// (`Game;ActionGame;`), from the freedesktop.org main categories.
pub fn categories(list: &str) -> Vec<Category> {
    let mut out = Vec::new();
    for name in list.split(';').map(str::trim) {
        let category = match name {
            "Game" => Category::Games,
            "Network" => Category::Internet,
            "AudioVideo" | "Audio" | "Video" => Category::Multimedia,
            "Graphics" => Category::Graphics,
            "Office" => Category::Office,
            "Development" => Category::Development,
            "Education" => Category::Education,
            "Science" => Category::Science,
            "Settings" => Category::Settings,
            "System" => Category::System,
            "Utility" => Category::Utilities,
            _ => continue,
        };
        if !out.contains(&category) {
            out.push(category);
        }
    }
    if out.is_empty() {
        out.push(Category::Other);
    }
    out
}

/// The applications the Start menu has seen, kept in
/// ~/.local/state/seglinux/known-apps (one desktop file id per line).
pub struct Known {
    path: PathBuf,
    ids: BTreeSet<String>,
    /// No list yet: everything installed now counts as known.
    first_run: bool,
}

impl Known {
    pub fn default_path() -> PathBuf {
        seglinux_core::xdg::state_home()
            .join("seglinux")
            .join("known-apps")
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self {
                path: path.to_path_buf(),
                ids: text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect(),
                first_run: false,
            },
            Err(_) => Self {
                path: path.to_path_buf(),
                ids: BTreeSet::new(),
                first_run: true,
            },
        }
    }

    /// Notes the installed applications; on the first run they all become
    /// known, so a new account starts without "New" marks.
    pub fn note_installed<'a>(&mut self, installed: impl IntoIterator<Item = &'a str>) {
        if self.first_run {
            self.ids.extend(installed.into_iter().map(String::from));
            self.first_run = false;
            self.save();
        }
    }

    /// Installed after the list was started, and not opened since.
    pub fn is_new(&self, id: &str) -> bool {
        !self.first_run && !self.ids.contains(id)
    }

    /// The application was opened: it is no longer new.
    pub fn opened(&mut self, id: &str) {
        if self.ids.insert(id.to_string()) {
            self.save();
        }
    }

    fn save(&self) {
        let mut text = String::new();
        for id in &self.ids {
            text.push_str(id);
            text.push('\n');
        }
        let saved = self
            .path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&self.path, text));
        if let Err(error) = saved {
            // Only the "New" marks depend on it.
            eprintln!("{}: {error}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_categories() {
        assert_eq!(categories("Game;ActionGame;"), [Category::Games]);
        assert_eq!(categories("Network;WebBrowser;"), [Category::Internet]);
        assert_eq!(
            categories("AudioVideo;Audio;Player;"),
            [Category::Multimedia]
        );
        assert_eq!(
            categories("Qt;KDE;System;FileManager;Utility;"),
            [Category::System, Category::Utilities]
        );
        assert_eq!(categories(""), [Category::Other]);
        assert_eq!(categories("X-Custom;"), [Category::Other]);
    }

    #[test]
    fn every_category_has_a_label_and_icons() {
        for category in Category::ALL {
            assert!(!category.label().is_empty());
            assert!(!category.icons().is_empty());
        }
    }

    #[test]
    fn new_applications_until_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/known-apps");
        let mut known = Known::load(&path);
        // The first list: nothing is new.
        assert!(!known.is_new("firefox"));
        known.note_installed(["firefox", "steam"]);
        assert!(path.is_file());

        // Later sessions: what was installed since is new until opened.
        let mut known = Known::load(&path);
        known.note_installed(["firefox", "steam", "gimp"]);
        assert!(!known.is_new("firefox"));
        assert!(known.is_new("gimp"));
        known.opened("gimp");
        assert!(!known.is_new("gimp"));
        assert!(!Known::load(&path).is_new("gimp"), "saved");
    }
}
