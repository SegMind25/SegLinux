//! segrun: SegLinux's Windows program launcher.
//!
//! It plays the role of Explorer's "double-click to run": it looks at the
//! file (see `seglinux-pe`), picks a Wine prefix and runner, prepares the
//! prefix once (Wine environment, DXVK, VKD3D-Proton) and starts the
//! program from its own folder.

pub mod advice;
pub mod binfmt;
pub mod config;
pub mod desktop;
pub mod doctor;
pub mod launch;
pub mod prefix;
pub mod runner;
