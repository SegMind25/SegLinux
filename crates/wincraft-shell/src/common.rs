//! Small helpers shared by the desktop, dock and menus.
use gtk::{gdk, gio, glib, prelude::*};

/// Opens whichever terminal emulator is installed (see data/system/wincraft-terminal).
pub const TERMINAL: &str = "wincraft-terminal";

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
            "Sign out of Wincraft?",
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
