//! Rust settings for the shell and the existing segrun game launcher.
use crate::{
    common,
    config::{Config, DockPosition, Theme, WallpaperFit},
};
use gtk::{gio, glib, prelude::*};
use segrun::config::{Config as GameConfig, Toggle};
use std::path::{Path, PathBuf};

fn note(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label
}
fn page(book: &gtk::Notebook, title: &str) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();
    book.append_page(&scroll, Some(&gtk::Label::new(Some(title))));
    content
}
fn entry(page: &gtk::Box, label: &str, text: &str) -> gtk::Entry {
    page.append(&note(label));
    let widget = gtk::Entry::new();
    widget.set_text(text);
    page.append(&widget);
    widget
}
fn toggle(page: &gtk::Box, label: &str, value: bool) -> gtk::CheckButton {
    let widget = gtk::CheckButton::with_label(label);
    widget.set_active(value);
    page.append(&widget);
    widget
}
fn choice(page: &gtk::Box, label: &str, values: &[&str], selected: u32) -> gtk::DropDown {
    page.append(&note(label));
    let widget = gtk::DropDown::from_strings(values);
    widget.set_selected(selected);
    page.append(&widget);
    widget
}
fn number(page: &gtk::Box, label: &str, low: f64, high: f64, value: f64) -> gtk::SpinButton {
    page.append(&note(label));
    let widget = gtk::SpinButton::with_range(low, high, 1.0);
    widget.set_value(value);
    page.append(&widget);
    widget
}
fn save_text(path: &Path, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("Invalid settings path")?)
        .map_err(|e| e.to_string())?;
    gio::File::for_path(path)
        .replace_contents(
            text.as_bytes(),
            None,
            false,
            gio::FileCreateFlags::REPLACE_DESTINATION,
            gio::Cancellable::NONE,
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn open(app: &gtk::Application, config: &Config) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("SegLinux Settings")
        .default_width(760)
        .default_height(680)
        .build();
    window.add_css_class("settings-window");
    let book = gtk::Notebook::new();
    let appearance = page(&book, "Appearance");
    appearance.append(&note("Customize your SegLinux shell. Desktop changes apply next time you sign in; they do not change other applications' themes."));
    let wallpaper = entry(
        &appearance,
        "Wallpaper image",
        &config.wallpaper.to_string_lossy(),
    );
    let browse = gtk::Button::with_label("Choose image…");
    appearance.append(&browse);
    let parent = window.downgrade();
    let wallpaper_entry = wallpaper.clone();
    browse.connect_clicked(move |_| {
        let parent = parent.upgrade();
        let dialog = gtk::FileChooserNative::new(
            Some("Wallpaper image"),
            parent.as_ref(),
            gtk::FileChooserAction::Open,
            Some("Choose"),
            Some("Cancel"),
        );
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Images"));
        filter.add_pixbuf_formats();
        dialog.add_filter(&filter);
        let target = wallpaper_entry.clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept
                && let Some(path) = dialog.file().and_then(|f| f.path())
            {
                target.set_text(&path.to_string_lossy());
            }
            dialog.destroy();
        });
        dialog.show();
    });
    let fit = choice(
        &appearance,
        "Wallpaper layout",
        &["Cover", "Fit inside screen", "Stretch"],
        match config.wallpaper_fit {
            WallpaperFit::Cover => 0,
            WallpaperFit::Contain => 1,
            WallpaperFit::Fill => 2,
        },
    );
    let theme = choice(
        &appearance,
        "Shell theme",
        &["Dark", "Light"],
        u32::from(config.theme == Theme::Light),
    );
    let accent = entry(&appearance, "Accent color (#RRGGBB)", &config.accent);
    let font = number(
        &appearance,
        "Shell text size (pixels)",
        10.,
        24.,
        config.font_size.into(),
    );
    let animations = toggle(&appearance, "Enable shell animations", config.animations);
    let desktop = page(&book, "Desktop & dock");
    let icons = toggle(&desktop, "Show Desktop folder icons", config.desktop_icons);
    let icon_size = number(
        &desktop,
        "Desktop icon size",
        24.,
        96.,
        config.desktop_icon_size.into(),
    );
    let dock_size = number(
        &desktop,
        "Dock icon size",
        20.,
        64.,
        config.dock_icon_size.into(),
    );
    let position = choice(
        &desktop,
        "Dock position",
        &["Bottom", "Top"],
        u32::from(config.dock_position == DockPosition::Top),
    );
    let clock = entry(
        &desktop,
        "Clock format (%H:%M = 24-hour; %I:%M %p = 12-hour; %x = date)",
        &config.clock_format,
    );
    let presets = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (label, format) in [
        ("24-hour", crate::config::CLOCK_24H),
        ("12-hour", crate::config::CLOCK_12H),
    ] {
        let button = gtk::Button::with_label(label);
        let clock_entry = clock.clone();
        button.connect_clicked(move |_| clock_entry.set_text(format));
        presets.append(&button);
    }
    desktop.append(&presets);
    let pins = entry(
        &desktop,
        "Pinned app IDs in order, separated by commas",
        &config.pinned.join(", "),
    );
    desktop.append(&note("Example: org.kde.dolphin, firefox, seglinux-terminal, steam, net.lutris.Lutris. Applications that are not installed are skipped."));
    let save = gtk::Button::with_label("Save desktop and appearance");
    desktop.append(&save);
    let status =
        note("Changes in both desktop tabs are saved together. Sign out and back in to apply.");
    desktop.append(&status);
    let saved = config.clone();
    save.connect_clicked(move |_| {
        let result = (|| -> Result<(), String> {
            let path = PathBuf::from(wallpaper.text().as_str());
            // An unchanged missing default wallpaper should not block unrelated preferences.
            if path != saved.wallpaper
                && (!path.is_absolute() || gtk::gdk::Texture::from_filename(&path).is_err())
            {
                return Err("Choose an existing image using an absolute path.".into());
            }
            let mut updated = saved.clone();
            updated.wallpaper = path;
            updated.wallpaper_fit = match fit.selected() {
                1 => WallpaperFit::Contain,
                2 => WallpaperFit::Fill,
                _ => WallpaperFit::Cover,
            };
            updated.theme = if theme.selected() == 1 {
                Theme::Light
            } else {
                Theme::Dark
            };
            updated.accent = accent.text().trim().to_string();
            updated.font_size = font.value_as_int() as u32;
            updated.animations = animations.is_active();
            updated.desktop_icons = icons.is_active();
            updated.desktop_icon_size = icon_size.value_as_int() as u32;
            updated.dock_icon_size = dock_size.value_as_int() as u32;
            updated.dock_position = if position.selected() == 1 {
                DockPosition::Top
            } else {
                DockPosition::Bottom
            };
            updated.clock_format = clock.text().to_string();
            updated.pinned = pins
                .text()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            updated.validate()?;
            glib::DateTime::now_local()
                .and_then(|now| now.format(&updated.clock_format))
                .map_err(|_| "Invalid clock format".to_string())?;
            let path = Config::path().ok_or("HOME or XDG_CONFIG_HOME must be set")?;
            save_text(
                &path,
                &toml::to_string_pretty(&updated).map_err(|e| e.to_string())?,
            )
        })();
        status.set_text(&match result {
            Ok(()) => "Saved desktop and appearance. Sign out and back in to apply.".into(),
            Err(e) => format!("Could not save: {e}"),
        });
    });
    let save_appearance = gtk::Button::with_label("Save desktop and appearance");
    let desktop_save = save.clone();
    let tabs = book.clone();
    save_appearance.connect_clicked(move |_| {
        desktop_save.emit_clicked();
        tabs.set_current_page(Some(1));
    });
    appearance.append(&save_appearance);
    gaming_page(&book);
    system_page(&book);
    window.set_child(Some(&book));
    window.present();
}

fn toggle_index(value: Toggle) -> u32 {
    match value {
        Toggle::Auto => 0,
        Toggle::Always => 1,
        Toggle::Never => 2,
    }
}
fn selected_toggle(widget: &gtk::DropDown) -> Toggle {
    match widget.selected() {
        1 => Toggle::Always,
        2 => Toggle::Never,
        _ => Toggle::Auto,
    }
}
fn gaming_page(book: &gtk::Notebook) {
    let content = page(book, "Gaming");
    content.append(&note("These preferences apply to future Windows programs launched through segrun. They do not override Steam or Lutris settings. Per-game overrides and custom environment variables are preserved."));
    let config = match GameConfig::load(None) {
        Ok((c, _)) => c,
        Err(e) => {
            content.append(&note(&format!(
                "Cannot load gaming settings: {e:#}. Correct the configuration file before saving."
            )));
            return;
        }
    };
    let d = &config.defaults;
    let runner = entry(
        &content,
        "Runner: wine, wine:/path/to/wine, proton:/path/to/Proton, umu, or umu:/path/to/Proton",
        &d.runner,
    );
    let prefix = entry(
        &content,
        "Default Windows environment (prefix name)",
        &d.prefix,
    );
    let isolate = toggle(
        &content,
        "Give each installer its own Windows environment",
        d.isolate_installers,
    );
    let mode = choice(
        &content,
        "GameMode performance tuning",
        &["Automatic for games", "Always", "Never"],
        toggle_index(d.gamemode),
    );
    let dxvk = choice(
        &content,
        "Direct3D 9/10/11 via DXVK",
        &["Automatic", "Always", "Never"],
        toggle_index(d.dxvk),
    );
    let vkd3d = choice(
        &content,
        "Direct3D 12 via VKD3D-Proton",
        &["Automatic", "Always", "Never"],
        toggle_index(d.vkd3d),
    );
    let hud = toggle(
        &content,
        "Show performance overlay (MangoHud / DXVK HUD)",
        d.hud,
    );
    let esync = toggle(&content, "Enable Wine esync", d.esync);
    let fsync = toggle(&content, "Enable Wine fsync", d.fsync);
    let debug = toggle(
        &content,
        "Keep Wine debug output for troubleshooting",
        d.debug,
    );
    let gamescope = toggle(&content, "Launch inside gamescope", d.gamescope);
    let fullscreen = toggle(
        &content,
        "gamescope fullscreen",
        d.gamescope_args.iter().any(|a| a == "-f"),
    );
    let width = number(
        &content,
        "gamescope output width (0 = automatic)",
        0.,
        16384.,
        crate::gaming::option(&d.gamescope_args, "-W").into(),
    );
    let height = number(
        &content,
        "gamescope output height (0 = automatic)",
        0.,
        16384.,
        crate::gaming::option(&d.gamescope_args, "-H").into(),
    );
    let fps = number(
        &content,
        "gamescope frame-rate limit (0 = default)",
        0.,
        1000.,
        crate::gaming::option(&d.gamescope_args, "-r").into(),
    );
    content.append(&note("gamescope arguments: one argument per line. Example: -f, -W, 1920, -H, 1080, -r, 60 (each on a separate line). Resolution, fullscreen and frame-rate controls above take precedence over matching arguments."));
    let args = gtk::TextView::new();
    args.buffer().set_text(&d.gamescope_args.join("\n"));
    args.set_monospace(true);
    args.set_height_request(130);
    content.append(&args);
    let save = gtk::Button::with_label("Save gaming preferences");
    content.append(&save);
    let status = note("Changes apply to the next segrun launch. Required tools must be installed.");
    content.append(&status);
    save.connect_clicked(move |_| {
        let result = (|| -> Result<(), String> {
            let mut updated = GameConfig::load(None).map_err(|e| format!("{e:#}"))?.0;
            runner.text().parse::<segrun::runner::Runner>()?;
            let prefix_text = prefix.text().trim().to_string();
            if prefix_text.is_empty()
                || !prefix_text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || prefix_text == "."
                || prefix_text == ".."
            {
                return Err(
                    "Use a prefix name containing letters, numbers, dots, hyphens and underscores."
                        .into(),
                );
            }
            let d = &mut updated.defaults;
            d.runner = runner.text().to_string();
            d.prefix = prefix_text;
            d.isolate_installers = isolate.is_active();
            d.gamemode = selected_toggle(&mode);
            d.dxvk = selected_toggle(&dxvk);
            d.vkd3d = selected_toggle(&vkd3d);
            d.hud = hud.is_active();
            d.esync = esync.is_active();
            d.fsync = fsync.is_active();
            d.debug = debug.is_active();
            d.gamescope = gamescope.is_active();
            let buffer = args.buffer();
            d.gamescope_args = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .lines()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .collect();
            crate::gaming::set_option(&mut d.gamescope_args, "-W", width.value_as_int() as u32);
            crate::gaming::set_option(&mut d.gamescope_args, "-H", height.value_as_int() as u32);
            crate::gaming::set_option(&mut d.gamescope_args, "-r", fps.value_as_int() as u32);
            d.gamescope_args.retain(|arg| arg != "-f");
            if fullscreen.is_active() {
                d.gamescope_args.push("-f".into());
            }
            if d.gamescope_args.iter().any(|s| s.contains('\0')) {
                return Err("Arguments cannot contain NUL characters.".into());
            }
            save_text(
                &GameConfig::user_path(),
                &toml::to_string_pretty(&updated).map_err(|e| e.to_string())?,
            )
        })();
        status.set_text(&match result {
            Ok(()) => "Gaming preferences saved. They apply to the next segrun launch.".into(),
            Err(e) => format!("Could not save: {e}"),
        });
    });
    content.append(&note("Installed gaming tools"));
    for command in [
        "wine",
        "umu-run",
        "gamemoderun",
        "mangohud",
        "gamescope",
        "steam",
        "lutris",
    ] {
        content.append(&note(&format!(
            "{command}: {}",
            if common::installed(command) {
                "available"
            } else {
                "not installed"
            }
        )));
    }
}
fn system_page(book: &gtk::Notebook) {
    let content = page(book, "System");
    content.append(&note("Open the system tools installed on your edition. These tools manage their own preferences and request authentication when needed."));
    for (name, icon, command, args) in [
        (
            "Network connections",
            "network-wireless-symbolic",
            "nm-connection-editor",
            vec![],
        ),
        (
            "Sound and microphone",
            "audio-volume-high-symbolic",
            "pavucontrol",
            vec![],
        ),
        (
            "Displays and resolution",
            "video-display-symbolic",
            "wdisplays",
            vec![],
        ),
        (
            "Bluetooth devices",
            "bluetooth-symbolic",
            "blueman-manager",
            vec![],
        ),
        (
            "Printers",
            "printer-symbolic",
            "system-config-printer",
            vec![],
        ),
        (
            "KDE system settings (Plasma session)",
            "preferences-system-symbolic",
            "systemsettings",
            vec![],
        ),
        (
            "Steam settings and library",
            "applications-games-symbolic",
            "steam",
            vec![],
        ),
        (
            "Lutris game library",
            "applications-games-symbolic",
            "lutris",
            vec![],
        ),
    ] {
        let button = common::command_button(name, icon, command, &args);
        if !common::installed(command) {
            button.set_sensitive(false);
            button.set_tooltip_text(Some(&format!("{command} is not installed on this system.")));
        }
        content.append(&button);
    }
    if !common::installed("nm-connection-editor") && common::installed("nmtui") {
        content.append(&common::command_button(
            "Network setup in terminal",
            "network-wireless-symbolic",
            common::TERMINAL,
            &["nmtui"],
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut all = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(current) = child {
            all.extend(descendants(&current));
            child = current.next_sibling();
        }
        all
    }
    #[test]
    #[ignore = "requires an isolated XDG_CONFIG_HOME and GTK display"]
    fn settings_save_validates_and_preserves_game_overrides() {
        let root =
            std::env::var("SEGLINUX_SETTINGS_TEST_ROOT").expect("isolated test directory required");
        assert!(Config::path().unwrap().starts_with(&root));
        assert!(GameConfig::user_path().starts_with(&root));
        gtk::init().unwrap();
        let app = gtk::Application::builder()
            .application_id("org.seglinux.Settings.Test")
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let game = GameConfig::parse(
            "[env]\nKEEP_ME = 'yes'\n[[app]]\nmatch = 'game.exe'\nrunner = 'umu'\n",
        )
        .unwrap();
        save_text(&GameConfig::user_path(), &toml::to_string(&game).unwrap()).unwrap();
        let config = Config::default();
        for theme in [Theme::Dark, Theme::Light] {
            let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let captured = errors.clone();
            let provider = gtk::CssProvider::new();
            provider.connect_parsing_error(move |_, _, error| {
                captured.borrow_mut().push(error.to_string())
            });
            provider.load_from_data(
                &Config {
                    theme,
                    ..config.clone()
                }
                .css(),
            );
            assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
        }
        open(&app, &config);
        let window = app
            .windows()
            .into_iter()
            .find(|w| w.title().as_deref() == Some("SegLinux Settings"))
            .unwrap();
        let all = descendants(window.upcast_ref());
        let accent = all
            .iter()
            .filter_map(|w| w.clone().downcast::<gtk::Entry>().ok())
            .find(|e| e.text() == "#3daee9")
            .unwrap();
        let button = |name: &str| {
            all.iter()
                .filter_map(|w| w.clone().downcast::<gtk::Button>().ok())
                .find(|b| b.label().as_deref() == Some(name))
                .unwrap()
        };
        accent.set_text("#abc123");
        button("Save desktop and appearance").emit_clicked();
        let before = std::fs::read_to_string(Config::path().unwrap()).unwrap();
        assert_eq!(toml::from_str::<Config>(&before).unwrap().accent, "#abc123");
        accent.set_text("invalid; CSS");
        button("Save desktop and appearance").emit_clicked();
        assert_eq!(
            std::fs::read_to_string(Config::path().unwrap()).unwrap(),
            before
        );
        button("Save gaming preferences").emit_clicked();
        let saved = GameConfig::load(None).unwrap().0;
        assert_eq!(saved.apps, game.apps);
        assert_eq!(saved.env, game.env);
        window.close();
    }
}
