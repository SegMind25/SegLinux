mod config;
mod settings;
mod tasks;

use config::Config;
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

type Menus = Rc<RefCell<Vec<glib::WeakRef<gtk::MenuButton>>>>;

fn main() -> glib::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Wincraft Shell\n  --preview   Run in a window without taking over your desktop\n  --wallpaper PATH   Override the background\n  --settings  Open desktop preferences\n  --version   Show version\n\nOpen Start: gapplication action org.wincraft.Shell menu"
        );
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version") {
        println!("wincraft-shell {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    let mut config = Config::load();
    let mut preview = false;
    let mut settings = false;
    let mut arguments = args.iter();
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--preview" => preview = true,
            "--settings" => settings = true,
            "--wallpaper" => match arguments.next() {
                Some(path) => config.wallpaper = PathBuf::from(path),
                None => {
                    eprintln!("--wallpaper needs a path");
                    return glib::ExitCode::FAILURE;
                }
            },
            _ => {
                eprintln!("Unknown option: {arg}");
                return glib::ExitCode::FAILURE;
            }
        }
    }
    let app = gtk::Application::builder()
        .application_id(if settings {
            "org.wincraft.Shell.Settings"
        } else if preview {
            "org.wincraft.Shell.Preview"
        } else {
            "org.wincraft.Shell"
        })
        .build();
    let menus: Menus = Rc::default();
    let menu_action = gio::SimpleAction::new("menu", None);
    let action_menus = menus.clone();
    menu_action.connect_activate(move |_, _| {
        if let Some(menu) = action_menus
            .borrow()
            .first()
            .and_then(glib::WeakRef::upgrade)
        {
            if menu.popover().is_some_and(|p| p.is_visible()) {
                menu.popdown();
            } else {
                menu.popup();
            }
        }
    });
    app.add_action(&menu_action);
    let failed = Rc::new(Cell::new(false));
    let startup_failed = failed.clone();
    app.connect_activate(move |app| {
        if let Some(window) = app.windows().first() { window.present(); return; }
        let Some(display) = gdk::Display::default() else { app.quit(); return; };
        if !preview && !settings && !gtk4_layer_shell::is_supported() {
            startup_failed.set(true);
            eprintln!("Wincraft Shell needs a Wayland compositor with layer-shell support. Use wincraft-session, or --preview for a windowed demonstration.");
            app.quit();
            return;
        }
        let css = gtk::CssProvider::new();
        css.load_from_data(include_str!("../../../data/desktop/shell.css"));
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        gtk::Settings::default().unwrap().set_gtk_application_prefer_dark_theme(true);
        if settings {
            settings::open(app, &config);
        } else if preview {
            let window = gtk::ApplicationWindow::builder().application(app).title("Wincraft Desktop · Preview").default_width(1280).default_height(800).build();
            let overlay = background(&config);
            let panel = panel(app, &menus, true);
            panel.set_valign(gtk::Align::End);
            overlay.add_overlay(&panel);
            window.set_child(Some(&overlay));
            window.present();
        } else {
            let monitors = display.monitors();
            let windows: Rc<RefCell<Vec<gtk::ApplicationWindow>>> = Rc::default();
            let rebuild = {
                let app = app.clone();
                let config = config.clone();
                let menus = menus.clone();
                let windows = windows.clone();
                move |monitors: &gio::ListModel| {
                    // Keep the application alive while replacing output surfaces.
                    let _guard = app.hold();
                    for window in windows.borrow_mut().drain(..) { window.close(); }
                    menus.borrow_mut().clear();
                    for index in 0..monitors.n_items() {
                        let monitor = monitors.item(index).unwrap().downcast::<gdk::Monitor>().unwrap();
                        let desktop = gtk::ApplicationWindow::builder().application(&app).title("Wincraft Desktop").build();
                        desktop.init_layer_shell();
                        desktop.set_namespace(Some("wincraft-background"));
                        desktop.set_layer(Layer::Background);
                        desktop.set_monitor(Some(&monitor));
                        desktop.set_exclusive_zone(-1);
                        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] { desktop.set_anchor(edge, true); }
                        desktop.set_child(Some(&background(&config)));
                        desktop.present();
                        let bar = gtk::ApplicationWindow::builder().application(&app).title("Wincraft Panel").build();
                        bar.init_layer_shell();
                        bar.set_namespace(Some("wincraft-panel"));
                        bar.set_layer(Layer::Top);
                        bar.set_monitor(Some(&monitor));
                        bar.set_keyboard_mode(KeyboardMode::OnDemand);
                        for edge in [Edge::Bottom, Edge::Left, Edge::Right] { bar.set_anchor(edge, true); }
                        bar.auto_exclusive_zone_enable();
                        bar.set_child(Some(&panel(&app, &menus, false)));
                        bar.present();
                        windows.borrow_mut().extend([desktop, bar]);
                    }
                }
            };
            rebuild(&monitors);
            monitors.connect_items_changed(move |list, _, _, _| rebuild(list));
        }
    });
    let status = app.run_with_args(&["wincraft-shell"]);
    if failed.get() {
        glib::ExitCode::FAILURE
    } else {
        status
    }
}

fn background(config: &Config) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("desktop");
    let picture = gtk::Picture::for_filename(&config.wallpaper);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    overlay.set_child(Some(&picture));
    let brand = gtk::Box::new(gtk::Orientation::Vertical, 6);
    brand.set_halign(gtk::Align::Start);
    brand.set_valign(gtk::Align::Start);
    brand.set_margin_start(42);
    brand.set_margin_top(42);
    let title = gtk::Label::new(Some("Wincraft"));
    title.set_xalign(0.0);
    title.add_css_class("desktop-title");
    brand.append(&title);
    let subtitle = gtk::Label::new(Some("YOUR SPACE. YOUR SYSTEM."));
    subtitle.set_xalign(0.0);
    subtitle.add_css_class("desktop-subtitle");
    brand.append(&subtitle);
    let shortcuts = gtk::Box::new(gtk::Orientation::Vertical, 8);
    shortcuts.set_margin_top(30);
    for (label, icon, command) in [
        ("Home", "user-home-symbolic", "dolphin"),
        ("Terminal", "utilities-terminal-symbolic", TERMINAL),
    ] {
        if installed(command) {
            shortcuts.append(&command_button(label, icon, command, &[]));
        }
    }
    if std::path::Path::new("/usr/lib/wincraft/live-autostart").exists() {
        // The graphical Setup; the text-mode one when it is missing.
        let install = if installed("wincraft-installer") {
            command_button(
                "Install Wincraft",
                "system-software-install-symbolic",
                "wincraft-installer",
                &[],
            )
        } else {
            command_button(
                "Install Wincraft",
                "system-software-install-symbolic",
                TERMINAL,
                &["sudo", "/usr/bin/wincraft-setup"],
            )
        };
        shortcuts.append(&install);
    }
    brand.append(&shortcuts);
    overlay.add_overlay(&brand);
    overlay
}

fn panel(app: &gtk::Application, menus: &Menus, preview: bool) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("panel");
    let start = gtk::MenuButton::builder()
        .icon_name("view-app-grid-symbolic")
        .tooltip_text("Start · Super+Space")
        .build();
    start.add_css_class("start");
    let popover = launcher(app);
    start.set_popover(Some(&popover));
    menus.borrow_mut().push(start.downgrade());
    bar.append(&start);
    for (label, icon, command) in [
        ("Files", "system-file-manager-symbolic", "dolphin"),
        ("Web browser", "web-browser-symbolic", "firefox"),
        ("Terminal", "utilities-terminal-symbolic", TERMINAL),
        ("Steam", "steam", "steam"),
    ] {
        // The Lite edition installs most applications later.
        if !installed(command) {
            continue;
        }
        let button = command_button("", icon, command, &[]);
        button.set_tooltip_text(Some(label));
        bar.append(&button);
    }
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    let taskbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    taskbar.set_hexpand(true);
    if !preview {
        tasks::attach(&taskbar);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .child(&taskbar)
        .build();
    bar.append(&scroll);
    let controls = gtk::MenuButton::builder()
        .icon_name("preferences-system-symbolic")
        .tooltip_text("Quick settings")
        .build();
    controls.set_popover(Some(&quick_settings(app, preview)));
    bar.append(&controls);
    let clock = gtk::Label::new(None);
    clock.add_css_class("clock");
    let format = Config::load().clock_format;
    let tick = move |label: &gtk::Label| {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(text) = now.format(&format) {
                label.set_text(&text);
            }
            if let Ok(date) = now.format("%A, %d %B %Y") {
                label.set_tooltip_text(Some(&date));
            }
        }
    };
    tick(&clock);
    let weak = clock.downgrade();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let Some(label) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        tick(&label);
        glib::ControlFlow::Continue
    });
    bar.append(&clock);
    bar
}

fn launcher(_app: &gtk::Application) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("launcher");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_size_request(470, -1);
    let heading = gtk::Label::new(Some("Welcome to Wincraft"));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    content.append(&heading);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search applications…")
        .build();
    content.append(&search);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let mut apps = gio::AppInfo::all();
    apps.retain(|app| app.should_show());
    apps.sort_by_key(|a| a.display_name().to_lowercase());
    let mut entries = Vec::new();
    for app in apps {
        let row = gtk::ListBoxRow::new();
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
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        inner.append(&text);
        button.set_child(Some(&inner));
        button.set_tooltip_text(app.description().as_deref());
        row.set_child(Some(&button));
        list.append(&row);
        entries.push((
            row,
            app.display_name().to_string(),
            app.description().unwrap_or_default().to_string(),
        ));
        let weak = popover.downgrade();
        button.connect_clicked(move |_| {
            let context = gdk::Display::default().map(|d| d.app_launch_context());
            if let Err(error) = app.launch(&[], context.as_ref()) {
                show_error(&format!("Cannot start {}: {error}", app.display_name()));
            }
            if let Some(p) = weak.upgrade() {
                p.popdown();
            }
        });
    }
    let empty = gtk::Label::new(Some("No matching applications"));
    empty.set_visible(entries.is_empty());
    let empty_search = empty.clone();
    search.connect_search_changed(move |search| {
        let query = search.text();
        let mut any = false;
        for (row, name, description) in &entries {
            let visible = config::matches_query(name, description, &query);
            row.set_visible(visible);
            any |= visible;
        }
        empty_search.set_visible(!any);
    });
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(350)
        .max_content_height(350)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    content.append(&scroll);
    content.append(&empty);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if installed("dolphin") {
        footer.append(&command_button("Files", "folder-symbolic", "dolphin", &[]));
    }
    footer.append(&command_button(
        "Settings",
        "preferences-system-symbolic",
        "wincraft-shell",
        &["--settings"],
    ));
    content.append(&footer);
    popover.set_child(Some(&content));
    popover.connect_show(move |_| {
        search.set_text("");
        search.grab_focus();
    });
    popover
}

fn quick_settings(app: &gtk::Application, preview: bool) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_size_request(310, -1);
    let heading = gtk::Label::new(Some("Quick settings"));
    heading.add_css_class("heading");
    content.append(&heading);
    // Without the graphical connection editor (Lite edition), nmtui runs in
    // a terminal.
    let (network, network_args) = if installed("nm-connection-editor") {
        ("nm-connection-editor", vec![])
    } else {
        (TERMINAL, vec!["nmtui"])
    };
    for (label, icon, command, args) in [
        (
            "Network connections",
            "network-wireless-symbolic",
            network,
            network_args,
        ),
        ("Sound", "audio-volume-high-symbolic", "pavucontrol", vec![]),
        (
            "Display settings",
            "video-display-symbolic",
            "wdisplays",
            vec![],
        ),
        (
            "Lock screen",
            "system-lock-screen-symbolic",
            "swaylock",
            vec!["-c", "090f1d"],
        ),
    ] {
        if !installed(command) {
            continue;
        }
        let button = command_button(label, icon, command, &args);
        if preview && command == "swaylock" {
            button.set_sensitive(false);
        }
        content.append(&button);
    }
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let logout = gtk::Button::with_label(if preview {
        "Close preview"
    } else {
        "Sign out…"
    });
    let app = app.clone();
    logout.connect_clicked(move |_| {
        if preview {
            app.quit();
        } else {
            confirm(
                "Sign out of Wincraft?",
                "Save your work before signing out.",
                "labwc",
                &["--exit"],
            );
        }
    });
    content.append(&logout);
    let shutdown = gtk::Button::with_label("Power off…");
    shutdown.set_sensitive(!preview);
    shutdown.connect_clicked(|_| {
        confirm(
            "Power off this computer?",
            "Save your work before powering off.",
            "loginctl",
            &["poweroff"],
        )
    });
    content.append(&shutdown);
    popover.set_child(Some(&content));
    popover
}

fn command_button(label: &str, icon: &str, command: &str, args: &[&str]) -> gtk::Button {
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
        if let Some(popover) = button
            .ancestor(gtk::Popover::static_type())
            .and_then(|widget| widget.downcast::<gtk::Popover>().ok())
        {
            popover.popdown();
        }
    });
    button
}

/// Opens whichever terminal emulator is installed (see data/system/wincraft-terminal).
const TERMINAL: &str = "wincraft-terminal";

fn installed(command: &str) -> bool {
    glib::find_program_in_path(command).is_some()
}

fn run(command: &str, args: &[String]) {
    let mut argv = vec![std::ffi::OsStr::new(command)];
    argv.extend(args.iter().map(std::ffi::OsStr::new));
    // GSubprocess reaps children; arguments never pass through a shell.
    if let Err(error) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) {
        show_error(&format!("Cannot start {command}: {error}"));
    }
}

fn show_error(message: &str) {
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

fn confirm(title: &str, detail: &str, command: &str, args: &[&str]) {
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
