//! The Start menu, like Plasma's Kickoff: favorites (the dock's pins), all
//! applications and their categories, search, and a menu for each
//! application (its own actions, pin to the dock, add to the desktop), then
//! settings, sign out, restart and shut down. The desktop is rebuilt when
//! applications are installed or removed (see main.rs), so the list follows,
//! and applications installed since the menu first listed them are marked
//! "New" until they are opened.
use crate::catalog::{self, Category, Known};
use crate::common;
use crate::config;
use crate::desktop;
use crate::pins::{self, Change};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

/// What the filter and the sort need to know about one application.
struct Entry {
    name: String,
    /// The description and keywords, for search.
    details: String,
    categories: Vec<Category>,
    /// Kept out of menus (NoDisplay), like the Terminal launcher: listed
    /// only in Favorites, when it is pinned.
    hidden: bool,
}

/// The menu's state, shared by its callbacks.
struct State {
    /// By desktop file id, which is also each row's widget name.
    entries: RefCell<HashMap<String, Entry>>,
    pinned: Vec<String>,
    category: Cell<Category>,
    query: RefCell<String>,
}

impl State {
    fn shows(&self, id: &str, entry: &Entry) -> bool {
        let query = self.query.borrow();
        if !query.trim().is_empty() {
            return !entry.hidden && config::matches_query(&entry.name, &entry.details, &query);
        }
        match self.category.get() {
            Category::Favorites => self.pinned.iter().any(|p| p == id),
            _ if entry.hidden => false,
            Category::All => true,
            category => entry.categories.contains(&category),
        }
    }

    fn any_shown(&self) -> bool {
        self.entries
            .borrow()
            .iter()
            .any(|(id, entry)| self.shows(id, entry))
    }
}

pub fn menu(preview: bool, pinned: &[String]) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("launcher");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_size_request(620, -1);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let avatar = gtk::Image::from_icon_name(&common::themed_icon(&[
        "user-identity",
        "avatar-default-symbolic",
    ]));
    avatar.set_pixel_size(36);
    header.append(&avatar);
    let name = glib::real_name().to_string_lossy().into_owned();
    let user = if name.is_empty() || name == "Unknown" {
        glib::user_name().to_string_lossy().into_owned()
    } else {
        name
    };
    let heading = gtk::Label::new(Some(&user));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    heading.set_hexpand(true);
    header.append(&heading);
    content.append(&header);

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search applications…")
        .build();
    content.append(&search);

    let state = Rc::new(State {
        entries: RefCell::default(),
        pinned: pinned.to_vec(),
        category: Cell::new(Category::Favorites),
        query: RefCell::default(),
    });

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("launcher-categories");
    sidebar.set_selection_mode(gtk::SelectionMode::Single);
    sidebar.set_size_request(170, -1);
    body.append(&sidebar);

    let list = gtk::ListBox::new();
    list.add_css_class("launcher-apps");
    list.set_selection_mode(gtk::SelectionMode::None);
    let filter_state = state.clone();
    list.set_filter_func(move |row| {
        let id = row.widget_name();
        filter_state
            .entries
            .borrow()
            .get(id.as_str())
            .is_none_or(|entry| filter_state.shows(&id, entry))
    });
    let sort_state = state.clone();
    list.set_sort_func(move |a, b| {
        let (a, b) = (a.widget_name(), b.widget_name());
        let order = if sort_state.category.get() == Category::Favorites
            && sort_state.query.borrow().trim().is_empty()
        {
            // Favorites in the dock's order.
            let place = |id: &str| sort_state.pinned.iter().position(|p| p == id);
            place(&a).cmp(&place(&b))
        } else {
            let entries = sort_state.entries.borrow();
            let name = |id: &str| entries.get(id).map(|e| e.name.to_lowercase());
            name(&a).cmp(&name(&b))
        };
        order.into()
    });

    let apps = gtk::Box::new(gtk::Orientation::Vertical, 0);
    apps.set_hexpand(true);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(380)
        .max_content_height(380)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .vexpand(true)
        .build();
    apps.append(&scroll);
    let empty = gtk::Label::new(None);
    empty.add_css_class("hint");
    empty.set_wrap(true);
    empty.set_visible(false);
    apps.append(&empty);
    body.append(&apps);
    content.append(&body);

    // After any change of category or search.
    let refresh = {
        let state = state.clone();
        let list = list.downgrade();
        let scroll = scroll.downgrade();
        let empty = empty.downgrade();
        move || {
            let (Some(list), Some(scroll), Some(empty)) =
                (list.upgrade(), scroll.upgrade(), empty.upgrade())
            else {
                return;
            };
            list.invalidate_filter();
            list.invalidate_sort();
            let any = state.any_shown();
            scroll.set_visible(any);
            empty.set_visible(!any);
            empty.set_text(
                if state.category.get() == Category::Favorites
                    && state.query.borrow().trim().is_empty()
                {
                    "Applications pinned to the dock show here. Right-click an application to pin it."
                } else {
                    "No matching applications"
                },
            );
            scroll.vadjustment().set_value(0.0);
        }
    };
    let refresh: Rc<dyn Fn()> = Rc::new(refresh);

    let search_state = state.clone();
    let search_refresh = refresh.clone();
    let search_sidebar = sidebar.downgrade();
    search.connect_search_changed(move |search| {
        let query = search.text().to_string();
        let searching = !query.trim().is_empty();
        *search_state.query.borrow_mut() = query;
        // Results come from every category: none is highlighted meanwhile.
        if let Some(sidebar) = search_sidebar.upgrade() {
            if searching {
                sidebar.unselect_all();
            } else {
                select_category(&sidebar, search_state.category.get());
            }
        }
        search_refresh();
    });
    // Enter starts the first application shown, as in Kickoff.
    let first_list = list.downgrade();
    search.connect_activate(move |_| {
        if let Some(button) = first_list
            .upgrade()
            .and_then(|l| shown_rows(&l).into_iter().next())
        {
            button.emit_clicked();
        }
    });

    let sidebar_state = state.clone();
    let sidebar_refresh = refresh.clone();
    let sidebar_search = search.downgrade();
    sidebar.connect_row_selected(move |_, row| {
        let Some(row) = row else {
            return;
        };
        if let Some(category) = Category::ALL
            .into_iter()
            .find(|c| c.label() == row.widget_name().as_str())
        {
            sidebar_state.category.set(category);
            // Choosing a category ends a search.
            let searching = !sidebar_state.query.borrow().trim().is_empty();
            if searching && let Some(search) = sidebar_search.upgrade() {
                search.set_text("");
            }
            sidebar_refresh();
        }
    });

    // The list and its icons are built the first time Start opens, not at
    // sign-in: less memory for a menu that may not be used.
    let populate = {
        let list = list.downgrade();
        let sidebar = sidebar.downgrade();
        let popover = popover.downgrade();
        let state = state.clone();
        let refresh = refresh.clone();
        move || {
            let (Some(list), Some(sidebar), Some(popover)) =
                (list.upgrade(), sidebar.upgrade(), popover.upgrade())
            else {
                return;
            };
            if !state.entries.borrow().is_empty() {
                return;
            }
            let mut apps = gio::AppInfo::all();
            // Every installed application counts, hidden ones included.
            let mut known = Known::load(&Known::default_path());
            let ids: Vec<String> = apps.iter().filter_map(pins::desktop_id).collect();
            known.note_installed(ids.iter().map(String::as_str));
            let known = Rc::new(RefCell::new(known));
            let pinned = |app: &gio::AppInfo| {
                pins::desktop_id(app).is_some_and(|id| state.pinned.contains(&id))
            };
            apps.retain(|app| app.should_show() || pinned(app));
            for app in apps {
                let Some(id) = pins::desktop_id(&app) else {
                    continue;
                };
                let desktop_app = app.downcast_ref::<gio::DesktopAppInfo>().cloned();
                let categories = catalog::categories(
                    &desktop_app
                        .as_ref()
                        .and_then(|a| a.categories())
                        .unwrap_or_default(),
                );
                let keywords = desktop_app
                    .as_ref()
                    .map(|a| {
                        a.keywords()
                            .iter()
                            .map(|k| k.to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                let row = app_row(&app, desktop_app, &id, &state, &known, &popover);
                list.append(&row);
                state.entries.borrow_mut().insert(
                    id,
                    Entry {
                        name: app.display_name().to_string(),
                        details: format!("{} {keywords}", app.description().unwrap_or_default()),
                        categories,
                        hidden: !app.should_show(),
                    },
                );
            }
            // Favorites, all applications, then the categories in use.
            let entries = state.entries.borrow();
            for category in Category::ALL {
                let used = match category {
                    Category::Favorites | Category::All => true,
                    category => entries
                        .values()
                        .any(|e| !e.hidden && e.categories.contains(&category)),
                };
                if used {
                    sidebar.append(&category_row(category));
                }
            }
            drop(entries);
            if !state
                .pinned
                .iter()
                .any(|p| state.entries.borrow().contains_key(p))
            {
                state.category.set(Category::All);
            }
            select_category(&sidebar, state.category.get());
            refresh();
        }
    };

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    footer.add_css_class("footer");
    if common::installed("dolphin") {
        footer.append(&common::command_button(
            "Files",
            "folder-symbolic",
            "dolphin",
            &[],
        ));
    }
    footer.append(&common::command_button(
        "Settings",
        "preferences-system-symbolic",
        "seglinux-shell",
        &["--settings"],
    ));
    let fill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    fill.set_hexpand(true);
    footer.append(&fill);
    footer.append(&common::power_buttons(preview));
    content.append(&footer);

    popover.set_child(Some(&content));
    popover.connect_show(move |_| {
        populate();
        search.set_text("");
        search.grab_focus();
    });
    popover
}

/// The buttons of the applications the list shows, in their order.
fn shown_rows(list: &gtk::ListBox) -> Vec<gtk::Button> {
    // By index: the list's sorted order, which its widgets do not follow.
    (0..)
        .map_while(|i| list.row_at_index(i))
        .filter(|row| row.is_child_visible())
        .filter_map(|row| row.child().and_then(|b| b.downcast::<gtk::Button>().ok()))
        .collect()
}

fn category_row(category: Category) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(category.label());
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = gtk::Image::from_icon_name(&common::themed_icon(category.icons()));
    icon.set_pixel_size(20);
    inner.append(&icon);
    let label = gtk::Label::new(Some(category.label()));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    inner.append(&label);
    row.set_child(Some(&inner));
    row
}

fn select_category(sidebar: &gtk::ListBox, category: Category) {
    let mut child = sidebar.first_child();
    while let Some(widget) = child {
        if let Ok(row) = widget.clone().downcast::<gtk::ListBoxRow>()
            && row.widget_name().as_str() == category.label()
        {
            sidebar.select_row(Some(&row));
            return;
        }
        child = widget.next_sibling();
    }
}

/// One application: starts it on a click; a right click (or a long press)
/// opens its menu.
fn app_row(
    app: &gio::AppInfo,
    desktop_app: Option<gio::DesktopAppInfo>,
    id: &str,
    state: &Rc<State>,
    known: &Rc<RefCell<Known>>,
    start: &gtk::Popover,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_widget_name(id);
    let button = gtk::Button::new();
    button.add_css_class("app-row");
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let icon = app
        .icon()
        .map(|i| gtk::Image::from_gicon(&i))
        .unwrap_or_else(|| gtk::Image::from_icon_name("application-x-executable-symbolic"));
    icon.set_pixel_size(28);
    inner.append(&icon);
    let text = gtk::Label::new(Some(&app.display_name()));
    text.set_xalign(0.0);
    text.set_hexpand(true);
    text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    inner.append(&text);
    let badge = gtk::Label::new(Some("New"));
    badge.add_css_class("new-badge");
    badge.set_valign(gtk::Align::Center);
    badge.set_visible(app.should_show() && known.borrow().is_new(id));
    inner.append(&badge);
    button.set_child(Some(&inner));
    button.set_tooltip_text(app.description().as_deref());
    row.set_child(Some(&button));

    let launch_app = app.clone();
    let launch_id = id.to_string();
    let launch_known = known.clone();
    button.connect_clicked(move |b| {
        common::launch_app(&launch_app);
        launch_known.borrow_mut().opened(&launch_id);
        badge.set_visible(false);
        common::close_popover(b);
    });

    let menu_app = app.clone();
    let menu_id = id.to_string();
    let pinned = state.pinned.iter().any(|p| p == id);
    let start = start.downgrade();
    common::context_menu(&button, move || {
        app_menu(&menu_app, desktop_app.clone(), &menu_id, pinned, &start)
    });
    row
}

/// An application's menu: its own actions, then pin or unpin, then add to
/// the desktop.
fn app_menu(
    app: &gio::AppInfo,
    desktop_app: Option<gio::DesktopAppInfo>,
    id: &str,
    pinned: bool,
    start: &glib::WeakRef<gtk::Popover>,
) -> gtk::Popover {
    let menu = gtk::Popover::new();
    menu.add_css_class("menu");
    let items = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let title = gtk::Label::new(Some(&app.display_name()));
    title.add_css_class("menu-title");
    title.set_xalign(0.0);
    items.append(&title);
    if let Some(desktop_app) = &desktop_app {
        let actions = crate::apps::action_buttons(desktop_app);
        if !actions.is_empty() {
            for button in actions {
                close_start_too(&button, start);
                items.append(&button);
            }
            items.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
    }
    let pin = common::menu_item(if pinned {
        "Unpin from dock"
    } else {
        "Pin to dock"
    });
    let pin_id = id.to_string();
    pin.connect_clicked(move |b| {
        common::close_popover(b);
        pins::update(&pin_id, if pinned { Change::Unpin } else { Change::Pin });
    });
    items.append(&pin);
    if let Some(desktop_app) = desktop_app {
        let add = common::menu_item("Add to desktop");
        add.connect_clicked(move |b| {
            if let Err(error) = desktop::add_launcher(&desktop_app) {
                common::show_error(&error);
            }
            common::close_popover(b);
        });
        close_start_too(&add, start);
        items.append(&add);
    }
    items.set_width_request(200);
    menu.set_child(Some(&items));
    menu
}

/// After `button`'s own action, Start closes as well.
fn close_start_too(button: &gtk::Button, start: &glib::WeakRef<gtk::Popover>) {
    let start = start.clone();
    button.connect_clicked(move |_| {
        if let Some(start) = start.upgrade() {
            start.popdown();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut all = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(current) = child {
            all.extend(descendants(&current));
            child = current.next_sibling();
        }
        all
    }

    fn list_with_class(widgets: &[gtk::Widget], class: &str) -> gtk::ListBox {
        widgets
            .iter()
            .filter_map(|w| w.clone().downcast::<gtk::ListBox>().ok())
            .find(|l| l.has_css_class(class))
            .unwrap_or_else(|| panic!("no list {class}"))
    }

    /// The test applications the list shows, in order.
    fn shown(list: &gtk::ListBox) -> Vec<String> {
        (0..)
            .map_while(|i| list.row_at_index(i))
            .filter(|row| row.is_child_visible())
            .map(|row| row.widget_name().to_string())
            .filter(|id| id.starts_with("org.example."))
            .collect()
    }

    fn settle() {
        let context = glib::MainContext::default();
        for _ in 0..40 {
            while context.iteration(false) {}
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn write_app(id: &str, lines: &str) {
        let dir = seglinux_core::xdg::applications_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.desktop")),
            format!("[Desktop Entry]\nType=Application\nExec=true\n{lines}\n"),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "requires isolated XDG config, data and state directories and a GTK display"]
    fn start_menu_favorites_categories_search_and_new_applications() {
        let root =
            std::env::var("SEGLINUX_SETTINGS_TEST_ROOT").expect("isolated test directory required");
        for path in [
            Config::path(),
            seglinux_core::xdg::applications_dir(),
            Known::default_path(),
        ] {
            assert!(
                path.starts_with(&root),
                "{} is outside the test directory",
                path.display()
            );
        }
        write_app(
            "org.example.Paint",
            "Name=Example Paint\nCategories=Graphics;",
        );
        write_app("org.example.Zebra", "Name=Zebra Game\nCategories=Game;");
        write_app(
            "org.example.Hidden",
            "Name=Hidden Terminal\nNoDisplay=true\nCategories=System;",
        );
        // An earlier session knew Zebra and Hidden: Paint was installed since.
        let known = Known::default_path();
        std::fs::create_dir_all(known.parent().unwrap()).unwrap();
        std::fs::write(&known, "org.example.Zebra\norg.example.Hidden\n").unwrap();

        gtk::init().unwrap();
        let pinned = [
            "org.example.Zebra",
            "org.example.Hidden",
            "org.example.Paint",
        ]
        .map(String::from);
        let start = menu(true, &pinned);
        let window = gtk::Window::new();
        let button = gtk::MenuButton::new();
        button.set_popover(Some(&start));
        window.set_child(Some(&button));
        window.present();
        start.popup();
        settle();

        let widgets = descendants(start.upcast_ref());
        let list = list_with_class(&widgets, "launcher-apps");
        let sidebar = list_with_class(&widgets, "launcher-categories");
        // Favorites: the pins in the dock's order, the hidden one included.
        assert_eq!(
            shown(&list),
            [
                "org.example.Zebra",
                "org.example.Hidden",
                "org.example.Paint"
            ]
        );
        // Only Paint is new.
        let badge = |id: &str| {
            widgets
                .iter()
                .filter(|w| w.widget_name() == id)
                .flat_map(descendants)
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .find(|l| l.has_css_class("new-badge"))
                .unwrap()
                .is_visible()
        };
        assert!(badge("org.example.Paint"));
        assert!(!badge("org.example.Zebra"));

        // Categories, then all applications: by name, the hidden one left out.
        let select = |label: &str| {
            let row = (0..)
                .map_while(|i| sidebar.row_at_index(i))
                .find(|r| r.widget_name() == label)
                .unwrap_or_else(|| panic!("no category {label}"));
            sidebar.select_row(Some(&row));
            settle();
        };
        select("Games");
        assert_eq!(shown(&list), ["org.example.Zebra"]);
        select("Graphics");
        assert_eq!(shown(&list), ["org.example.Paint"]);
        select("All applications");
        assert_eq!(shown(&list), ["org.example.Paint", "org.example.Zebra"]);

        // A search covers every category and highlights none; Enter starts
        // the first result, which is then no longer new.
        let search = widgets
            .iter()
            .find_map(|w| w.clone().downcast::<gtk::SearchEntry>().ok())
            .unwrap();
        search.set_text("paint");
        settle();
        assert_eq!(shown(&list), ["org.example.Paint"]);
        assert!(sidebar.selected_row().is_none());
        search.emit_activate();
        settle();
        assert!(!badge("org.example.Paint"));
        assert!(
            std::fs::read_to_string(&known)
                .unwrap()
                .lines()
                .any(|l| l == "org.example.Paint")
        );
        window.close();

        // Pins are saved in shell.toml, keeping the other preferences.
        let mut config = Config {
            pinned: pinned.to_vec(),
            dock_icon_size: 40,
            ..Config::default()
        };
        crate::settings::save_text(&Config::path(), &config.to_toml()).unwrap();
        pins::update("org.example.Paint", Change::Unpin);
        pins::update(
            "org.example.Hidden",
            Change::TakePlaceOf("org.example.Zebra".into()),
        );
        pins::update("org.example.Paint", Change::Pin);
        let saved = Config::try_load().unwrap();
        config.pinned = [
            "org.example.Hidden",
            "org.example.Zebra",
            "org.example.Paint",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(saved, config);
        std::fs::remove_file(Config::path()).unwrap();
    }
}
