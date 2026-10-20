//! `seg game apply`, run at boot by the `seg-game` service of every init:
//! the gaming sysctls, transparent hugepages on `madvise` and the permissions
//! of /dev/ntsync. OpenRC and udev do the sysctls and the device too; runit,
//! dinit and s6 images may not, and applying twice is harmless.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::error::{Context, Result};
use crate::sys;
use crate::tune;

pub const HELP: &str = "\
Usage: seg game apply

Applies /etc/sysctl.d/80-seglinux-gaming.conf now (vm.max_map_count,
kernel.split_lock_mitigate, vm.compaction_proactiveness, ...), changes
transparent hugepages from `always` to `madvise` (linux-zen starts with
`always`, which can stall a game on memory compaction; `never` is left as it
is) and makes /dev/ntsync usable by every user, so Wine and Proton can use
NTSYNC. Needs root. The seg-game service runs it at boot.
";

pub const SYSCTL_FILE: &str = "etc/sysctl.d/80-seglinux-gaming.conf";
const THP_FILE: &str = "sys/kernel/mm/transparent_hugepage/enabled";

/// The word in brackets of `always [madvise] never`.
fn thp_selected(text: &str) -> Option<&str> {
    text.split_once('[')?
        .1
        .split_once(']')
        .map(|(word, _)| word)
}

/// Apply the sysctls of the gaming file below `root`; then fix /dev/ntsync.
pub fn apply(root: &Path) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let path = root.join(SYSCTL_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let report = tune::Fs::new(root).apply_sysctl_text(&text)?;
            lines.extend(report.lines);
            lines.extend(report.warnings.into_iter().map(|w| format!("note: {w}")));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            lines.push(format!(
                "note: {} does not exist; no sysctls to apply",
                path.display()
            ));
        }
        Err(e) => {
            return Err(crate::error::Error::io(
                format!("reading {}", path.display()),
                e,
            ));
        }
    }
    let thp = root.join(THP_FILE);
    if let Ok(text) = fs::read_to_string(&thp)
        && thp_selected(&text) == Some("always")
    {
        fs::write(&thp, "madvise").context(format!("writing {}", thp.display()))?;
        lines.push("transparent hugepages: always -> madvise".into());
    }
    let ntsync = root.join("dev/ntsync");
    if let Ok(meta) = fs::metadata(&ntsync) {
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o006 != 0o006 {
            fs::set_permissions(&ntsync, fs::Permissions::from_mode(0o666))
                .context(format!("opening up {}", ntsync.display()))?;
            lines.push(format!("/dev/ntsync {mode:o} -> 666"));
        }
    }
    Ok(lines)
}

pub fn run() -> Result<u8> {
    sys::require_root("`seg game apply`")?;
    let lines = apply(Path::new("/"))?;
    if lines.is_empty() {
        outln!("seg game apply: already applied");
    }
    for l in lines {
        outln!("  {l}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    #[test]
    fn the_gaming_file_is_applied_and_ntsync_opened() {
        let t = TempDir::new();
        t.put(SYSCTL_FILE, "# gaming\nvm.max_map_count = 2147483642\nkernel.split_lock_mitigate = 0\nvm.compaction_proactiveness = 0\n\nvm.unknown_key = 1\n");
        t.put("proc/sys/vm/max_map_count", "65530\n");
        t.put("proc/sys/kernel/split_lock_mitigate", "1\n");
        t.put("proc/sys/vm/compaction_proactiveness", "0\n");
        let ntsync = t.put("dev/ntsync", "");
        fs::set_permissions(&ntsync, fs::Permissions::from_mode(0o600)).unwrap();
        let lines = apply(t.path()).unwrap();
        let read = |rel: &str| {
            fs::read_to_string(t.path().join(rel))
                .unwrap()
                .trim()
                .to_string()
        };
        assert_eq!(read("proc/sys/vm/max_map_count"), "2147483642");
        assert_eq!(read("proc/sys/kernel/split_lock_mitigate"), "0");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("vm.max_map_count") && l.contains("65530 -> 2147483642")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("vm.unknown_key") && l.starts_with("note:")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "/dev/ntsync 600 -> 666"),
            "{lines:?}"
        );
        assert_eq!(
            fs::metadata(&ntsync).unwrap().permissions().mode() & 0o777,
            0o666
        );
        // Again: nothing to do (the unknown key is still a note).
        let again = apply(t.path()).unwrap();
        assert!(again.iter().all(|l| l.starts_with("note:")), "{again:?}");
    }

    #[test]
    fn hugepages_go_from_always_to_madvise_and_nothing_else_changes() {
        let t = TempDir::new();
        t.put(THP_FILE, "[always] madvise never\n");
        let lines = apply(t.path()).unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l == "transparent hugepages: always -> madvise"),
            "{lines:?}"
        );
        assert_eq!(
            fs::read_to_string(t.path().join(THP_FILE)).unwrap(),
            "madvise"
        );
        // madvise is left alone, and so is never (someone chose it).
        for already in ["always [madvise] never\n", "always madvise [never]\n"] {
            let t = TempDir::new();
            t.put(THP_FILE, already);
            let lines = apply(t.path()).unwrap();
            assert!(lines.iter().all(|l| !l.contains("hugepages")), "{lines:?}");
            assert_eq!(
                fs::read_to_string(t.path().join(THP_FILE)).unwrap(),
                already
            );
        }
    }

    #[test]
    fn the_hugepage_setting_is_read_from_the_brackets() {
        assert_eq!(thp_selected("[always] madvise never"), Some("always"));
        assert_eq!(thp_selected("always [madvise] never"), Some("madvise"));
        assert_eq!(thp_selected("always madvise never"), None);
        assert_eq!(thp_selected(""), None);
    }

    #[test]
    fn no_file_and_no_device_is_not_an_error() {
        let t = TempDir::new();
        let lines = apply(t.path()).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("no sysctls to apply"), "{lines:?}");
    }
}
