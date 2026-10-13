//! The right end of the dock: network, volume, battery, quick settings and
//! the clock with its calendar.
use crate::common::{self, TERMINAL};
use crate::config::Config;
use gtk::{gio, glib, prelude::*};
use std::time::Duration;

pub fn tray(app: &gtk::Application, preview: bool, config: &Config) -> gtk::Box {
    let tray = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    tray.add_css_class("tray");
    tray.append(&network());
    tray.append(&volume());
    if let Some(battery) = battery() {
        tray.append(&battery);
    }
    let settings = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("Quick settings")
        .build();
    settings.set_popover(Some(&quick_settings(app, preview)));
    tray.append(&settings);
    tray.append(&clock(&config.clock_format));
    tray
}

/// Network status from GLib's network monitor; a click opens the
/// connection editor (or nmtui in a terminal on the Lite edition).
fn network() -> gtk::Button {
    let button = gtk::Button::new();
    let image = gtk::Image::new();
    button.set_child(Some(&image));
    let update = {
        let image = image.clone();
        let button = button.clone();
        move |monitor: &gio::NetworkMonitor| {
            let online = monitor.connectivity() == gio::NetworkConnectivity::Full;
            image.set_icon_name(Some(if online {
                "network-transmit-receive-symbolic"
            } else if monitor.is_network_available() {
                "network-idle-symbolic"
            } else {
                "network-offline-symbolic"
            }));
            button.set_tooltip_text(Some(if online {
                "Connected to the internet"
            } else if monitor.is_network_available() {
                "Connected, without internet access"
            } else {
                "Not connected"
            }));
        }
    };
    let monitor = gio::NetworkMonitor::default();
    update(&monitor);
    monitor.connect_network_changed(move |m, _| update(m));
    button.connect_clicked(|_| {
        if common::installed("nm-connection-editor") {
            common::run("nm-connection-editor", &[]);
        } else {
            common::run(TERMINAL, &["nmtui".into()]);
        }
    });
    button
}

/// Volume: a click opens the mixer, scrolling changes the volume.
fn volume() -> gtk::Button {
    let button = gtk::Button::from_icon_name("audio-volume-high-symbolic");
    button.set_tooltip_text(Some("Volume (scroll to change)"));
    button.connect_clicked(|_| {
        if common::installed("pavucontrol") {
            common::run("pavucontrol", &[]);
        }
    });
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(|_, _, dy| {
        if common::installed("wpctl") {
            let step = if dy < 0.0 { "5%+" } else { "5%-" };
            common::run(
                "wpctl",
                &["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", step].map(String::from),
            );
        }
        glib::Propagation::Stop
    });
    button.add_controller(scroll);
    button
}

/// Battery level and state from /sys, for laptops.
fn read_battery() -> Option<(u32, bool)> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" {
            continue;
        }
        let capacity = std::fs::read_to_string(path.join("capacity")).ok()?;
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        return Some((
            capacity.trim().parse().ok()?,
            matches!(status.trim(), "Charging" | "Full"),
        ));
    }
    None
}

/// The icon name for a battery level, in Adwaita's 10% steps.
pub fn battery_icon(level: u32, charging: bool) -> String {
    let step = (level.min(100) + 5) / 10 * 10;
    if charging {
        format!("battery-level-{step}-charging-symbolic")
    } else {
        format!("battery-level-{step}-symbolic")
    }
}

fn battery() -> Option<gtk::Box> {
    read_battery()?;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    row.add_css_class("battery");
    let image = gtk::Image::new();
    let label = gtk::Label::new(None);
    row.append(&image);
    row.append(&label);
    let update = move |image: &gtk::Image, label: &gtk::Label| {
        if let Some((level, charging)) = read_battery() {
            image.set_icon_name(Some(&common::themed_icon(&[
                &battery_icon(level, charging),
                "battery-symbolic",
            ])));
            label.set_text(&format!("{level}%"));
        }
    };
    update(&image, &label);
    let (image, label) = (image.downgrade(), label.downgrade());
    glib::timeout_add_seconds_local(30, move || match (image.upgrade(), label.upgrade()) {
        (Some(image), Some(label)) => {
            update(&image, &label);
            glib::ControlFlow::Continue
        }
        _ => glib::ControlFlow::Break,
    });
    Some(row)
}

fn quick_settings(app: &gtk::Application, preview: bool) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_size_request(300, -1);
    let heading = gtk::Label::new(Some("Quick settings"));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    content.append(&heading);
    for (label, icon, command, args) in [
        ("Sound", "audio-volume-high-symbolic", "pavucontrol", vec![]),
        ("Display", "video-display-symbolic", "wdisplays", vec![]),
        (
            "Lock screen",
            "system-lock-screen-symbolic",
            "swaylock",
            vec!["-c", "202326"],
        ),
        (
            "Desktop settings",
            "preferences-desktop-wallpaper-symbolic",
            "seglinux-shell",
            vec!["--settings"],
        ),
    ] {
        if !common::installed(command) {
            continue;
        }
        let button = common::command_button(label, icon, command, &args);
        if preview && command == "swaylock" {
            button.set_sensitive(false);
        }
        content.append(&button);
    }
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    if preview {
        let close = gtk::Button::with_label("Close preview");
        let app = app.clone();
        close.connect_clicked(move |_| app.quit());
        content.append(&close);
    }
    content.append(&common::power_buttons(preview));
    popover.set_child(Some(&content));
    popover
}

/// The clock; a click shows the calendar.
fn clock(format: &str) -> gtk::MenuButton {
    let label = gtk::Label::new(None);
    label.add_css_class("clock");
    let button = gtk::MenuButton::builder().child(&label).build();
    button.add_css_class("clock-button");
    let popover = gtk::Popover::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let today = gtk::Label::new(None);
    today.add_css_class("heading");
    content.append(&today);
    content.append(&gtk::Calendar::new());
    popover.set_child(Some(&content));
    button.set_popover(Some(&popover));
    let format = format.to_string();
    let tick = move |label: &gtk::Label, today: &gtk::Label| {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(text) = now.format(&format) {
                label.set_text(&text);
            }
            if let Ok(date) = now.format("%A, %d %B %Y") {
                label.set_tooltip_text(Some(&date));
                today.set_text(&date);
            }
        }
    };
    tick(&label, &today);
    let (label, today) = (label.downgrade(), today.downgrade());
    glib::timeout_add_local(Duration::from_secs(1), move || {
        match (label.upgrade(), today.upgrade()) {
            (Some(label), Some(today)) => {
                tick(&label, &today);
                glib::ControlFlow::Continue
            }
            _ => glib::ControlFlow::Break,
        }
    });
    button
}

#[cfg(test)]
mod tests {
    use super::battery_icon;

    #[test]
    fn battery_icons_in_steps() {
        assert_eq!(battery_icon(87, false), "battery-level-90-symbolic");
        assert_eq!(battery_icon(4, false), "battery-level-0-symbolic");
        assert_eq!(
            battery_icon(100, true),
            "battery-level-100-charging-symbolic"
        );
        assert_eq!(battery_icon(250, false), "battery-level-100-symbolic");
    }
}
