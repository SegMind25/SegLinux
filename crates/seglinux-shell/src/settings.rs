use crate::config::{CLOCK_12H, CLOCK_24H, Config};
use gtk::{gio, prelude::*};

pub fn open(app: &gtk::Application, config: &Config) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("SegLinux Desktop Settings")
        .default_width(560)
        .default_height(300)
        .build();
    window.add_css_class("settings-window");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    for side in [
        gtk::PositionType::Top,
        gtk::PositionType::Bottom,
        gtk::PositionType::Left,
        gtk::PositionType::Right,
    ] {
        match side {
            gtk::PositionType::Top => content.set_margin_top(28),
            gtk::PositionType::Bottom => content.set_margin_bottom(28),
            gtk::PositionType::Left => content.set_margin_start(28),
            _ => content.set_margin_end(28),
        }
    }
    content.append(&gtk::Label::new(Some("Desktop appearance")));
    content.append(&gtk::Label::new(Some("Wallpaper image (absolute path)")));
    let wallpaper = gtk::Entry::new();
    wallpaper.set_text(&config.wallpaper.to_string_lossy());
    content.append(&wallpaper);
    let clock = gtk::CheckButton::with_label("Use a 12-hour clock");
    clock.set_active(config.clock_format.contains("%I"));
    content.append(&clock);
    let icons = gtk::CheckButton::with_label("Show the files of the Desktop folder on the desktop");
    icons.set_active(config.desktop_icons);
    content.append(&icons);
    let saved = config.clone();
    let save = gtk::Button::with_label("Save preferences");
    content.append(&save);
    let status = gtk::Label::new(Some("Changes apply the next time you sign in."));
    status.set_wrap(true);
    content.append(&status);
    save.connect_clicked(move |_| {
        let path = std::path::PathBuf::from(wallpaper.text().as_str());
        if !path.is_absolute() || !path.is_file() {
            status.set_text("Choose an existing image using its absolute path.");
            return;
        }
        if gtk::gdk::Texture::from_filename(&path).is_err() {
            status.set_text("This file could not be loaded as an image.");
            return;
        }
        let config = Config {
            wallpaper: path,
            clock_format: if clock.is_active() {
                CLOCK_12H
            } else {
                CLOCK_24H
            }
            .into(),
            desktop_icons: icons.is_active(),
            ..saved.clone()
        };
        let result = (|| -> Result<(), String> {
            let path = Config::path().ok_or("HOME or XDG_CONFIG_HOME must be set")?;
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let text = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;
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
        })();
        match result {
            Ok(()) => status.set_text("Saved. Your changes will appear next time you sign in."),
            Err(error) => status.set_text(&format!("Could not save: {error}")),
        }
    });
    window.set_child(Some(&content));
    window.present();
}
