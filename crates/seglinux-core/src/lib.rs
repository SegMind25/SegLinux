//! Shared building blocks for the SegLinux tools.
//!
//! * [`cmd`] describes external commands so they can be logged, shown in a
//!   dry run, or executed.
//! * [`xdg`] resolves the XDG base directories SegLinux keeps state in.
//! * [`init`] knows the init systems Artix ships and how to enable services
//!   in an offline root, which is what the installer needs.

pub mod cmd;
pub mod init;
pub mod xdg;

/// Name shown in user-facing messages.
pub const PRODUCT_NAME: &str = "SegLinux";

/// Path checks against a root directory (`/` on a running system, the mount
/// point of the new system during installation).
pub fn in_root(root: &std::path::Path, path: &str) -> std::path::PathBuf {
    root.join(path.trim_start_matches('/'))
}

/// Find an executable in `PATH`.
pub fn which(program: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if program.contains('/') {
        let p = std::path::PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn in_root_strips_leading_slash() {
        assert_eq!(
            in_root(Path::new("/mnt/target"), "/etc/fstab"),
            Path::new("/mnt/target/etc/fstab")
        );
        assert_eq!(
            in_root(Path::new("/"), "etc/hostname"),
            Path::new("/etc/hostname")
        );
    }

    #[test]
    fn which_finds_sh() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-program-seglinux").is_none());
    }
}
