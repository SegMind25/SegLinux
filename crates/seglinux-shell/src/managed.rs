//! Configuration files of other programs (labwc, MangoHud) that SegLinux
//! Settings writes. Such files carry a mark in their first lines; a file
//! without it was written by hand and is never replaced.
use std::io;
use std::path::{Path, PathBuf};

pub const MARK: &str = "Managed by SegLinux Settings";

/// What a configuration file should be.
#[derive(Debug, PartialEq, Eq)]
pub enum Wanted {
    /// A link to the distribution's file.
    Link(PathBuf),
    /// Our own content.
    Text(String),
    /// No file (the program's defaults).
    Absent,
}

/// Whether we may replace or remove `path`: it is missing, a link, or a
/// file carrying the mark.
pub fn ours(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Err(_) => true,
        Ok(meta) if meta.file_type().is_symlink() => true,
        Ok(_) => std::fs::read_to_string(path)
            .map(|text| text.lines().take(3).any(|line| line.contains(MARK)))
            .unwrap_or(false),
    }
}

pub fn is_current(path: &Path, wanted: &Wanted) -> bool {
    let meta = std::fs::symlink_metadata(path);
    match wanted {
        Wanted::Absent => meta.is_err(),
        Wanted::Link(target) => {
            meta.is_ok_and(|m| m.file_type().is_symlink())
                && std::fs::read_link(path).is_ok_and(|t| &t == target)
        }
        Wanted::Text(text) => {
            meta.is_ok_and(|m| m.is_file())
                && std::fs::read_to_string(path).is_ok_and(|t| &t == text)
        }
    }
}

/// Replaces `path` atomically: a new file or link next to it, renamed over it.
pub fn put(path: &Path, wanted: &Wanted) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("seglinux-new");
    let _ = std::fs::remove_file(&temp);
    match wanted {
        Wanted::Absent => {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }
        Wanted::Link(target) => std::os::unix::fs::symlink(target, &temp)?,
        Wanted::Text(text) => std::fs::write(&temp, text)?,
    }
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// What happened to one file.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Unchanged,
    Written,
    /// Edited by hand: left as it is.
    Kept,
}

/// Makes `path` what is wanted, unless it was edited by hand.
pub fn update(path: &Path, wanted: &Wanted) -> io::Result<Outcome> {
    if is_current(path, wanted) {
        Ok(Outcome::Unchanged)
    } else if ours(path) {
        put(path, wanted).map(|()| Outcome::Written)
    } else {
        Ok(Outcome::Kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_ours_and_keeps_hand_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/app.conf");
        let text = Wanted::Text(format!("# {MARK}\na=1\n"));
        assert_eq!(update(&path, &text).unwrap(), Outcome::Written);
        assert_eq!(update(&path, &text).unwrap(), Outcome::Unchanged);
        assert_eq!(update(&path, &Wanted::Absent).unwrap(), Outcome::Written);
        assert!(!path.exists());
        std::fs::write(&path, "a=2\n").unwrap();
        assert_eq!(update(&path, &text).unwrap(), Outcome::Kept);
        assert_eq!(update(&path, &Wanted::Absent).unwrap(), Outcome::Kept);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a=2\n");
        assert!(!dir.path().join("sub/app.seglinux-new").exists());
    }
}
