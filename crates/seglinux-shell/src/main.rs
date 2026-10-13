//! SegLinux's desktop, in Rust: wallpaper and desktop icons, a floating
//! dock with Start menu, applications, tray and clock, on the labwc Wayland
//! compositor (which manages the windows, like KWin under Plasma).
mod apps;
mod common;
mod compositor;
mod config;
mod desktop;
mod dock;
mod gaming;
mod launcher;
mod managed;
mod settings;
mod tasks;
mod tray;

use config::Config;
use dock::Menus;
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{cell::Cell, cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

/// Gap between the floating dock and the edge of the screen.
const DOCK_MARGIN: i32 = 6;

const HELP: &str = "SegLinux Shell
  --preview            Run in a window without taking over your desktop
  --wallpaper PATH     Override the background
  --settings [PAGE]    Open Settings (appearance, desktop, windows, gaming, system)
  --gaming-mode-args   Print gamescope's options for the Gaming Mode session
  --version            Show version

Open Start: gapplication action org.seglinux.Shell menu";

/// Applies the parts of the configuration GTK handles itself.
fn apply_gtk_settings(config: &Config) {
    if let Some(settings) = gtk::Settings::default() {
        set_dark_theme(&settings, config.theme == config::Theme::Dark);
        settings.set_gtk_enable_animations(config.animations);
    }
}

/// GTK 4.20 replaced gtk-application-prefer-dark-theme with
/// gtk-interface-color-scheme and warns when the old setting is used; older
/// GTK only knows the old one.
fn set_dark_theme(settings: &gtk::Settings, dark: bool) {
    const SCHEME: &str = "gtk-interface-color-scheme";
    let scheme = settings
        .find_property(SCHEME)
        .and_then(|pspec| glib::EnumClass::with_type(pspec.value_type()))
        .and_then(|class| class.to_value_by_nick(if dark { "dark" } else { "light" }));
    match scheme {
        Some(value) => settings.set_property_from_value(SCHEME, &value),
        None => settings.set_gtk_application_prefer_dark_theme(dark),
    }
}

/// Calls `on_change` shortly after `shell.toml` is written (Settings
/// replaces it atomically, which shows up as several events).
fn watch_config(on_change: impl Fn() + 'static) -> Option<gio::FileMonitor> {
    let path = Config::path();
    let dir = path.parent()?;
    let _ = std::fs::create_dir_all(dir);
    let monitor = gio::File::for_path(dir)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .ok()?;
    let name = path.file_name()?.to_owned();
    let pending = Rc::new(Cell::new(false));
    let on_change = Rc::new(on_change);
    monitor.connect_changed(move |_, file, other, _| {
        let concerns = |f: Option<&gio::File>| {
            f.and_then(|f| f.basename())
                .is_some_and(|b| b.as_os_str() == name)
        };
        if !(concerns(Some(file)) || concerns(other)) || pending.replace(true) {
            return;
        }
        let pending = pending.clone();
        let on_change = on_change.clone();
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            pending.set(false);
            on_change();
        });
    });
    Some(monitor)
}

fn main() -> glib::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version") {
        println!("seglinux-shell {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    // For seglinux-gaming-session: one option per line, without starting GTK.
    if args.iter().any(|a| a == "--gaming-mode-args") {
        for arg in Config::load().gaming_mode.gamescope_args() {
            println!("{arg}");
        }
        return glib::ExitCode::SUCCESS;
    }
    let mut preview = false;
    let mut settings: Option<String> = None;
    let mut wallpaper: Option<PathBuf> = None;
    let mut arguments = args.iter().peekable();
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--preview" => preview = true,
            "--settings" => {
                let page = arguments.next_if(|next| !next.starts_with('-'));
                settings = Some(page.cloned().unwrap_or_default());
            }
            "--wallpaper" => match arguments.next() {
                Some(path) => wallpaper = Some(PathBuf::from(path)),
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
    // The configuration, with the command line's wallpaper if any.
    let load = {
        let wallpaper = wallpaper.clone();
        move |config: Config| -> Config {
            match &wallpaper {
                Some(path) => Config {
                    wallpaper: path.clone(),
                    ..config
                },
                None => config,
            }
        }
    };
    let config = Rc::new(RefCell::new(load(Config::load())));
    let app = gtk::Application::builder()
        .application_id(if settings.is_some() {
            "org.seglinux.Shell.Settings"
        } else if preview {
            "org.seglinux.Shell.Preview"
        } else {
            "org.seglinux.Shell"
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
    // Kept for the life of the application.
    let config_monitor: Rc<RefCell<Option<gio::FileMonitor>>> = Rc::default();
    app.connect_activate(move |app| {
        if let Some(window) = app.windows().first() {
            window.present();
            return;
        }
        let Some(display) = gdk::Display::default() else {
            app.quit();
            return;
        };
        if !preview && settings.is_none() && !gtk4_layer_shell::is_supported() {
            startup_failed.set(true);
            eprintln!(
                "SegLinux Shell needs a Wayland compositor with layer-shell support. Use seglinux-session, or --preview for a windowed demonstration."
            );
            app.quit();
            return;
        }
        let css = gtk::CssProvider::new();
        css.load_from_data(&config.borrow().css());
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        apply_gtk_settings(&config.borrow());
        if let Some(page) = &settings {
            settings::open(app, &config.borrow(), page, &css);
            return;
        }

        // Builds (or rebuilds) the desktop with the current configuration.
        let rebuild: Rc<dyn Fn()> = if preview {
            let window = gtk::ApplicationWindow::builder()
                .application(app)
                .title("SegLinux Desktop · Preview")
                .default_width(1280)
                .default_height(800)
                .build();
            window.present();
            let app = app.clone();
            let config = config.clone();
            let menus = menus.clone();
            Rc::new(move || {
                let config = config.borrow();
                menus.borrow_mut().clear();
                let overlay = desktop::background(&config, true, None);
                let bar = dock::dock(&app, &menus, &config, true);
                let top = config.dock_position == config::DockPosition::Top;
                bar.set_valign(if top { gtk::Align::Start } else { gtk::Align::End });
                bar.set_halign(gtk::Align::Center);
                bar.set_margin_bottom(DOCK_MARGIN);
                bar.set_margin_top(DOCK_MARGIN);
                overlay.add_overlay(&bar);
                window.set_child(Some(&overlay));
            })
        } else {
            if let Err(error) = compositor::sync(&config.borrow()) {
                eprintln!("Cannot update the compositor's settings: {error}");
            }
            let monitors = display.monitors();
            let windows: Rc<RefCell<Vec<gtk::ApplicationWindow>>> = Rc::default();
            let rebuild = {
                let app = app.clone();
                let config = config.clone();
                let menus = menus.clone();
                let windows = windows.clone();
                let monitors = monitors.clone();
                move || {
                    // Keep the application alive while replacing output surfaces.
                    let _guard = app.hold();
                    let config = config.borrow();
                    let old: Vec<_> = windows.borrow_mut().drain(..).collect();
                    menus.borrow_mut().clear();
                    for index in 0..monitors.n_items() {
                        let Some(monitor) = monitors
                            .item(index)
                            .and_then(|m| m.downcast::<gdk::Monitor>().ok())
                        else {
                            continue;
                        };
                        let background = gtk::ApplicationWindow::builder()
                            .application(&app)
                            .title("SegLinux Desktop")
                            .build();
                        background.init_layer_shell();
                        background.set_namespace(Some("seglinux-background"));
                        background.set_layer(Layer::Background);
                        background.set_monitor(Some(&monitor));
                        background.set_exclusive_zone(-1);
                        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                            background.set_anchor(edge, true);
                        }
                        // Desktop icons on the first monitor, as in Plasma.
                        let geometry = monitor.geometry();
                        let scale = monitor.scale_factor();
                        background.set_child(Some(&desktop::background(
                            &config,
                            index == 0,
                            Some((geometry.width() * scale, geometry.height() * scale)),
                        )));
                        background.present();

                        // The dock floats: anchored to one edge only, so it
                        // is centred and only as wide as its contents.
                        let bar = gtk::ApplicationWindow::builder()
                            .application(&app)
                            .title("SegLinux Dock")
                            .build();
                        bar.init_layer_shell();
                        bar.set_namespace(Some("seglinux-panel"));
                        bar.set_layer(Layer::Top);
                        bar.set_monitor(Some(&monitor));
                        bar.set_keyboard_mode(KeyboardMode::OnDemand);
                        let edge = if config.dock_position == config::DockPosition::Top {
                            Edge::Top
                        } else {
                            Edge::Bottom
                        };
                        bar.set_anchor(edge, true);
                        bar.set_margin(edge, DOCK_MARGIN);
                        bar.auto_exclusive_zone_enable();
                        bar.set_child(Some(&dock::dock(&app, &menus, &config, false)));
                        bar.present();
                        windows.borrow_mut().extend([background, bar]);
                    }
                    // The old surfaces go once the new ones are up: no
                    // flash of an empty screen while settings are applied.
                    for window in old {
                        window.close();
                    }
                }
            };
            let rebuild: Rc<dyn Fn()> = Rc::new(rebuild);
            let on_monitors = rebuild.clone();
            monitors.connect_items_changed(move |_, _, _, _| on_monitors());
            rebuild
        };
        rebuild();

        // Settings take effect as soon as they are saved.
        let config = config.clone();
        let load = load.clone();
        *config_monitor.borrow_mut() = watch_config(move || {
            let fresh = match Config::try_load() {
                Ok(fresh) => load(fresh),
                // Being edited by hand: keep the desktop as it is.
                Err(error) => {
                    eprintln!("{}: {error}; keeping the current settings", Config::path().display());
                    return;
                }
            };
            if fresh == *config.borrow() {
                return;
            }
            css.load_from_data(&fresh.css());
            apply_gtk_settings(&fresh);
            if !preview && let Err(error) = compositor::sync(&fresh) {
                eprintln!("Cannot update the compositor's settings: {error}");
            }
            *config.borrow_mut() = fresh;
            rebuild();
        });
    });
    let status = app.run_with_args(&["seglinux-shell"]);
    if failed.get() {
        glib::ExitCode::FAILURE
    } else {
        status
    }
}
