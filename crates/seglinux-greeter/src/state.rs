//! What the sign-in screen preselects: the account and session used last.
//! Setup writes the first one (the account it created, the SegLinux
//! desktop); the greeter updates it at each sign-in.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const PATH: &str = "/var/lib/seglinux-greeter/state.toml";

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct State {
    pub user: String,
    pub session: String,
}

impl State {
    pub fn load(path: &Path) -> State {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Saves it when the folder can be written to; it is only a preference.
    pub fn save(&self, path: &Path) {
        if let Ok(text) = toml::to_string(self) {
            let partial = path.with_extension("toml.new");
            if std::fs::write(&partial, text).is_ok() {
                let _ = std::fs::rename(&partial, path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        assert_eq!(State::load(&path), State::default());
        let state = State {
            user: "alice".into(),
            session: "seglinux".into(),
        };
        state.save(&path);
        assert_eq!(State::load(&path), state);
        // What Setup writes.
        std::fs::write(&path, "user = \"bob\"\nsession = \"plasma\"\n").unwrap();
        assert_eq!(State::load(&path).user, "bob");
        std::fs::write(&path, "not toml [").unwrap();
        assert_eq!(State::load(&path), State::default());
        // A folder that cannot be written to is not an error.
        state.save(&dir.path().join("missing/state.toml"));
    }
}
