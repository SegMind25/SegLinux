//! The trash, as Plasma's trash applet shows it and Windows' Recycle Bin on
//! the desktop: whether it is full, opening it, emptying it. It is the
//! freedesktop.org trash (~/.local/share/Trash), where Dolphin, GIO and
//! every desktop put deleted files.
use crate::common;
use gtk::{gio, glib, prelude::*};
use std::path::{Path, PathBuf};

pub fn dir() -> PathBuf {
    glib::user_data_dir().join("Trash")
}

/// How many items the trash `dir` holds.
pub fn count_in(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("files")).map_or(0, |entries| entries.flatten().count())
}

pub fn count() -> usize {
    count_in(&dir())
}

/// Deletes what the trash `dir` holds, for good: its files and what the
/// trash knows of them. Links are removed, never followed.
pub fn empty_in(dir: &Path) -> std::io::Result<()> {
    for part in ["files", "info"] {
        let Ok(entries) = std::fs::read_dir(dir.join(part)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
    }
    match std::fs::remove_file(dir.join("directorysizes")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Shows the trash: in Dolphin (which can restore files), else the folder.
pub fn open() {
    if common::installed("dolphin") {
        common::run("dolphin", &["trash:/".into()]);
    } else {
        let files = dir().join("files");
        let _ = std::fs::create_dir_all(&files);
        common::open_path(&files);
    }
}

/// Moves a file or folder to the trash (GIO, as file managers do).
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    gio::File::for_path(path)
        .trash(gio::Cancellable::NONE)
        .map_err(|e| format!("Cannot move {} to the trash: {e}", path.display()))
}

/// The trash's icon: full or empty.
pub fn icon_name(count: usize) -> &'static str {
    if count > 0 {
        "user-trash-full"
    } else {
        "user-trash"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counting_and_emptying() {
        let root = std::env::temp_dir().join(format!("seglinux-trash-{}", std::process::id()));
        let trash = root.join("Trash");
        std::fs::create_dir_all(trash.join("files/folder/inner")).unwrap();
        std::fs::create_dir_all(trash.join("info")).unwrap();
        std::fs::write(trash.join("files/notes.txt"), "x").unwrap();
        std::fs::write(trash.join("files/folder/inner/deep.txt"), "x").unwrap();
        std::fs::write(trash.join("info/notes.txt.trashinfo"), "[Trash Info]").unwrap();
        std::fs::write(trash.join("directorysizes"), "").unwrap();
        // A link in the trash to something outside: the link goes, not its
        // target.
        let outside = root.join("keep.txt");
        std::fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, trash.join("files/link")).unwrap();
        assert_eq!(count_in(&trash), 3);
        assert_eq!(icon_name(count_in(&trash)), "user-trash-full");

        empty_in(&trash).unwrap();
        assert_eq!(count_in(&trash), 0);
        assert!(trash.join("files").is_dir(), "the trash itself stays");
        assert_eq!(std::fs::read_dir(trash.join("info")).unwrap().count(), 0);
        assert!(!trash.join("directorysizes").exists());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
        assert_eq!(icon_name(0), "user-trash");
        // An empty or missing trash empties without error.
        empty_in(&trash).unwrap();
        empty_in(&root.join("none")).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }
}
