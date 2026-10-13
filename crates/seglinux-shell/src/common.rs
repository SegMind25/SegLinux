//! Small helpers shared by the desktop, dock and menus.
use gtk::{gdk, gio, glib, prelude::*};
use std::{cell::OnceCell, rc::Rc};

/// Opens whichever terminal emulator is installed (see data/system/seglinux-terminal).
pub const TERMINAL: &str = "seglinux-terminal";

pub fn installed(command: &str) -> bool {
    glib::find_program_in_path(command).is_some()
}

pub fn run(command: &str, args: &[String]) {
    let mut argv = vec![std::ffi::OsStr::new(command)];
    argv.extend(args.iter().map(std::ffi::OsStr::new));
    // GSubprocess reaps children; arguments never pass through a shell.
    if let Err(error) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) {
        show_error(&format!("Cannot start {command}: {error}"));
    }
}

pub fn launch_context() -> Option<gdk::AppLaunchContext> {
    gdk::Display::default().map(|d| d.app_launch_context())
}

pub fn launch_app(app: &impl IsA<gio::AppInfo>) {
    if let Err(error) = app.launch(&[], launch_context().as_ref()) {
        show_error(&format!("Cannot start {}: {error}", app.display_name()));
    }
}

/// Opens a file or folder with its default application, like a double
/// click in a file manager. Desktop entries start their application.
pub fn open_path(path: &std::path::Path) {
    if path.extension().is_some_and(|e| e == "desktop")
        && let Some(app) = gio::DesktopAppInfo::from_filename(path)
    {
        launch_app(&app);
        return;
    }
    let uri = gio::File::for_path(path).uri();
    if let Err(error) = gio::AppInfo::launch_default_for_uri(&uri, launch_context().as_ref()) {
        show_error(&format!("Cannot open {}: {error}", path.display()));
    }
}

/// Closes the popover a widget sits in, if any.
pub fn close_popover(widget: &impl IsA<gtk::Widget>) {
    if let Some(popover) = widget
        .ancestor(gtk::Popover::static_type())
        .and_then(|w| w.downcast::<gtk::Popover>().ok())
    {
        popover.popdown();
    }
}

/// An item of a right-click menu: a flat button with its label at the
/// start, as in Plasma's menus.
pub fn menu_item(label: &str) -> gtk::Button {
    let button = gtk::Button::new();
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    button.set_child(Some(&text));
    button
}

/// Opens the menu `build` makes on a right click or a long press on
/// `widget`. The menu is made the first time it opens: a list of a hundred
/// applications does not hold a hundred menus nobody opened. The widget's
/// tooltip is kept hidden while the menu is open, so it does not cover it.
pub fn context_menu(widget: &impl IsA<gtk::Widget>, build: impl Fn() -> gtk::Popover + 'static) {
    let widget = widget.as_ref();
    let menu: Rc<OnceCell<gtk::Popover>> = Rc::default();
    let owned = menu.clone();
    widget.connect_destroy(move |_| {
        if let Some(menu) = owned.get() {
            menu.unparent();
        }
    });
    let had_tooltip = widget.has_tooltip();
    let open = {
        let widget = widget.downgrade();
        Rc::new(move || {
            let Some(widget) = widget.upgrade() else {
                return;
            };
            let menu = menu.get_or_init(|| {
                let menu = build();
                menu.set_parent(&widget);
                let restore = widget.downgrade();
                menu.connect_closed(move |_| {
                    if let Some(widget) = restore.upgrade() {
                        widget.set_has_tooltip(had_tooltip);
                    }
                });
                menu
            });
            widget.set_has_tooltip(false);
            menu.popup();
        })
    };
    let right = gtk::GestureClick::new();
    right.set_button(gdk::BUTTON_SECONDARY);
    let right_open = open.clone();
    right.connect_pressed(move |_, _, _, _| right_open());
    widget.add_controller(right);
    let press = gtk::GestureLongPress::new();
    press.connect_pressed(move |_, _, _| open());
    widget.add_controller(press);
}

pub fn command_button(label: &str, icon: &str, command: &str, args: &[&str]) -> gtk::Button {
    let button = gtk::Button::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.append(&gtk::Image::from_icon_name(icon));
    if !label.is_empty() {
        content.append(&gtk::Label::new(Some(label)));
    }
    button.set_child(Some(&content));
    let command = command.to_string();
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    button.connect_clicked(move |button| {
        run(&command, &args);
        close_popover(button);
    });
    button
}

pub fn show_error(message: &str) {
    eprintln!("{message}");
    let dialog = gtk::MessageDialog::builder()
        .text("Could not complete the action")
        .secondary_text(message)
        .buttons(gtk::ButtonsType::Close)
        .build();
    dialog.add_css_class("settings-window");
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.present();
}

pub fn confirm(title: &str, detail: &str, command: &str, args: &[&str]) {
    let dialog = gtk::MessageDialog::builder()
        .text(title)
        .secondary_text(detail)
        .modal(true)
        .buttons(gtk::ButtonsType::OkCancel)
        .build();
    let command = command.to_string();
    let args: Vec<_> = args.iter().map(|s| s.to_string()).collect();
    dialog.add_css_class("settings-window");
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Ok {
            run(&command, &args);
        }
        dialog.close();
    });
    dialog.present();
}

/// The first icon of `names` the current icon theme has, else the last one.
pub fn themed_icon(names: &[&str]) -> String {
    let theme = gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    names
        .iter()
        .find(|n| theme.as_ref().is_some_and(|t| t.has_icon(n)))
        .or(names.last())
        .map(|n| n.to_string())
        .unwrap_or_default()
}

/// Sign out, restart and power off, each after a confirmation.
pub fn power_buttons(preview: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("power");
    for (tooltip, icon, title, command, args) in [
        (
            "Sign out",
            "system-log-out-symbolic",
            "Sign out of SegLinux?",
            "labwc",
            &["--exit"][..],
        ),
        (
            "Restart",
            "system-reboot-symbolic",
            "Restart this computer?",
            "loginctl",
            &["reboot"][..],
        ),
        (
            "Shut down",
            "system-shutdown-symbolic",
            "Shut down this computer?",
            "loginctl",
            &["poweroff"][..],
        ),
    ] {
        let button = gtk::Button::from_icon_name(icon);
        button.set_tooltip_text(Some(tooltip));
        button.set_sensitive(!preview);
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        button.connect_clicked(move |b| {
            close_popover(b);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            confirm(title, "Save your work first.", command, &args);
        });
        row.append(&button);
    }
    row
}
