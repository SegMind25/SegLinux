//! Building blocks of the Settings pages: titled sections of rows, each a
//! label (with an optional explanation) and a control on the right.
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc};

/// A list's redraw function, reachable from the buttons it creates.
pub type Redraw = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// Calls the redraw function in `slot`, if it is set.
pub fn redraw(slot: &Redraw) {
    let render = slot.borrow().clone();
    if let Some(render) = render {
        render();
    }
}

/// A page: a scrolled column of sections.
pub fn page() -> (gtk::ScrolledWindow, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.add_css_class("settings-page");
    content.set_margin_top(18);
    content.set_margin_bottom(24);
    content.set_margin_start(28);
    content.set_margin_end(28);
    let clamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.set_hexpand(true);
    content.set_size_request(520, -1);
    clamp.append(&content);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .hexpand(true)
        .child(&clamp)
        .build();
    (scroll, content)
}

pub fn note(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.add_css_class("dim-label");
    label
}

/// A titled group of rows.
pub fn section(page: &gtk::Box, title: &str, description: Option<&str>) -> gtk::Box {
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("section-title");
    heading.set_xalign(0.0);
    heading.set_margin_top(14);
    page.append(&heading);
    if let Some(text) = description {
        page.append(&note(text));
    }
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rows.add_css_class("settings-section");
    page.append(&rows);
    rows
}

/// A row: title and explanation on the left, `control` on the right.
pub fn row(
    section: &gtk::Box,
    title: &str,
    subtitle: Option<&str>,
    control: &impl IsA<gtk::Widget>,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_wrap(true);
    text.append(&label);
    if let Some(subtitle) = subtitle {
        let sub = note(subtitle);
        sub.add_css_class("caption");
        text.append(&sub);
    }
    row.append(&text);
    control.set_valign(gtk::Align::Center);
    row.append(control);
    // The label names the control for screen readers.
    control
        .upcast_ref::<gtk::Widget>()
        .update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
    section.append(&row);
}

pub fn switch(section: &gtk::Box, title: &str, subtitle: Option<&str>) -> gtk::Switch {
    let widget = gtk::Switch::new();
    row(section, title, subtitle, &widget);
    widget
}

pub fn spin(
    section: &gtk::Box,
    title: &str,
    subtitle: Option<&str>,
    range: (f64, f64),
    step: f64,
) -> gtk::SpinButton {
    let widget = gtk::SpinButton::with_range(range.0, range.1, step);
    widget.set_numeric(true);
    row(section, title, subtitle, &widget);
    widget
}

pub fn dropdown(
    section: &gtk::Box,
    title: &str,
    subtitle: Option<&str>,
    items: &[&str],
) -> gtk::DropDown {
    let widget = gtk::DropDown::from_strings(items);
    row(section, title, subtitle, &widget);
    widget
}

pub fn entry(
    section: &gtk::Box,
    title: &str,
    subtitle: Option<&str>,
    placeholder: &str,
) -> gtk::Entry {
    let widget = gtk::Entry::new();
    widget.set_placeholder_text(Some(placeholder));
    widget.set_width_chars(22);
    row(section, title, subtitle, &widget);
    widget
}

pub fn scale(
    section: &gtk::Box,
    title: &str,
    subtitle: Option<&str>,
    range: (f64, f64),
    step: f64,
) -> gtk::Scale {
    let widget = gtk::Scale::with_range(gtk::Orientation::Horizontal, range.0, range.1, step);
    widget.set_size_request(220, -1);
    widget.set_draw_value(true);
    widget.set_digits(0);
    row(section, title, subtitle, &widget);
    widget
}

/// Width and height side by side, 0 meaning "automatic".
pub fn size(
    section: &gtk::Box,
    title: &str,
    subtitle: Option<&str>,
) -> (gtk::SpinButton, gtk::SpinButton) {
    let pair = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let width = gtk::SpinButton::with_range(0.0, 16384.0, 1.0);
    let height = gtk::SpinButton::with_range(0.0, 16384.0, 1.0);
    for (spin, name) in [(&width, "Width"), (&height, "Height")] {
        spin.set_numeric(true);
        spin.set_tooltip_text(Some(name));
    }
    pair.append(&width);
    pair.append(&gtk::Label::new(Some("×")));
    pair.append(&height);
    row(section, title, subtitle, &pair);
    (width, height)
}

/// A text area of a few lines.
pub fn text_area(section: &gtk::Box, title: &str, subtitle: Option<&str>) -> gtk::TextView {
    let heading = gtk::Box::new(gtk::Orientation::Vertical, 6);
    heading.add_css_class("settings-row");
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    heading.append(&label);
    if let Some(subtitle) = subtitle {
        let sub = note(subtitle);
        sub.add_css_class("caption");
        heading.append(&sub);
    }
    let view = gtk::TextView::new();
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.set_left_margin(8);
    view.set_right_margin(8);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(96)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&view)
        .build();
    scroll.add_css_class("text-area");
    heading.append(&scroll);
    view.update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
    section.append(&heading);
    view
}

pub fn text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

pub fn spin_u32(spin: &gtk::SpinButton) -> u32 {
    spin.value_as_int().max(0) as u32
}

/// The index of `value` in `values`, for a dropdown's selection.
pub fn index_of<T: PartialEq>(values: &[T], value: &T) -> u32 {
    values.iter().position(|v| v == value).unwrap_or(0) as u32
}

/// The value at a dropdown's selection.
pub fn selected<T: Copy>(values: &[T], dropdown: &gtk::DropDown) -> T {
    values[(dropdown.selected() as usize).min(values.len() - 1)]
}
