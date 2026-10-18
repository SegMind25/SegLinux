//! A stand-in application for testing dock badges: it sends the Unity
//! launcher API's Update signal (as Telegram or Brave do) for a desktop
//! file id, then stays connected; the badge goes when it quits.
//!
//!   cargo run -p seglinux-shell --example badge-test -- brave-browser.desktop 3 0.4
use gtk::{gio, glib, prelude::*};
use std::collections::HashMap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id = args
        .first()
        .cloned()
        .unwrap_or_else(|| "brave-browser.desktop".into());
    let count: i64 = args.get(1).and_then(|c| c.parse().ok()).unwrap_or(3);
    let progress: f64 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(0.4);
    let connection =
        gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).expect("a session bus");
    let mut properties: HashMap<String, glib::Variant> = HashMap::new();
    properties.insert("count".into(), count.to_variant());
    properties.insert("count-visible".into(), (count > 0).to_variant());
    properties.insert("progress".into(), progress.to_variant());
    properties.insert("progress-visible".into(), (progress > 0.0).to_variant());
    connection
        .emit_signal(
            None,
            "/com/canonical/unity/launcherentry/1",
            "com.canonical.Unity.LauncherEntry",
            "Update",
            Some(&(format!("application://{id}"), properties).to_variant()),
        )
        .expect("the signal is sent");
    connection
        .flush_sync(gio::Cancellable::NONE)
        .expect("the signal leaves");
    println!("sent count {count} and progress {progress} for {id}; Ctrl+C removes them");
    glib::MainLoop::new(None, false).run();
}
