//! Wincraft's desktop, in Rust: wallpaper and desktop icons, a floating
//! dock with Start menu, applications, tray and clock, on the labwc Wayland
//! compositor (which manages the windows, like KWin under Plasma).
mod apps;
mod common;
mod config;
mod desktop;
mod dock;
mod launcher;
mod settings;
mod tasks;
mod tray;

use config::Config;
use dock::Menus;
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{cell::Cell, cell::RefCell, path::PathBuf, rc::Rc};

/// Gap between the floating dock and the bottom of the screen.
const DOCK_MARGIN: i32 = 6;

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
        if let Some(window) = app.windows().first() {
            window.present();
            return;
        }
        let Some(display) = gdk::Display::default() else {
            app.quit();
            return;
        };
        if !preview && !settings && !gtk4_layer_shell::is_supported() {
            startup_failed.set(true);
            eprintln!(
                "Wincraft Shell needs a Wayland compositor with layer-shell support. Use wincraft-session, or --preview for a windowed demonstration."
            );
            app.quit();
            return;
        }
        let css = gtk::CssProvider::new();
        css.load_from_data(include_str!("../../../data/desktop/shell.css"));
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk::Settings::default()
            .unwrap()
            .set_gtk_application_prefer_dark_theme(true);
        if settings {
            settings::open(app, &config);
        } else if preview {
            let window = gtk::ApplicationWindow::builder()
                .application(app)
                .title("Wincraft Desktop · Preview")
                .default_width(1280)
                .default_height(800)
                .build();
            let overlay = desktop::background(&config, true);
            let bar = dock::dock(app, &menus, &config, true);
            bar.set_valign(gtk::Align::End);
            bar.set_halign(gtk::Align::Center);
            bar.set_margin_bottom(DOCK_MARGIN);
            overlay.add_overlay(&bar);
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
                    for window in windows.borrow_mut().drain(..) {
                        window.close();
                    }
                    menus.borrow_mut().clear();
                    for index in 0..monitors.n_items() {
                        let monitor = monitors
                            .item(index)
                            .unwrap()
                            .downcast::<gdk::Monitor>()
                            .unwrap();
                        let background = gtk::ApplicationWindow::builder()
                            .application(&app)
                            .title("Wincraft Desktop")
                            .build();
                        background.init_layer_shell();
                        background.set_namespace(Some("wincraft-background"));
                        background.set_layer(Layer::Background);
                        background.set_monitor(Some(&monitor));
                        background.set_exclusive_zone(-1);
                        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                            background.set_anchor(edge, true);
                        }
                        // Desktop icons on the first monitor, as in Plasma.
                        background.set_child(Some(&desktop::background(&config, index == 0)));
                        background.present();

                        // The dock floats: anchored to the bottom edge only,
                        // so it is centred and only as wide as its contents.
                        let bar = gtk::ApplicationWindow::builder()
                            .application(&app)
                            .title("Wincraft Dock")
                            .build();
                        bar.init_layer_shell();
                        bar.set_namespace(Some("wincraft-panel"));
                        bar.set_layer(Layer::Top);
                        bar.set_monitor(Some(&monitor));
                        bar.set_keyboard_mode(KeyboardMode::OnDemand);
                        bar.set_anchor(Edge::Bottom, true);
                        bar.set_margin(Edge::Bottom, DOCK_MARGIN);
                        bar.auto_exclusive_zone_enable();
                        bar.set_child(Some(&dock::dock(&app, &menus, &config, false)));
                        bar.present();
                        windows.borrow_mut().extend([background, bar]);
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
