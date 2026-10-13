//! Desktop & dock: desktop icons, the dock's place and contents, the clock
//! and the pinned applications.
use super::widgets::{self, index_of, selected};
use super::{Page, State};
use crate::apps;
use crate::config::{CLOCK_PRESETS, DockPosition};
use gtk::{gio, glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

const POSITIONS: [DockPosition; 2] = [DockPosition::Bottom, DockPosition::Top];

pub struct Desktop {
    root: gtk::ScrolledWindow,
    icons: gtk::Switch,
    icon_size: gtk::SpinButton,
    position: gtk::DropDown,
    dock_size: gtk::SpinButton,
    show_desktop: gtk::Switch,
    battery: gtk::Switch,
    clock: gtk::Entry,
    pins: Rc<RefCell<Vec<String>>>,
    render_pins: Rc<dyn Fn()>,
}

/// Installed applications that can be pinned: desktop file id (without
/// `.desktop`) and name, by name.
fn installed_apps() -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| {
            let id = a.id()?.trim_end_matches(".desktop").to_string();
            Some((id, a.display_name().to_string()))
        })
        .collect();
    list.sort_by_key(|(_, name)| name.to_lowercase());
    list.dedup_by(|a, b| a.0 == b.0);
    list
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    button.add_css_class("flat");
    button
}

impl Desktop {
    pub fn new() -> Self {
        let (root, page) = widgets::page();

        let desk = widgets::section(&page, "Desktop", None);
        let icons = widgets::switch(
            &desk,
            "Desktop icons",
            Some("The files and folders of your Desktop folder, on the first screen."),
        );
        let icon_size = widgets::spin(
            &desk,
            "Desktop icon size",
            Some("In pixels."),
            (24.0, 96.0),
            4.0,
        );

        let dock = widgets::section(&page, "Dock", None);
        let position = widgets::dropdown(
            &dock,
            "Position",
            None,
            &["Bottom of the screen", "Top of the screen"],
        );
        let dock_size = widgets::spin(&dock, "Icon size", Some("In pixels."), (20.0, 64.0), 2.0);
        let show_desktop = widgets::switch(
            &dock,
            "Show desktop button",
            Some("Next to Start; Super+D does the same."),
        );
        let battery = widgets::switch(
            &dock,
            "Battery percentage",
            Some("Next to the battery icon, on laptops."),
        );

        let time = widgets::section(&page, "Clock", None);
        let presets: Vec<&str> = std::iter::once("Choose a format…")
            .chain(CLOCK_PRESETS.iter().map(|(label, _)| *label))
            .collect();
        let preset = widgets::dropdown(&time, "Format", None, &presets);
        let clock = widgets::entry(
            &time,
            "Custom format",
            Some("%H:%M is 24-hour time, %I:%M %p 12-hour time, %x the date, %a the weekday."),
            "%x  %H:%M",
        );
        clock.set_widget_name("clock");
        let sample = widgets::note("");
        sample.add_css_class("caption");
        sample.add_css_class("settings-row");
        time.append(&sample);
        let target = clock.clone();
        preset.connect_selected_notify(move |dropdown| {
            if let Some((_, format)) = (dropdown.selected() as usize)
                .checked_sub(1)
                .and_then(|i| CLOCK_PRESETS.get(i))
            {
                target.set_text(format);
            }
        });
        clock.connect_changed(move |entry| {
            let shown = glib::DateTime::now_local()
                .and_then(|now| now.format(&entry.text()))
                .map(|text| format!("The clock shows: {text}"))
                .unwrap_or_else(|_| "This format is not valid.".into());
            sample.set_text(&shown);
        });

        let pinned = widgets::section(
            &page,
            "Pinned applications",
            Some("Shown on the dock in this order, before other open applications."),
        );
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("pins");
        pinned.append(&list);
        let pins: Rc<RefCell<Vec<String>>> = Rc::default();
        // The list's buttons redraw the list: they reach the function
        // through this slot, filled once it exists.
        let slot: widgets::Redraw = Rc::default();
        let render_pins: Rc<dyn Fn()> = Rc::new({
            let list = list.downgrade();
            let pins = pins.clone();
            let slot = slot.clone();
            move || {
                let Some(list) = list.upgrade() else {
                    return;
                };
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                let ids = pins.borrow().clone();
                for (index, id) in ids.iter().enumerate() {
                    let app = apps::lookup(id);
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                    row.add_css_class("settings-row");
                    let image = gtk::Image::from_gicon(&apps::icon(app.as_ref()));
                    image.set_pixel_size(24);
                    row.append(&image);
                    let name = match &app {
                        Some(app) => app.display_name().to_string(),
                        None => format!("{id} (not installed)"),
                    };
                    let label = gtk::Label::new(Some(&name));
                    label.set_xalign(0.0);
                    label.set_hexpand(true);
                    row.append(&label);
                    for (icon, tooltip, step) in [
                        ("go-up-symbolic", "Move left", -1isize),
                        ("go-down-symbolic", "Move right", 1),
                        ("list-remove-symbolic", "Unpin", 0),
                    ] {
                        let button = icon_button(icon, &format!("{tooltip}: {name}"));
                        let target = index.checked_add_signed(step).filter(|t| *t < ids.len());
                        button.set_sensitive(step == 0 || target.is_some());
                        let (pins, slot) = (pins.clone(), slot.clone());
                        button.connect_clicked(move |_| {
                            {
                                let mut pins = pins.borrow_mut();
                                match target {
                                    _ if step == 0 => {
                                        pins.remove(index);
                                    }
                                    Some(t) => pins.swap(index, t),
                                    None => {}
                                }
                            }
                            widgets::redraw(&slot);
                        });
                        row.append(&button);
                    }
                    list.append(&row);
                }
                if ids.is_empty() {
                    list.append(&widgets::note("Nothing is pinned."));
                }
            }
        });
        *slot.borrow_mut() = Some(render_pins.clone());

        let available = installed_apps();
        let names: Vec<&str> = available.iter().map(|(_, name)| name.as_str()).collect();
        let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        add_row.add_css_class("settings-row");
        let choice = gtk::DropDown::from_strings(&names);
        choice.set_hexpand(true);
        choice.set_enable_search(true);
        choice.set_expression(Some(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<gtk::Expression>,
            "string",
        )));
        let add = gtk::Button::with_label("Pin");
        add.set_sensitive(!available.is_empty());
        add_row.append(&choice);
        add_row.append(&add);
        pinned.append(&add_row);
        {
            let (pins, render) = (pins.clone(), render_pins.clone());
            add.connect_clicked(move |_| {
                if let Some((id, _)) = available.get(choice.selected() as usize) {
                    let mut list = pins.borrow_mut();
                    if !list.contains(id) {
                        list.push(id.clone());
                    }
                } else {
                    return;
                }
                render();
            });
        }

        Self {
            root,
            icons,
            icon_size,
            position,
            dock_size,
            show_desktop,
            battery,
            clock,
            pins,
            render_pins,
        }
    }
}

impl Page for Desktop {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn load(&self, state: &State) {
        let c = &state.shell;
        self.icons.set_active(c.desktop_icons);
        self.icon_size.set_value(c.desktop_icon_size.into());
        self.position
            .set_selected(index_of(&POSITIONS, &c.dock_position));
        self.dock_size.set_value(c.dock_icon_size.into());
        self.show_desktop.set_active(c.show_desktop_button);
        self.battery.set_active(c.battery_percentage);
        self.clock.set_text(&c.clock_format);
        *self.pins.borrow_mut() = c.pinned.clone();
        (self.render_pins)();
    }

    fn store(&self, state: &mut State) -> Result<(), String> {
        let c = &mut state.shell;
        c.desktop_icons = self.icons.is_active();
        c.desktop_icon_size = widgets::spin_u32(&self.icon_size);
        c.dock_position = selected(&POSITIONS, &self.position);
        c.dock_icon_size = widgets::spin_u32(&self.dock_size);
        c.show_desktop_button = self.show_desktop.is_active();
        c.battery_percentage = self.battery.is_active();
        c.clock_format = self.clock.text().to_string();
        c.pinned = self.pins.borrow().clone();
        Ok(())
    }
}
