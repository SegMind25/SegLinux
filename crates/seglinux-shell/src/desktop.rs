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
    /// The trash, first on the desktop (trash.rs).
    pub trash: bool,
}

impl Item {
    fn trash() -> Item {
        Item {
            path: crate::trash::dir().join("files"),
            name: "Trash".into(),
            is_dir: true,
            trash: true,
        }
    }
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
                    trash: false,
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

/// Puts a launcher for `app` on the desktop, as Plasma's "Add to Desktop":
/// a copy of its desktop file in ~/Desktop. Returns where it is.
pub fn add_launcher(app: &gio::DesktopAppInfo) -> Result<PathBuf, String> {
    let source = app
        .filename()
        .ok_or_else(|| format!("{} has no launcher file.", app.display_name()))?;
    copy_launcher(&source, &desktop_dir())
}

/// Copies the desktop file `source` into `dir`, unless one of that name is
/// there already.
pub fn copy_launcher(source: &std::path::Path, dir: &std::path::Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let name = source
        .file_name()
        .ok_or_else(|| format!("{} is not a file.", source.display()))?;
    let dest = dir.join(name);
    if !dest.exists() {
        let failed =
            |e: std::io::Error| format!("Cannot add {} to the desktop: {e}", source.display());
        std::fs::create_dir_all(dir).map_err(failed)?;
        std::fs::copy(source, &dest).map_err(failed)?;
        // Executable, as file managers expect of trusted launchers.
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).map_err(failed)?;
    }
    Ok(dest)
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
    picture.set_content_fit(match config.wallpaper_fit {
        crate::config::WallpaperFit::Cover => gtk::ContentFit::Cover,
        crate::config::WallpaperFit::Contain => gtk::ContentFit::Contain,
        crate::config::WallpaperFit::Fill => gtk::ContentFit::Fill,
    });
    picture.set_can_shrink(true);
    overlay.set_child(Some(&picture));
    if icons && config.desktop_icons {
        overlay.add_overlay(&icon_view(
            config.desktop_icon_size as i32,
            config.dock_position == crate::config::DockPosition::Top,
        ));
    }
    context_menu(&overlay);
    overlay
}

/// Name and icon for a desktop item: desktop entries show their
/// application, other files their type's icon.
fn describe(item: &Item) -> (String, gio::Icon) {
    if item.trash {
        let icon = crate::trash::icon_name(crate::trash::count());
        return (item.name.clone(), gio::ThemedIcon::new(icon).upcast());
    }
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

fn icon_view(icon_size: i32, dock_top: bool) -> gtk::FlowBox {
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
    flow.set_margin_top(if dock_top { 96 } else { 12 });
    flow.set_margin_start(12);
    flow.set_margin_bottom(72);

    let dir = desktop_dir();
    let items: Rc<RefCell<Vec<Item>>> = Rc::default();
    let refresh = {
        // Weak: the folder monitor below must not keep replaced icons alive.
        let flow = flow.downgrade();
        let items = items.clone();
        let dir = dir.clone();
        move || {
            let Some(flow) = flow.upgrade() else {
                return;
            };
            while let Some(child) = flow.first_child() {
                flow.remove(&child);
            }
            let mut listed = vec![Item::trash()];
            listed.extend(list(&dir));
            for item in &listed {
                let (name, icon) = describe(item);
                let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
                tile.add_css_class("desktop-icon");
                let image = gtk::Image::from_gicon(&icon);
                image.set_pixel_size(icon_size);
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
                // Only names the label cuts short: the pointer starts out
                // over the first icon at sign-in, and a tooltip repeating
                // the label would sit there until it moves.
                if name_is_cut(&name) {
                    tile.set_tooltip_text(Some(&name));
                }
                let menu_item = item.clone();
                let menu_tile = tile.downgrade();
                common::context_menu(&tile, move || item_menu(&menu_item, menu_tile.clone()));
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
            if item.trash {
                crate::trash::open();
            } else {
                common::open_path(&item.path);
            }
        }
    });
    // Follow changes, at most a few times a second: the Desktop folder, and
    // the trash for its icon (full or empty).
    let trash_files = crate::trash::dir().join("files");
    let _ = std::fs::create_dir_all(&trash_files);
    let pending = Rc::new(std::cell::Cell::new(false));
    for watched in [dir, trash_files] {
        let Ok(monitor) = gio::File::for_path(&watched)
            .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        else {
            continue;
        };
        let pending = pending.clone();
        let refresh = refresh.clone();
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
        // The monitors live as long as the icons.
        flow.connect_destroy(move |_| {
            monitor.cancel();
        });
    }
    flow
}

/// The right-click menu of a desktop icon: open, rename, move to the trash;
/// for the trash, open and empty it.
fn item_menu(item: &Item, tile: glib::WeakRef<gtk::Box>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.add_css_class("menu");
    let add = |label: &str, f: Box<dyn Fn()>| {
        let button = common::menu_item(label);
        button.add_css_class("flat");
        button.connect_clicked(move |button| {
            common::close_popover(button);
            f();
        });
        column.append(&button);
    };
    if item.trash {
        add("Open", Box::new(crate::trash::open));
        if crate::trash::count() > 0 {
            add(
                "Empty Trash…",
                Box::new(|| {
                    let count = crate::trash::count();
                    common::ask(
                        "Empty the trash?",
                        &format!(
                            "{count} item{} will be deleted for good.",
                            if count == 1 { "" } else { "s" }
                        ),
                        "Empty Trash",
                        || {
                            if let Err(e) = crate::trash::empty_in(&crate::trash::dir()) {
                                common::show_error(&format!("Cannot empty the trash: {e}"));
                            }
                        },
                    );
                }),
            );
        }
    } else {
        let path = item.path.clone();
        add("Open", Box::new(move || common::open_path(&path)));
        let (path, name) = (item.path.clone(), item.name.clone());
        add(
            "Rename…",
            Box::new(move || {
                let Some(tile) = tile.upgrade() else {
                    return;
                };
                let path = path.clone();
                ask_name(tile.upcast_ref(), None, &name, "Rename", move |new| {
                    let target = valid_name(path.parent().unwrap_or(&path), new)?;
                    std::fs::rename(&path, &target).map_err(|e| format!("Cannot rename: {e}"))
                });
            }),
        );
        let path = item.path.clone();
        add(
            "Move to Trash",
            Box::new(move || {
                if let Err(e) = crate::trash::move_to_trash(&path) {
                    common::show_error(&e);
                }
            }),
        );
    }
    popover.set_child(Some(&column));
    popover
}

/// Where a file named `name` would go in `dir`, if that name can be used:
/// not empty, no slash, nothing of that name there yet.
pub fn valid_name(dir: &std::path::Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("Type a name.".into());
    }
    if name.contains('/') {
        return Err("A name cannot contain a slash (/).".into());
    }
    let target = dir.join(name);
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(format!("“{name}” is already there."));
    }
    Ok(target)
}

/// A small form over `parent` asking for a name (renaming, a new folder):
/// `done` gets the name and says what went wrong, if anything.
fn ask_name(
    parent: &gtk::Widget,
    at: Option<(f64, f64)>,
    initial: &str,
    verb: &str,
    done: impl Fn(&str) -> Result<(), String> + 'static,
) {
    let popover = gtk::Popover::new();
    popover.set_parent(parent);
    if let Some((x, y)) = at {
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let entry = gtk::Entry::builder().text(initial).width_chars(24).build();
    let button = gtk::Button::with_label(verb);
    button.add_css_class("suggested-action");
    row.append(&entry);
    row.append(&button);
    column.append(&row);
    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_xalign(0.0);
    error.set_visible(false);
    column.append(&error);
    popover.set_child(Some(&column));
    popover.connect_closed(|popover| popover.unparent());
    let submit = {
        let (entry, error, popover) = (entry.downgrade(), error.downgrade(), popover.downgrade());
        Rc::new(move || {
            let (Some(entry), Some(error), Some(popover)) =
                (entry.upgrade(), error.upgrade(), popover.upgrade())
            else {
                return;
            };
            match done(&entry.text()) {
                Ok(()) => popover.popdown(),
                Err(message) => {
                    error.set_text(&message);
                    error.set_visible(true);
                }
            }
        })
    };
    let on_enter = submit.clone();
    entry.connect_activate(move |_| on_enter());
    button.connect_clicked(move |_| submit());
    popover.popup();
    // The name without its extension is selected, ready to type over.
    let stem = initial
        .rfind('.')
        .filter(|&i| i > 0)
        .unwrap_or(initial.len());
    entry.grab_focus();
    entry.select_region(0, initial[..stem].chars().count() as i32);
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
            let dir = if dir.is_dir() { dir } else { glib::home_dir() };
            common::run_in(Some(&dir), TERMINAL, &[]);
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
    // Where the menu was opened, for the new folder's name form.
    let clicked: Rc<std::cell::Cell<(f64, f64)>> = Rc::default();
    let place = clicked.clone();
    let form_parent = overlay.downgrade();
    add(
        "newfolder",
        Box::new(move || {
            let Some(overlay) = form_parent.upgrade() else {
                return;
            };
            ask_name(
                overlay.upcast_ref(),
                Some(place.get()),
                "New Folder",
                "Create",
                |name| {
                    let dir = desktop_dir();
                    let target = valid_name(&dir, name)?;
                    std::fs::create_dir_all(&target)
                        .map_err(|e| format!("Cannot create the folder: {e}"))
                },
            );
        }),
    );
    add(
        "settings",
        Box::new(|| {
            common::run(
                "seglinux-shell",
                &["--settings".into(), "appearance".into()],
            )
        }),
    );
    overlay.insert_action_group("desk", Some(&actions));

    let menu = gio::Menu::new();
    if common::installed(TERMINAL) {
        menu.append(Some("Open terminal here"), Some("desk.terminal"));
    }
    menu.append(Some("New folder…"), Some("desk.newfolder"));
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
        clicked.set((x, y));
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    });
    overlay.add_controller(click);
}

/// Whether a desktop icon's label (two lines of 11 characters) cannot show
/// the whole name.
pub fn name_is_cut(name: &str) -> bool {
    let mut lines = 1;
    let mut used = 0;
    for word in name.split_whitespace() {
        let len = word.chars().count();
        if used > 0 && used + 1 + len <= 11 {
            used += 1 + len;
            continue;
        }
        if used > 0 {
            lines += 1;
        }
        // A long word is broken across lines.
        lines += (len - 1) / 11;
        used = (len - 1) % 11 + 1;
    }
    lines > 2
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
        for d in ["SegLinux", "brawlhalla", "42cursus"] {
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
                ("SegLinux".into(), true),
                ("Alpha.png".into(), false),
                ("notes.txt".into(), false),
            ]
        );
        assert!(list(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn names_for_new_files() {
        let dir = std::env::temp_dir().join(format!("seglinux-names-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Taken")).unwrap();
        assert_eq!(valid_name(&dir, " Photos "), Ok(dir.join("Photos")));
        assert!(valid_name(&dir, "").is_err());
        assert!(valid_name(&dir, "..").is_err());
        assert!(valid_name(&dir, "a/b").is_err());
        assert!(valid_name(&dir, "Taken").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_names_only_get_a_tooltip() {
        assert!(!name_is_cut("Install SegLinux"));
        assert!(!name_is_cut("Steam"));
        assert!(!name_is_cut("Holiday photos 2026"));
        assert!(name_is_cut("Quarterly report final version"));
        assert!(name_is_cut("averyveryverylongfilename.txt"));
        assert!(!name_is_cut("Installation123"), "15 characters: two lines");
        assert!(!name_is_cut(""));
    }

    #[test]
    fn launchers_are_copied_once_and_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("firefox.desktop");
        std::fs::write(&source, "[Desktop Entry]\nName=Firefox\n").unwrap();
        let desktop = dir.path().join("Desktop");
        let dest = copy_launcher(&source, &desktop).unwrap();
        assert_eq!(dest, desktop.join("firefox.desktop"));
        assert_eq!(
            std::fs::read_to_string(&dest).unwrap(),
            "[Desktop Entry]\nName=Firefox\n"
        );
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // A launcher already there, perhaps edited, is kept.
        std::fs::write(&dest, "edited").unwrap();
        assert_eq!(copy_launcher(&source, &desktop).unwrap(), dest);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "edited");
    }
}
