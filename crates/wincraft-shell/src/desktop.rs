//! The desktop: wallpaper, the icons of ~/Desktop and its context menu.
use crate::common::{self, TERMINAL};
use crate::config::Config;
use gtk::{gdk, gio, glib, prelude::*};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

/// One icon on the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// What ~/Desktop shows, in desktop order: folders first, then files, each
/// by name ignoring case; hidden files left out.
pub fn list(dir: &std::path::Path) -> Vec<Item> {
    let mut items: Vec<Item> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .map(|e| Item {
                    path: e.path(),
                    name: e.file_name().to_string_lossy().into_owned(),
                    is_dir: e.path().is_dir(),
                })
                .collect()
        })
        .unwrap_or_default();
    items.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    items
}

pub fn desktop_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Desktop)
        .unwrap_or_else(|| glib::home_dir().join("Desktop"))
}

/// The background of one monitor. Only the first monitor shows icons.
/// The wallpaper, decoded at the size it is shown at (`size`, in pixels):
/// a 2560 x 1440 image shown on a 1366 x 768 screen takes 4 MB of memory
/// instead of 15.
fn wallpaper(path: &std::path::Path, size: Option<(i32, i32)>) -> gtk::Picture {
    use gtk::gdk_pixbuf::Pixbuf;
    if let Some((width, height)) = size.filter(|(w, h)| *w > 0 && *h > 0)
        && let Some((_, image_width, image_height)) = Pixbuf::file_info(path)
        && image_width > 0
        && image_height > 0
    {
        let (w, h) = cover_size((image_width, image_height), (width, height));
        if let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, w, h, true) {
            return gtk::Picture::for_paintable(&gdk::Texture::for_pixbuf(&pixbuf));
        }
    }
    gtk::Picture::for_filename(path)
}

/// The smallest size, keeping the image's proportions, that covers the
/// screen; never larger than the image.
pub fn cover_size(image: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
    let scale = (f64::from(screen.0) / f64::from(image.0))
        .max(f64::from(screen.1) / f64::from(image.1))
        .min(1.0);
    (
        (f64::from(image.0) * scale).ceil() as i32,
        (f64::from(image.1) * scale).ceil() as i32,
    )
}

/// The background of one monitor (`size` in pixels, when known). Only the
/// first monitor shows icons.
pub fn background(config: &Config, icons: bool, size: Option<(i32, i32)>) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("desktop");
    let picture = wallpaper(&config.wallpaper, size);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    overlay.set_child(Some(&picture));
    if icons && config.desktop_icons {
        overlay.add_overlay(&icon_view());
    }
    context_menu(&overlay);
    overlay
}

/// Name and icon for a desktop item: desktop entries show their
/// application, other files their type's icon.
fn describe(item: &Item) -> (String, gio::Icon) {
    if item.path.extension().is_some_and(|e| e == "desktop")
        && let Some(app) = gio::DesktopAppInfo::from_filename(&item.path)
    {
        return (
            app.display_name().to_string(),
            crate::apps::icon(Some(&app)),
        );
    }
    let info = gio::File::for_path(&item.path)
        .query_info(
            "standard::display-name,standard::icon",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok();
    let name = info
        .as_ref()
        .map(|i| i.display_name().to_string())
        .unwrap_or_else(|| item.name.clone());
    let icon = info.and_then(|i| i.icon()).unwrap_or_else(|| {
        gio::ThemedIcon::new(if item.is_dir {
            "folder"
        } else {
            "text-x-generic"
        })
        .upcast()
    });
    (name, icon)
}

fn icon_view() -> gtk::FlowBox {
    let flow = gtk::FlowBox::new();
    flow.add_css_class("desktop-icons");
    flow.set_orientation(gtk::Orientation::Vertical);
    flow.set_selection_mode(gtk::SelectionMode::Single);
    flow.set_activate_on_single_click(false);
    flow.set_homogeneous(true);
    flow.set_max_children_per_line(64);
    flow.set_row_spacing(4);
    flow.set_column_spacing(4);
    flow.set_halign(gtk::Align::Start);
    flow.set_valign(gtk::Align::Start);
    flow.set_margin_top(12);
    flow.set_margin_start(12);
    flow.set_margin_bottom(72);

    let dir = desktop_dir();
    let items: Rc<RefCell<Vec<Item>>> = Rc::default();
    let refresh = {
        let flow = flow.clone();
        let items = items.clone();
        let dir = dir.clone();
        move || {
            while let Some(child) = flow.first_child() {
                flow.remove(&child);
            }
            let listed = list(&dir);
            for item in &listed {
                let (name, icon) = describe(item);
                let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
                tile.add_css_class("desktop-icon");
                let image = gtk::Image::from_gicon(&icon);
                image.set_pixel_size(48);
                tile.append(&image);
                let label = gtk::Label::new(Some(&name));
                label.set_wrap(true);
                label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                label.set_justify(gtk::Justification::Center);
                label.set_lines(2);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_width_chars(11);
                label.set_max_width_chars(11);
                tile.append(&label);
                tile.set_tooltip_text(Some(&name));
                flow.insert(&tile, -1);
            }
            flow.unselect_all();
            *items.borrow_mut() = listed;
        }
    };
    refresh();
    let opened = items.clone();
    flow.connect_child_activated(move |_, child| {
        if let Some(item) = usize::try_from(child.index())
            .ok()
            .and_then(|i| opened.borrow().get(i).cloned())
        {
            common::open_path(&item.path);
        }
    });
    // Follow changes, at most a few times a second.
    if let Ok(monitor) = gio::File::for_path(&dir)
        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
    {
        let pending = Rc::new(std::cell::Cell::new(false));
        monitor.connect_changed(move |_, _, _, _| {
            if pending.replace(true) {
                return;
            }
            let pending = pending.clone();
            let refresh = refresh.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                pending.set(false);
                refresh();
            });
        });
        // The monitor lives as long as the icons.
        flow.connect_destroy(move |_| {
            monitor.cancel();
        });
    }
    flow
}

/// Right-click on the desktop, as in Plasma: open a terminal or the
/// Desktop folder, change the wallpaper and other settings.
fn context_menu(overlay: &gtk::Overlay) {
    let actions = gio::SimpleActionGroup::new();
    let add = |name: &str, f: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| f());
        actions.add_action(&action);
    };
    add(
        "terminal",
        Box::new(|| {
            let dir = desktop_dir();
            let mut cmd = std::process::Command::new(TERMINAL);
            cmd.current_dir(if dir.is_dir() { dir } else { glib::home_dir() });
            if let Err(e) = cmd.spawn() {
                common::show_error(&format!("Cannot start {TERMINAL}: {e}"));
            }
        }),
    );
    add(
        "folder",
        Box::new(|| {
            let dir = desktop_dir();
            let _ = std::fs::create_dir_all(&dir);
            common::open_path(&dir);
        }),
    );
    add(
        "settings",
        Box::new(|| common::run("wincraft-shell", &["--settings".into()])),
    );
    overlay.insert_action_group("desk", Some(&actions));

    let menu = gio::Menu::new();
    if common::installed(TERMINAL) {
        menu.append(Some("Open terminal here"), Some("desk.terminal"));
    }
    menu.append(Some("Open Desktop folder"), Some("desk.folder"));
    menu.append(
        Some("Wallpaper and desktop settings…"),
        Some("desk.settings"),
    );
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);
    popover.set_parent(overlay);
    let owned = popover.clone();
    overlay.connect_destroy(move |_| owned.unparent());

    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    click.connect_pressed(move |_, _, x, y| {
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    overlay.add_controller(click);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallpaper_sizes() {
        assert_eq!(cover_size((2560, 1440), (1366, 768)), (1366, 769));
        assert_eq!(cover_size((2560, 1440), (1920, 1200)), (2134, 1200));
        assert_eq!(cover_size((2560, 1440), (3840, 2160)), (2560, 1440));
    }

    #[test]
    fn folders_first_then_files_by_name() {
        let dir = tempfile::tempdir().unwrap();
        for d in ["Wincraft", "brawlhalla", "42cursus"] {
            std::fs::create_dir(dir.path().join(d)).unwrap();
        }
        for f in ["notes.txt", "Alpha.png", ".hidden"] {
            std::fs::write(dir.path().join(f), "").unwrap();
        }
        let names: Vec<(String, bool)> = list(dir.path())
            .into_iter()
            .map(|i| (i.name, i.is_dir))
            .collect();
        assert_eq!(
            names,
            [
                ("42cursus".into(), true),
                ("brawlhalla".into(), true),
                ("Wincraft".into(), true),
                ("Alpha.png".into(), false),
                ("notes.txt".into(), false),
            ]
        );
        assert!(list(&dir.path().join("missing")).is_empty());
    }
}
