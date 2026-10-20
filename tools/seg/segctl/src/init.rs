//! Which init system is running. Artix offers four; `seg svc` speaks all of
//! them.

use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Init {
    OpenRc,
    Runit,
    Dinit,
    S6,
}

impl Init {
    pub fn name(self) -> &'static str {
        match self {
            Init::OpenRc => "openrc",
            Init::Runit => "runit",
            Init::Dinit => "dinit",
            Init::S6 => "s6",
        }
    }

    /// The name PID 1 gives itself in /proc/1/comm.
    fn from_pid1(comm: &str) -> Option<Init> {
        match comm.trim() {
            "openrc-init" => Some(Init::OpenRc),
            "runit" | "runit-init" => Some(Init::Runit),
            "dinit" => Some(Init::Dinit),
            "s6-svscan" | "s6-linux-init" | "s6-linux-init-maker" => Some(Init::S6),
            _ => None,
        }
    }
}

impl fmt::Display for Init {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Detect the init system of the system under `root` (`/` for the running
/// one). In order of certainty:
///
/// 1. what PID 1 is (`pid1_comm`, the contents of /proc/1/comm);
/// 2. state the running init leaves in /run;
/// 3. what is installed, for an offline root or a container whose PID 1 is
///    something else.
pub fn detect(root: &Path, pid1_comm: Option<&str>) -> Option<Init> {
    if let Some(init) = pid1_comm.and_then(Init::from_pid1) {
        return Some(init);
    }
    let has = |rel: &str| root.join(rel).exists();
    const RUNNING: [(&str, Init); 4] = [
        ("run/openrc", Init::OpenRc),
        ("run/runit", Init::Runit),
        ("run/dinitctl", Init::Dinit),
        ("run/s6-rc", Init::S6),
    ];
    if let Some((_, init)) = RUNNING.iter().find(|(path, _)| has(path)) {
        return Some(*init);
    }
    const INSTALLED: [(&str, Init); 7] = [
        ("usr/bin/openrc-init", Init::OpenRc),
        ("usr/bin/openrc", Init::OpenRc),
        ("usr/bin/dinit", Init::Dinit),
        ("usr/bin/runit", Init::Runit),
        ("usr/bin/runsvdir", Init::Runit),
        ("usr/bin/s6-svscan", Init::S6),
        ("usr/bin/s6-rc", Init::S6),
    ];
    INSTALLED
        .iter()
        .find(|(path, _)| has(path))
        .map(|(_, init)| *init)
}

/// Detect for the running system.
pub fn detect_running() -> Option<Init> {
    let comm = std::fs::read_to_string("/proc/1/comm").ok();
    detect(Path::new("/"), comm.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testutil::TempDir;

    #[test]
    fn pid1_decides_first() {
        let t = TempDir::new();
        // Even if another init's files are around.
        t.put("usr/bin/openrc", "");
        assert_eq!(detect(t.path(), Some("dinit\n")), Some(Init::Dinit));
        assert_eq!(detect(t.path(), Some("runit")), Some(Init::Runit));
        assert_eq!(detect(t.path(), Some("openrc-init")), Some(Init::OpenRc));
        assert_eq!(detect(t.path(), Some("s6-svscan")), Some(Init::S6));
    }

    #[test]
    fn a_generic_pid1_falls_back_to_the_state_in_run() {
        let t = TempDir::new();
        t.put("run/openrc/softlevel", "default");
        assert_eq!(detect(t.path(), Some("init")), Some(Init::OpenRc));
        let t = TempDir::new();
        t.put("run/dinitctl", "");
        assert_eq!(detect(t.path(), None), Some(Init::Dinit));
        let t = TempDir::new();
        t.put("run/runit/stopit", "");
        assert_eq!(detect(t.path(), None), Some(Init::Runit));
    }

    #[test]
    fn installed_binaries_are_the_last_resort() {
        let t = TempDir::new();
        assert_eq!(detect(t.path(), Some("bash")), None);
        t.put("usr/bin/s6-svscan", "");
        assert_eq!(detect(t.path(), Some("bash")), Some(Init::S6));
        t.put("usr/bin/openrc", "");
        // The first match in the list wins: OpenRC before s6.
        assert_eq!(detect(t.path(), None), Some(Init::OpenRc));
    }

    #[test]
    fn running_state_beats_installed_binaries() {
        let t = TempDir::new();
        t.put("usr/bin/openrc", "");
        t.put("run/dinitctl", "");
        assert_eq!(detect(t.path(), None), Some(Init::Dinit));
    }

    #[test]
    fn names() {
        assert_eq!(Init::OpenRc.to_string(), "openrc");
        assert_eq!(Init::S6.name(), "s6");
    }
}
