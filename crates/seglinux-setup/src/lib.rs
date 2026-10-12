//! SegLinux Setup.
//!
//! Installs SegLinux the way Windows Setup installs Windows: it reads an
//! optional answer file (`autounattend.toml`), partitions the disk, applies
//! the system image from the install media (`LiveOS/rootfs.img`, the
//! counterpart of `sources\install.wim`), then configures the new system
//! in passes named after Windows' (windowsPE, offlineServicing,
//! specialize, oobeSystem) and finishes with SetupComplete scripts.

pub mod answer;
pub mod disk;
pub mod exec;
pub mod facts;
pub mod ids;
pub mod plan;
#[cfg(feature = "tui")]
pub mod tui;

/// Effective user ID from /proc (avoids a libc dependency).
pub fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(|euid| euid == "0"))
        })
        .unwrap_or(false)
}
