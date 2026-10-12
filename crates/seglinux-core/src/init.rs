//! Artix init systems and offline service management.
//!
//! Artix lets the user choose the init system (it is what makes Artix
//! "Arch without systemd"). SegLinux supports enabling services for OpenRC,
//! dinit and runit inside a mounted root without booting it, the way
//! `rc-update`/`dinitctl enable` would.

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::str::FromStr;

use crate::in_root;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitSystem {
    OpenRc,
    Dinit,
    Runit,
    S6,
}

impl InitSystem {
    pub const ALL: [InitSystem; 4] = [
        InitSystem::OpenRc,
        InitSystem::Dinit,
        InitSystem::Runit,
        InitSystem::S6,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            InitSystem::OpenRc => "openrc",
            InitSystem::Dinit => "dinit",
            InitSystem::Runit => "runit",
            InitSystem::S6 => "s6",
        }
    }

    /// Detect the init system installed in `root`.
    pub fn detect(root: &Path) -> Option<InitSystem> {
        for init in ["usr/bin/init", "sbin/init", "usr/sbin/init"] {
            if let Ok(target) = fs::read_link(root.join(init)) {
                let target = target.to_string_lossy();
                if let Some(found) = Self::ALL.into_iter().find(|i| target.contains(i.name())) {
                    return Some(found);
                }
            }
        }
        let has = |p: &str| in_root(root, p).exists();
        if has("usr/bin/openrc") || has("usr/bin/openrc-run") || has("sbin/openrc-run") {
            Some(InitSystem::OpenRc)
        } else if has("usr/bin/dinit") {
            Some(InitSystem::Dinit)
        } else if has("usr/bin/runit") || has("usr/bin/runsvdir") {
            Some(InitSystem::Runit)
        } else if has("usr/bin/s6-svscan") {
            Some(InitSystem::S6)
        } else {
            None
        }
    }

    /// Where the service definition for `name` lives.
    pub fn service_path(&self, name: &str) -> String {
        match self {
            InitSystem::OpenRc => format!("/etc/init.d/{name}"),
            InitSystem::Dinit => format!("/etc/dinit.d/{name}"),
            InitSystem::Runit => format!("/etc/runit/sv/{name}"),
            InitSystem::S6 => format!("/etc/s6/sv/{name}"),
        }
    }

    /// Enable `name` in the system mounted at `root` (offline).
    pub fn enable_service(&self, root: &Path, name: &str) -> io::Result<ServiceChange> {
        if !in_root(root, &self.service_path(name)).exists() {
            return Ok(ServiceChange::Missing);
        }
        let (link, target) = match self {
            InitSystem::OpenRc => (
                format!("/etc/runlevels/{}/{name}", openrc_runlevel(name)),
                format!("/etc/init.d/{name}"),
            ),
            InitSystem::Dinit => (format!("/etc/dinit.d/boot.d/{name}"), format!("../{name}")),
            InitSystem::Runit => (
                format!("/etc/runit/runsvdir/default/{name}"),
                format!("/etc/runit/sv/{name}"),
            ),
            InitSystem::S6 => {
                // s6-rc bundles are compiled databases; adding the service to
                // the "default" bundle source takes effect after the next
                // `s6-db-reload`, which Artix runs from its s6 hooks.
                let path = in_root(root, &format!("/etc/s6/adminsv/default/contents.d/{name}"));
                if path.exists() {
                    return Ok(ServiceChange::AlreadyEnabled);
                }
                fs::create_dir_all(path.parent().expect("has parent"))?;
                fs::write(path, "")?;
                return Ok(ServiceChange::Enabled);
            }
        };
        let link = in_root(root, &link);
        if link.symlink_metadata().is_ok() {
            return Ok(ServiceChange::AlreadyEnabled);
        }
        fs::create_dir_all(link.parent().expect("has parent"))?;
        symlink(target, &link)?;
        Ok(ServiceChange::Enabled)
    }

    /// Disable `name` in the system mounted at `root` (offline).
    pub fn disable_service(&self, root: &Path, name: &str) -> io::Result<ServiceChange> {
        let links: Vec<String> = match self {
            InitSystem::OpenRc => ["boot", "default", "sysinit", "shutdown"]
                .iter()
                .map(|lvl| format!("/etc/runlevels/{lvl}/{name}"))
                .collect(),
            InitSystem::Dinit => vec![format!("/etc/dinit.d/boot.d/{name}")],
            InitSystem::Runit => vec![format!("/etc/runit/runsvdir/default/{name}")],
            InitSystem::S6 => vec![format!("/etc/s6/adminsv/default/contents.d/{name}")],
        };
        let mut changed = false;
        for link in links {
            let path = in_root(root, &link);
            if path.symlink_metadata().is_ok() {
                fs::remove_file(path)?;
                changed = true;
            }
        }
        Ok(if changed {
            ServiceChange::Disabled
        } else {
            ServiceChange::AlreadyDisabled
        })
    }
}

/// OpenRC services that belong in the `boot` runlevel rather than `default`.
pub fn openrc_runlevel(name: &str) -> &'static str {
    const BOOT: &[&str] = &[
        "elogind",
        "udev",
        "udev-trigger",
        "binfmt",
        "sysctl",
        "modules",
        "hwclock",
        "keymaps",
        "hostname",
        "localmount",
        "lvm",
        "dmcrypt",
        "mdraid",
        "haveged",
    ];
    if BOOT.contains(&name) {
        "boot"
    } else {
        "default"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceChange {
    Enabled,
    AlreadyEnabled,
    Disabled,
    AlreadyDisabled,
    /// The service is not installed in the target.
    Missing,
}

impl fmt::Display for InitSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for InitSystem {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|i| i.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| {
                format!("unknown init system `{s}` (expected openrc, dinit, runit or s6)")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, path: &str) {
        let p = in_root(root, path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "").unwrap();
    }

    #[test]
    fn detects_init_from_init_symlink() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("usr/bin")).unwrap();
        symlink("dinit", dir.path().join("usr/bin/init")).unwrap();
        assert_eq!(InitSystem::detect(dir.path()), Some(InitSystem::Dinit));
    }

    #[test]
    fn detects_init_from_binaries() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(InitSystem::detect(dir.path()), None);
        touch(dir.path(), "/usr/bin/openrc-run");
        assert_eq!(InitSystem::detect(dir.path()), Some(InitSystem::OpenRc));
    }

    #[test]
    fn openrc_enable_and_disable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rc = InitSystem::OpenRc;
        assert_eq!(
            rc.enable_service(root, "sddm").unwrap(),
            ServiceChange::Missing
        );

        touch(root, "/etc/init.d/sddm");
        touch(root, "/etc/init.d/elogind");
        assert_eq!(
            rc.enable_service(root, "sddm").unwrap(),
            ServiceChange::Enabled
        );
        assert_eq!(
            rc.enable_service(root, "sddm").unwrap(),
            ServiceChange::AlreadyEnabled
        );
        assert_eq!(
            rc.enable_service(root, "elogind").unwrap(),
            ServiceChange::Enabled
        );
        assert_eq!(
            fs::read_link(root.join("etc/runlevels/default/sddm")).unwrap(),
            Path::new("/etc/init.d/sddm")
        );
        assert!(
            root.join("etc/runlevels/boot/elogind")
                .symlink_metadata()
                .is_ok()
        );

        assert_eq!(
            rc.disable_service(root, "sddm").unwrap(),
            ServiceChange::Disabled
        );
        assert_eq!(
            rc.disable_service(root, "sddm").unwrap(),
            ServiceChange::AlreadyDisabled
        );
    }

    #[test]
    fn dinit_uses_relative_boot_d_links() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "/etc/dinit.d/NetworkManager");
        assert_eq!(
            InitSystem::Dinit
                .enable_service(dir.path(), "NetworkManager")
                .unwrap(),
            ServiceChange::Enabled
        );
        assert_eq!(
            fs::read_link(dir.path().join("etc/dinit.d/boot.d/NetworkManager")).unwrap(),
            Path::new("../NetworkManager")
        );
    }

    #[test]
    fn runit_links_into_default_runsvdir() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("etc/runit/sv/cupsd")).unwrap();
        InitSystem::Runit
            .enable_service(dir.path(), "cupsd")
            .unwrap();
        assert_eq!(
            fs::read_link(dir.path().join("etc/runit/runsvdir/default/cupsd")).unwrap(),
            Path::new("/etc/runit/sv/cupsd")
        );
    }

    #[test]
    fn parse_names() {
        assert_eq!("OpenRC".parse::<InitSystem>().unwrap(), InitSystem::OpenRc);
        assert!("systemd".parse::<InitSystem>().is_err());
    }
}
