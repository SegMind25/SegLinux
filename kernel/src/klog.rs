//! Kernel log: `error!`, `warn!`, `info!`, `debug!` print to the serial
//! port and the screen console with a level prefix.

use core::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
}

static MAX_LEVEL: AtomicU8 = AtomicU8::new(if cfg!(debug_assertions) {
    Level::Debug
} else {
    Level::Info
} as u8);

pub fn set_max_level(level: Level) {
    MAX_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn log(level: Level, args: core::fmt::Arguments) {
    if level as u8 > MAX_LEVEL.load(Ordering::Relaxed) {
        return;
    }
    let tag = match level {
        Level::Error => "error",
        Level::Warn => "warn ",
        Level::Info => "info ",
        Level::Debug => "debug",
    };
    crate::console::print(format_args!("[{tag}] {args}\n"));
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::klog::log($crate::klog::Level::Error, format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::klog::log($crate::klog::Level::Warn, format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::klog::log($crate::klog::Level::Info, format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { $crate::klog::log($crate::klog::Level::Debug, format_args!($($arg)*)) };
}
