//! The Start menu: search and start applications, open settings, sign out,
//! restart or shut down, like Plasma's Kickoff.
use crate::common;
use crate::config;
use gtk::{gio, glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

pub fn menu(preview: bool) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("launcher");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_size_request(460, -1);

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
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let entries: Rc<RefCell<Vec<(gtk::ListBoxRow, String, String)>>> = Rc::default();
    let empty = gtk::Label::new(Some("No matching applications"));
    empty.set_visible(false);
    let empty_search = empty.clone();
    let filter_entries = entries.clone();
    search.connect_search_changed(move |search| {
        let query = search.text();
        let mut any = false;
        for (row, name, description) in filter_entries.borrow().iter() {
            let visible = config::matches_query(name, description, &query);
            row.set_visible(visible);
            any |= visible;
        }
        empty_search.set_visible(!any);
    });
    // The list and its icons are built the first time Start opens, not at
    // sign-in: less memory for a menu that may not be used.
    let populate = {
        let list = list.clone();
        let entries = entries.clone();
        let empty = empty.clone();
        move || {
            if !entries.borrow().is_empty() {
                return;
            }
            let mut apps = gio::AppInfo::all();
            apps.retain(|app| app.should_show());
            apps.sort_by_key(|a| a.display_name().to_lowercase());
            for app in apps {
                let row = gtk::ListBoxRow::new();
                let button = gtk::Button::new();
                button.add_css_class("app-row");
                let inner = gtk::Box::new(gtk::Orientation::Horizontal, 14);
                let icon = app
                    .icon()
                    .map(|i| gtk::Image::from_gicon(&i))
                    .unwrap_or_else(|| {
                        gtk::Image::from_icon_name("application-x-executable-symbolic")
                    });
                icon.set_pixel_size(28);
                inner.append(&icon);
                let text = gtk::Label::new(Some(&app.display_name()));
                text.set_xalign(0.0);
                text.set_ellipsize(gtk::pango::EllipsizeMode::End);
                inner.append(&text);
                button.set_child(Some(&inner));
                button.set_tooltip_text(app.description().as_deref());
                row.set_child(Some(&button));
                list.append(&row);
                entries.borrow_mut().push((
                    row,
                    app.display_name().to_string(),
                    app.description().unwrap_or_default().to_string(),
                ));
                button.connect_clicked(move |b| {
                    common::launch_app(&app);
                    common::close_popover(b);
                });
            }
            empty.set_visible(entries.borrow().is_empty());
        }
    };
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(380)
        .max_content_height(380)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    content.append(&scroll);
    content.append(&empty);

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
        "wincraft-shell",
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
