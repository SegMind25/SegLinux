//! Windows & input: window focus and spacing, workspaces, the keyboard,
//! mouse and touchpad. The compositor (labwc) applies them at once.
use super::widgets::{self, index_of, selected};
use super::{Page, State};
use crate::config::{LayoutSwitch, NumLock};
use gtk::prelude::*;

const SWITCHES: [LayoutSwitch; 4] = [
    LayoutSwitch::AltShift,
    LayoutSwitch::CtrlShift,
    LayoutSwitch::CapsLock,
    LayoutSwitch::None,
];
const NUMLOCK: [NumLock; 3] = [NumLock::Unchanged, NumLock::On, NumLock::Off];

/// Keyboard layouts XKB knows: code and name, from the `! layout` part of
/// `base.lst`.
pub fn parse_xkb_layouts(text: &str) -> Vec<(String, String)> {
    text.lines()
        .skip_while(|line| line.trim() != "! layout")
        .skip(1)
        .take_while(|line| !line.starts_with('!'))
        .filter_map(|line| {
            let line = line.trim();
            let (code, name) = line.split_once(char::is_whitespace)?;
            Some((code.to_string(), name.trim().to_string()))
        })
        .collect()
}

pub struct Windows {
    root: gtk::ScrolledWindow,
    follow: gtk::Switch,
    raise: gtk::Switch,
    gap: gtk::SpinButton,
    radius: gtk::SpinButton,
    workspaces: gtk::SpinButton,
    layout: gtk::Entry,
    layout_switch: gtk::DropDown,
    rate: gtk::SpinButton,
    delay: gtk::SpinButton,
    numlock: gtk::DropDown,
    speed: gtk::Scale,
    acceleration: gtk::Switch,
    mouse_natural: gtk::Switch,
    left_handed: gtk::Switch,
    cursor: gtk::SpinButton,
    tap: gtk::Switch,
    pad_natural: gtk::Switch,
    typing: gtk::Switch,
}

impl Windows {
    pub fn new() -> Self {
        let (root, page) = widgets::page();
        page.append(&widgets::note("These settings are applied by the SegLinux desktop session as soon as you select Apply. The Plasma session keeps its own."));

        let win = widgets::section(&page, "Windows", None);
        let follow = widgets::switch(
            &win,
            "Focus follows the mouse",
            Some("The window under the pointer receives the keyboard."),
        );
        let raise = widgets::switch(&win, "Bring focused windows to the front", None);
        let gap = widgets::spin(
            &win,
            "Gap around snapped windows",
            Some("In pixels."),
            (0.0, 48.0),
            1.0,
        );
        let radius = widgets::spin(
            &win,
            "Title bar roundness",
            Some("In pixels; 0 for square corners."),
            (0.0, 20.0),
            1.0,
        );
        let workspaces = widgets::spin(
            &win,
            "Workspaces",
            Some(
                "Ctrl+Super+Left/Right switches between them; add Shift to take the active window along.",
            ),
            (1.0, 12.0),
            1.0,
        );
        workspaces.set_widget_name("workspaces");

        let keys = widgets::section(&page, "Keyboard", None);
        let layout = widgets::entry(
            &keys,
            "Layouts",
            Some(
                "XKB layout codes separated by commas, such as fr or us,ara. Empty: the layout chosen during Setup.",
            ),
            "Layout chosen in Setup",
        );
        layout.set_widget_name("keyboard-layout");
        let known = std::fs::read_to_string("/usr/share/X11/xkb/rules/base.lst")
            .map(|text| parse_xkb_layouts(&text))
            .unwrap_or_default();
        if !known.is_empty() {
            let labels: Vec<String> = known
                .iter()
                .map(|(code, name)| format!("{name} ({code})"))
                .collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            add_row.add_css_class("settings-row");
            let choice = gtk::DropDown::from_strings(&labels);
            choice.set_hexpand(true);
            choice.set_enable_search(true);
            choice.set_expression(Some(gtk::PropertyExpression::new(
                gtk::StringObject::static_type(),
                None::<gtk::Expression>,
                "string",
            )));
            let add = gtk::Button::with_label("Add layout");
            add_row.append(&choice);
            add_row.append(&add);
            let target = layout.clone();
            add.connect_clicked(move |_| {
                if let Some((code, _)) = known.get(choice.selected() as usize) {
                    let current = target.text();
                    let mut codes: Vec<&str> = current
                        .split(',')
                        .map(str::trim)
                        .filter(|c| !c.is_empty())
                        .collect();
                    if !codes.contains(&code.as_str()) {
                        codes.push(code);
                    }
                    target.set_text(&codes.join(","));
                }
            });
            keys.append(&add_row);
        }
        let layout_switch = widgets::dropdown(
            &keys,
            "Switch layouts with",
            Some("When there are several layouts."),
            &["Alt+Shift", "Ctrl+Shift", "Caps Lock", "No shortcut"],
        );
        let rate = widgets::spin(
            &keys,
            "Repeat rate",
            Some("Characters per second while a key is held."),
            (1.0, 100.0),
            1.0,
        );
        let delay = widgets::spin(
            &keys,
            "Repeat delay",
            Some("Milliseconds before a held key repeats."),
            (100.0, 2000.0),
            50.0,
        );
        let numlock = widgets::dropdown(
            &keys,
            "Num Lock at sign-in",
            None,
            &["Leave as it is", "On", "Off"],
        );

        let mouse = widgets::section(&page, "Mouse", None);
        let speed = widgets::scale(
            &mouse,
            "Pointer speed",
            Some("From slowest (-100) to fastest (100)."),
            (-100.0, 100.0),
            5.0,
        );
        speed.add_mark(0.0, gtk::PositionType::Bottom, None);
        let acceleration = widgets::switch(
            &mouse,
            "Pointer acceleration",
            Some(
                "Off moves the pointer exactly as far as the mouse, which many players prefer for aiming.",
            ),
        );
        let mouse_natural = widgets::switch(
            &mouse,
            "Natural scrolling",
            Some("The content follows the wheel, as on a phone."),
        );
        let left_handed = widgets::switch(&mouse, "Left-handed buttons", None);
        let cursor = widgets::spin(
            &mouse,
            "Pointer size",
            Some("In pixels; applications started afterwards use it."),
            (16.0, 96.0),
            8.0,
        );

        let pad = widgets::section(&page, "Touchpad", None);
        let tap = widgets::switch(&pad, "Tap to click", None);
        let pad_natural = widgets::switch(
            &pad,
            "Natural scrolling",
            Some("Two fingers move the content, as on a phone."),
        );
        let typing = widgets::switch(&pad, "Disable while typing", None);

        Self {
            root,
            follow,
            raise,
            gap,
            radius,
            workspaces,
            layout,
            layout_switch,
            rate,
            delay,
            numlock,
            speed,
            acceleration,
            mouse_natural,
            left_handed,
            cursor,
            tap,
            pad_natural,
            typing,
        }
    }
}

impl Page for Windows {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn load(&self, state: &State) {
        let w = &state.shell.windows;
        self.follow.set_active(w.focus_follows_mouse);
        self.raise.set_active(w.raise_on_focus);
        self.gap.set_value(w.gap.into());
        self.radius.set_value(w.corner_radius.into());
        self.workspaces.set_value(w.workspaces.into());
        let i = &state.shell.input;
        self.layout.set_text(&i.keyboard_layout);
        self.layout_switch
            .set_selected(index_of(&SWITCHES, &i.layout_switch));
        self.rate.set_value(i.repeat_rate.into());
        self.delay.set_value(i.repeat_delay.into());
        self.numlock.set_selected(index_of(&NUMLOCK, &i.numlock));
        self.speed.set_value(i.pointer_speed.into());
        self.acceleration.set_active(i.mouse_acceleration);
        self.mouse_natural.set_active(i.natural_scroll_mouse);
        self.left_handed.set_active(i.left_handed);
        self.cursor.set_value(i.cursor_size.into());
        self.tap.set_active(i.tap_to_click);
        self.pad_natural.set_active(i.natural_scroll_touchpad);
        self.typing.set_active(i.disable_touchpad_while_typing);
    }

    fn store(&self, state: &mut State) -> Result<(), String> {
        let w = &mut state.shell.windows;
        w.focus_follows_mouse = self.follow.is_active();
        w.raise_on_focus = self.raise.is_active();
        w.gap = widgets::spin_u32(&self.gap);
        w.corner_radius = widgets::spin_u32(&self.radius);
        w.workspaces = widgets::spin_u32(&self.workspaces);
        let i = &mut state.shell.input;
        i.keyboard_layout = self
            .layout
            .text()
            .split(',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        i.layout_switch = selected(&SWITCHES, &self.layout_switch);
        i.repeat_rate = widgets::spin_u32(&self.rate);
        i.repeat_delay = widgets::spin_u32(&self.delay);
        i.numlock = selected(&NUMLOCK, &self.numlock);
        i.pointer_speed = self.speed.value().round() as i32;
        i.mouse_acceleration = self.acceleration.is_active();
        i.natural_scroll_mouse = self.mouse_natural.is_active();
        i.left_handed = self.left_handed.is_active();
        i.cursor_size = widgets::spin_u32(&self.cursor);
        i.tap_to_click = self.tap.is_active();
        i.natural_scroll_touchpad = self.pad_natural.is_active();
        i.disable_touchpad_while_typing = self.typing.is_active();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_xkb_layout_names() {
        let text = "! model\n  pc105  Generic 105-key PC\n\n! layout\n  us              English (US)\n  ara             Arabic\n  fr              French\n\n! variant\n  intl  us: English (intl)\n";
        let layouts = super::parse_xkb_layouts(text);
        assert_eq!(
            layouts,
            [
                ("us".to_string(), "English (US)".to_string()),
                ("ara".into(), "Arabic".into()),
                ("fr".into(), "French".into()),
            ]
        );
    }
}
