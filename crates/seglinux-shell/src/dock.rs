//! The floating dock at the bottom of the screen, like Plasma's floating
//! panel: Start, show desktop, one icon per application (pinned, running or
//! both), the tray and the clock.
use crate::pins::{self, Change};
use crate::services::Services;
use crate::tasks::{self, Action, Task, Tasks};
use crate::{apps, common, config::Config, launcher, tray};
use gtk::{gdk, gio, glib, prelude::*};
use std::{cell::OnceCell, cell::RefCell, rc::Rc};

/// Start buttons, so `gapplication action org.seglinux.Shell menu` can open one.
pub type Menus = Rc<RefCell<Vec<glib::WeakRef<gtk::MenuButton>>>>;

/// One dock icon: a pinned application and/or a group of windows of the
/// same application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// Index into the pinned list, if pinned.
    pub pin: Option<usize>,
    pub app_id: String,
    pub windows: Vec<Task>,
}

/// Pinned applications first, in their order, with their windows; then the
/// other windows grouped by application, in the order they opened.
pub fn slots(pins: &[String], windows: &[Task]) -> Vec<Slot> {
    let mut slots: Vec<Slot> = pins
        .iter()
        .enumerate()
        .map(|(i, pin)| Slot {
            pin: Some(i),
            app_id: pin.clone(),
            windows: Vec::new(),
        })
        .collect();
    for window in windows {
        if let Some(slot) = slots
            .iter_mut()
            .find(|s| s.pin.is_some() && apps::belongs_to(&s.app_id, &window.app_id))
        {
            slot.windows.push(window.clone());
            continue;
        }
        let key = window.app_id.to_lowercase();
        match slots
            .iter_mut()
            .find(|s| s.pin.is_none() && !key.is_empty() && s.app_id.to_lowercase() == key)
        {
            Some(slot) => slot.windows.push(window.clone()),
            None => slots.push(Slot {
                pin: None,
                app_id: window.app_id.clone(),
                windows: vec![window.clone()],
            }),
        }
    }
    slots
}

/// The window a click on a group should activate: the one after the active
/// window (cycling), or the first.
pub fn next_window(windows: &[Task]) -> Option<u32> {
    match windows.iter().position(|w| w.active && !w.minimized) {
        Some(i) if windows.len() == 1 => Some(windows[i].id),
        Some(i) => Some(windows[(i + 1) % windows.len()].id),
        None => windows.first().map(|w| w.id),
    }
}

pub fn dock(
    app: &gtk::Application,
    menus: &Menus,
    config: &Config,
    preview: bool,
    services: &Services,
) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar.add_css_class("dock");

    let start = gtk::MenuButton::builder()
        .icon_name(common::themed_icon(&[
            "seglinux",
            "start-here",
            "view-app-grid-symbolic",
        ]))
        .tooltip_text("Start (Super+Space)")
        .build();
    start.add_css_class("start");
    let menu = launcher::menu(preview, &config.pinned);
    // Start's tooltip would cover the open menu.
    let tip = start.downgrade();
    menu.connect_show(move |_| {
        if let Some(start) = tip.upgrade() {
            start.set_has_tooltip(false);
        }
    });
    let tip = start.downgrade();
    menu.connect_closed(move |_| {
        if let Some(start) = tip.upgrade() {
            start.set_has_tooltip(true);
        }
    });
    start.set_popover(Some(&menu));
    menus.borrow_mut().push(start.downgrade());
    bar.append(&start);

    let handle: Rc<OnceCell<Tasks>> = Rc::default();
    let show_desktop = gtk::Button::from_icon_name(&common::themed_icon(&[
        "user-desktop-symbolic",
        "user-desktop",
        "desktop",
    ]));
    show_desktop.set_tooltip_text(Some("Show the desktop"));
    let desktop_handle = handle.clone();
    show_desktop.connect_clicked(move |_| {
        if let Some(tasks) = desktop_handle.get() {
            tasks.send(Action::ShowDesktop);
        }
    });
    if config.show_desktop_button {
        bar.append(&show_desktop);
    }
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    let apps_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    apps_box.add_css_class("apps");
    bar.append(&apps_box);

    // Pinned applications that are installed.
    let pins: Rc<Vec<(String, gio::DesktopAppInfo)>> = Rc::new(
        config
            .pinned
            .iter()
            .filter_map(|id| apps::lookup(id).map(|a| (id.clone(), a)))
            .collect(),
    );
    let icon_size = config.dock_icon_size as i32;
    let render = {
        // Weak: the window watcher outlives a dock that was replaced only
        // until it notices, and must not keep it alive.
        let apps_box = apps_box.downgrade();
        let handle = handle.clone();
        move |windows: &[Task]| {
            let Some(apps_box) = apps_box.upgrade() else {
                return;
            };
            while let Some(child) = apps_box.first_child() {
                apps_box.remove(&child);
            }
            let ids: Vec<String> = pins.iter().map(|(id, _)| id.clone()).collect();
            for slot in slots(&ids, windows) {
                let (app, place) = match slot.pin {
                    Some(i) => (
                        Some(pins[i].1.clone()),
                        Some(Place {
                            previous: i.checked_sub(1).map(|p| ids[p].clone()),
                            next: ids.get(i + 1).cloned(),
                        }),
                    ),
                    None => (apps::for_window(&slot.app_id), None),
                };
                apps_box.append(&app_button(&slot, app, place, handle.clone(), icon_size));
            }
        }
    };
    render(&[]);
    if !preview {
        let tasks = tasks::watch(&apps_box, render);
        let _ = handle.set(tasks);
    }

    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&tray::tray(app, preview, config, services));
    bar
}

/// Where a pinned icon is among the pinned icons shown: its neighbours.
struct Place {
    previous: Option<String>,
    next: Option<String>,
}

fn app_button(
    slot: &Slot,
    app: Option<gio::DesktopAppInfo>,
    place: Option<Place>,
    handle: Rc<OnceCell<Tasks>>,
    icon_size: i32,
) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("app");
    let image = gtk::Image::from_gicon(&apps::icon(app.as_ref()));
    image.set_pixel_size(icon_size);
    button.set_child(Some(&image));
    if !slot.windows.is_empty() {
        button.add_css_class("running");
    }
    if slot.windows.iter().any(|w| w.active && !w.minimized) {
        button.add_css_class("active");
    }
    let name = app
        .as_ref()
        .map(|a| a.display_name().to_string())
        .unwrap_or_else(|| slot.app_id.clone());
    let mut tooltip = name.clone();
    for w in &slot.windows {
        tooltip.push_str("\n• ");
        tooltip.push_str(&w.title);
    }
    button.set_tooltip_text(Some(&tooltip));

    // Left click: start the application, or focus/minimize its windows.
    let windows = slot.windows.clone();
    let start_app = app.clone();
    let click_handle = handle.clone();
    button.connect_clicked(move |_| match next_window(&windows) {
        Some(id) => {
            if let Some(tasks) = click_handle.get() {
                tasks.send(Action::Toggle(id));
            }
        }
        None => {
            if let Some(app) = &start_app {
                common::launch_app(app);
            }
        }
    });

    // Middle click: a new window, as in Plasma.
    let middle = gtk::GestureClick::new();
    middle.set_button(gdk::BUTTON_MIDDLE);
    let new_app = app.clone();
    middle.connect_pressed(move |_, _, _, _| {
        if let Some(app) = &new_app {
            common::launch_app(app);
        }
    });
    button.add_controller(middle);

    // The application's desktop file id: what pinning saves.
    let id = if place.is_some() {
        Some(slot.app_id.clone())
    } else {
        app.as_ref().and_then(pins::desktop_id)
    };
    if let (Some(id), Some(_)) = (&id, &place) {
        drag_to_reorder(&button, &image, id);
    }

    // Right click: the application's actions, new window, pin, unpin and
    // move, close.
    let windows: Vec<u32> = slot.windows.iter().map(|w| w.id).collect();
    common::context_menu(&button, move || {
        app_menu(
            &name,
            app.as_ref(),
            id.as_deref(),
            place.as_ref(),
            &windows,
            &handle,
        )
    });
    button
}

/// A dock icon's menu (see app_button); `windows` are the ids of its windows.
fn app_menu(
    name: &str,
    app: Option<&gio::DesktopAppInfo>,
    id: Option<&str>,
    place: Option<&Place>,
    windows: &[u32],
    handle: &Rc<OnceCell<Tasks>>,
) -> gtk::Popover {
    let menu = gtk::Popover::new();
    menu.add_css_class("menu");
    let items = gtk::Box::new(gtk::Orientation::Vertical, 2);
    items.set_width_request(190);
    let title = gtk::Label::new(Some(name));
    title.add_css_class("menu-title");
    title.set_xalign(0.0);
    items.append(&title);
    if let Some(app) = app {
        for action in apps::action_buttons(app) {
            items.append(&action);
        }
        let open = common::menu_item("New window");
        let app = app.clone();
        open.connect_clicked(move |b| {
            common::launch_app(&app);
            common::close_popover(b);
        });
        items.append(&open);
    }
    if let Some(id) = id {
        items.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let pinned = place.is_some();
        items.append(&change_button(
            if pinned {
                "Unpin from dock"
            } else {
                "Pin to dock"
            },
            id,
            if pinned { Change::Unpin } else { Change::Pin },
        ));
        if let Some(place) = place {
            if let Some(previous) = &place.previous {
                items.append(&change_button(
                    "Move left",
                    id,
                    Change::TakePlaceOf(previous.clone()),
                ));
            }
            if let Some(next) = &place.next {
                items.append(&change_button(
                    "Move right",
                    id,
                    Change::TakePlaceOf(next.clone()),
                ));
            }
        }
    }
    if !windows.is_empty() {
        items.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let close = common::menu_item(if windows.len() == 1 {
            "Close window"
        } else {
            "Close all windows"
        });
        let ids = windows.to_vec();
        let handle = handle.clone();
        close.connect_clicked(move |b| {
            if let Some(tasks) = handle.get() {
                for id in &ids {
                    tasks.send(Action::Close(*id));
                }
            }
            common::close_popover(b);
        });
        items.append(&close);
    }
    menu.set_child(Some(&items));
    menu
}

/// A menu button that changes the pins.
fn change_button(label: &str, id: &str, change: Change) -> gtk::Button {
    let button = common::menu_item(label);
    let id = id.to_string();
    button.connect_clicked(move |b| {
        common::close_popover(b);
        pins::update(&id, change.clone());
    });
    button
}

/// Pinned icons can be dragged onto one another to change their order.
fn drag_to_reorder(button: &gtk::Button, image: &gtk::Image, id: &str) {
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::MOVE);
    let dragged = id.to_string();
    source
        .connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&dragged.to_value())));
    let icon = image.downgrade();
    source.connect_drag_begin(move |source, _| {
        if let Some(icon) = icon.upgrade() {
            source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&icon))), 0, 0);
        }
    });
    button.add_controller(source);

    let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    let onto = id.to_string();
    target.connect_drop(move |_, value, _, _| match value.get::<String>() {
        Ok(dragged) if dragged != onto => {
            pins::update(&dragged, Change::TakePlaceOf(onto.clone()));
            true
        }
        _ => false,
    });
    button.add_controller(target);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: u32, app_id: &str, active: bool) -> Task {
        Task {
            id,
            title: format!("window {id}"),
            app_id: app_id.into(),
            active,
            minimized: false,
        }
    }

    #[test]
    fn pinned_first_then_other_apps_grouped() {
        let pins = [
            "org.kde.dolphin".to_string(),
            "firefox".into(),
            "steam".into(),
        ];
        let windows = [
            task(1, "firefox", false),
            task(2, "foot", true),
            task(3, "firefox", false),
            task(4, "Foot", false),
            task(5, "", false),
        ];
        let s = slots(&pins, &windows);
        assert_eq!(s.len(), 5);
        assert_eq!(s[0].windows.len(), 0);
        assert_eq!(
            s[1].windows.iter().map(|w| w.id).collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(
            (s[3].pin, s[3].app_id.as_str(), s[3].windows.len()),
            (None, "foot", 2)
        );
        assert_eq!((s[4].pin, s[4].windows[0].id), (None, 5));
    }

    #[test]
    fn terminal_windows_join_the_terminal_pin() {
        let s = slots(&["seglinux-terminal".into()], &[task(9, "foot", false)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].windows[0].id, 9);
    }

    #[test]
    fn clicks_cycle_through_windows() {
        assert_eq!(next_window(&[]), None);
        assert_eq!(next_window(&[task(1, "a", false)]), Some(1));
        assert_eq!(next_window(&[task(1, "a", true)]), Some(1));
        let two = [task(1, "a", true), task(2, "a", false)];
        assert_eq!(next_window(&two), Some(2));
        let none_active = [task(1, "a", false), task(2, "a", false)];
        assert_eq!(next_window(&none_active), Some(1));
    }
}
