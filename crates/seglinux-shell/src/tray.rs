//! The right end of the dock, as Plasma's system tray: other applications'
//! icons (Steam, Discord, ...), network, volume, battery, notifications,
//! quick settings and the clock with its calendar.
use crate::common::{self, TERMINAL};
use crate::config::Config;
use crate::services::Services;
use crate::{notify, sni};
use gtk::{gdk, gio, glib, prelude::*};
use std::time::Duration;

pub fn tray(
    app: &gtk::Application,
    preview: bool,
    config: &Config,
    services: &Services,
) -> gtk::Box {
    let tray = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    tray.add_css_class("tray");
    tray.append(&app_icons(&services.tray));
    tray.append(&network());
    tray.append(&volume(services));
    if let Some(battery) = battery(config.battery_percentage) {
        tray.append(&battery);
    }
    tray.append(&notifications(&services.notifications));
    let settings = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("Quick settings")
        .build();
    settings.set_popover(Some(&quick_settings(app, preview)));
    tray.append(&settings);
    tray.append(&clock(&config.clock_format));
    tray
}

/// The icons applications put in the tray (StatusNotifierItem): a click
/// shows the application, a right click its menu, a middle click its
/// secondary action; scrolling is passed on.
fn app_icons(host: &sni::Host) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("tray-apps");
    let render = {
        let row = row.downgrade();
        let host = host.clone();
        move || -> bool {
            let Some(row) = row.upgrade() else {
                return false;
            };
            while let Some(child) = row.first_child() {
                row.remove(&child);
            }
            for (key, props) in host.items() {
                row.append(&app_icon(&host, &key, &props));
            }
            row.set_visible(row.first_child().is_some());
            true
        }
    };
    render();
    host.subscribe(render);
    row
}

fn app_icon(host: &sni::Host, key: &str, props: &sni::Props) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_child(Some(&sni::image(props, 18)));
    let label = props.label();
    if !label.is_empty() {
        button.set_tooltip_text(Some(&label));
    }
    if props.status == "NeedsAttention" {
        button.add_css_class("attention");
    }
    let (left, k) = (host.clone(), key.to_string());
    button.connect_clicked(move |b| left.activate(&k, b.upcast_ref()));
    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    let (host_right, k) = (host.clone(), key.to_string());
    right.connect_pressed(move |gesture, _, _, _| {
        if let Some(widget) = gesture.widget() {
            host_right.open_menu(&k, &widget);
        }
    });
    button.add_controller(right);
    let middle = gtk::GestureClick::new();
    middle.set_button(gdk::BUTTON_MIDDLE);
    let (host_middle, k) = (host.clone(), key.to_string());
    middle.connect_pressed(move |_, _, _, _| host_middle.secondary_activate(&k));
    button.add_controller(middle);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let (host_scroll, k) = (host.clone(), key.to_string());
    scroll.connect_scroll(move |_, _, dy| {
        host_scroll.scroll(&k, if dy < 0.0 { 120 } else { -120 });
        glib::Propagation::Stop
    });
    button.add_controller(scroll);
    button
}

/// The notifications: a bell with the number of unread ones; its popover
/// holds the history, Do Not Disturb and Clear all.
fn notifications(center: &notify::Center) -> gtk::MenuButton {
    let image = gtk::Image::new();
    let badge = gtk::Label::new(None);
    badge.add_css_class("badge");
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::Start);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&image));
    overlay.add_overlay(&badge);
    let button = gtk::MenuButton::builder().child(&overlay).build();
    button.add_css_class("notifications-button");
    let popover = gtk::Popover::new();
    popover.add_css_class("notification-center");
    button.set_popover(Some(&popover));

    let fill = {
        let popover = popover.downgrade();
        let center = center.clone();
        move || {
            let Some(popover) = popover.upgrade() else {
                return;
            };
            popover.set_child(Some(&history(&center)));
        }
    };
    let update = {
        let (image, badge, button) = (image.downgrade(), badge.downgrade(), button.downgrade());
        let popover = popover.downgrade();
        let center = center.clone();
        let fill = fill.clone();
        move || -> bool {
            let (Some(image), Some(badge), Some(button)) =
                (image.upgrade(), badge.upgrade(), button.upgrade())
            else {
                return false;
            };
            let unread = center.unread();
            image.set_icon_name(Some(&common::themed_icon(if center.dnd() {
                &[
                    "notifications-disabled-symbolic",
                    "notification-disabled-symbolic",
                ][..]
            } else {
                &[
                    "notifications-symbolic",
                    "notification-symbolic",
                    "preferences-system-notifications-symbolic",
                ][..]
            })));
            badge.set_text(&unread.min(99).to_string());
            badge.set_visible(unread > 0);
            button.set_tooltip_text(Some(&match (unread, center.dnd()) {
                (0, false) => "No new notifications".to_string(),
                (0, true) => "Do not disturb".to_string(),
                (1, _) => "1 new notification".to_string(),
                (n, _) => format!("{n} new notifications"),
            }));
            if popover.upgrade().is_some_and(|p| p.is_visible()) {
                fill();
            }
            true
        }
    };
    update();
    center.subscribe(update);
    let opened = center.clone();
    let tip = button.downgrade();
    popover.connect_show(move |_| {
        // Its tooltip would cover the open history.
        if let Some(button) = tip.upgrade() {
            button.set_has_tooltip(false);
        }
        fill();
        opened.mark_read();
    });
    let tip = button.downgrade();
    popover.connect_closed(move |_| {
        if let Some(button) = tip.upgrade() {
            button.set_has_tooltip(true);
        }
    });
    button
}

/// The notification history, newest first.
fn history(center: &notify::Center) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_size_request(400, -1);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some("Notifications"));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    head.append(&title);
    head.append(&gtk::Label::new(Some("Do not disturb")));
    let dnd = gtk::Switch::new();
    dnd.set_active(center.dnd());
    dnd.set_valign(gtk::Align::Center);
    let toggled = center.clone();
    dnd.connect_active_notify(move |s| toggled.set_dnd(s.is_active()));
    head.append(&dnd);
    content.append(&head);

    let notices = center.history();
    if notices.is_empty() {
        let empty = gtk::Label::new(Some("No notifications"));
        empty.add_css_class("dim-label");
        empty.set_margin_top(24);
        empty.set_margin_bottom(24);
        content.append(&empty);
        return content;
    }
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for notice in &notices {
        list.append(&center.card(notice, false));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(480)
        .child(&list)
        .build();
    content.append(&scroll);
    let clear = gtk::Button::with_label("Clear all");
    clear.set_halign(gtk::Align::End);
    let cleared = center.clone();
    clear.connect_clicked(move |_| cleared.clear());
    content.append(&clear);
    content
}

/// Network status from GLib's network monitor; a click opens the
/// connection editor (or nmtui in a terminal on the Lite edition).
fn network() -> gtk::Button {
    let button = gtk::Button::new();
    let image = gtk::Image::new();
    button.set_child(Some(&image));
    let update = {
        let image = image.downgrade();
        let button = button.downgrade();
        move |monitor: &gio::NetworkMonitor| {
            let (Some(image), Some(button)) = (image.upgrade(), button.upgrade()) else {
                return;
            };
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
    // The monitor is shared by the whole process: disconnect when the dock
    // is replaced, or every rebuild would leave a handler behind.
    let handler =
        std::cell::Cell::new(Some(monitor.connect_network_changed(move |m, _| update(m))));
    button.connect_destroy(move |_| {
        if let Some(id) = handler.take() {
            monitor.disconnect(id);
        }
    });
    button.connect_clicked(|_| {
        if common::installed("nm-connection-editor") {
            common::run("nm-connection-editor", &[]);
        } else {
            common::run(TERMINAL, &["nmtui".into()]);
        }
    });
    button
}

/// Volume: a click opens the mixer, scrolling changes the volume (with the
/// on-screen display), a middle click mutes.
fn volume(services: &Services) -> gtk::Button {
    let button = gtk::Button::from_icon_name("audio-volume-high-symbolic");
    button.set_tooltip_text(Some("Volume (scroll to change, middle click to mute)"));
    button.connect_clicked(|_| {
        if common::installed("pavucontrol") {
            common::run("pavucontrol", &[]);
        }
    });
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let osd = services.osd.clone();
    scroll.connect_scroll(move |_, _, dy| {
        osd.volume(if dy < 0.0 { "up" } else { "down" });
        glib::Propagation::Stop
    });
    button.add_controller(scroll);
    let middle = gtk::GestureClick::new();
    middle.set_button(gdk::BUTTON_MIDDLE);
    let osd = services.osd.clone();
    middle.connect_pressed(move |_, _, _, _| osd.volume("mute"));
    button.add_controller(middle);
    button
}

/// Bluetooth's settings, when the computer has Bluetooth: Blueman's
/// manager, else Plasma's page in System Settings.
fn bluetooth_tool() -> Option<(&'static str, Vec<&'static str>)> {
    let adapter = std::fs::read_dir("/sys/class/bluetooth")
        .ok()?
        .flatten()
        .next()
        .is_some();
    if !adapter {
        None
    } else if common::installed("blueman-manager") {
        Some(("blueman-manager", vec![]))
    } else if common::installed("systemsettings") {
        Some(("systemsettings", vec!["kcm_bluetooth"]))
    } else {
        None
    }
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

fn battery(percentage: bool) -> Option<gtk::Box> {
    read_battery()?;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    row.add_css_class("battery");
    let image = gtk::Image::new();
    let label = gtk::Label::new(None);
    label.set_visible(percentage);
    row.append(&image);
    row.append(&label);
    let update = move |image: &gtk::Image, label: &gtk::Label| {
        if let Some((level, charging)) = read_battery() {
            image.set_icon_name(Some(&common::themed_icon(&[
                &battery_icon(level, charging),
                "battery-symbolic",
            ])));
            label.set_text(&format!("{level}%"));
            image.set_tooltip_text(Some(&format!(
                "Battery {level}%{}",
                if charging { ", charging" } else { "" }
            )));
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
    let bluetooth = bluetooth_tool();
    let (bluetooth_command, bluetooth_args) = bluetooth.clone().unwrap_or(("", vec![]));
    for (label, icon, command, args) in [
        ("Sound", "audio-volume-high-symbolic", "pavucontrol", vec![]),
        (
            "Bluetooth",
            "bluetooth-active-symbolic",
            bluetooth_command,
            bluetooth_args,
        ),
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
        if command.is_empty() || !common::installed(command) {
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
