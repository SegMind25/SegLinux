//! Appearance: theme and accent color, text size and animations, the
//! wallpaper and the dock's transparency.
use super::widgets::{self, index_of, selected};
use super::{Page, State};
use crate::config::{ACCENTS, Theme, WallpaperFit};
use gtk::{gdk, gio, glib, prelude::*};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const THEMES: [Theme; 2] = [Theme::Dark, Theme::Light];
const FITS: [WallpaperFit; 3] = [
    WallpaperFit::Cover,
    WallpaperFit::Contain,
    WallpaperFit::Fill,
];
/// Where the distribution's wallpapers are installed.
const BACKGROUNDS: &str = "/usr/share/backgrounds/seglinux";

pub struct Appearance {
    root: gtk::ScrolledWindow,
    theme: gtk::DropDown,
    accent: gtk::Entry,
    font: gtk::SpinButton,
    animations: gtk::Switch,
    wallpaper: gtk::Entry,
    /// Shows the thumbnails, with the given wallpaper among them.
    fill_gallery: Rc<dyn Fn(&Path)>,
    fit: gtk::DropDown,
    opacity: gtk::Scale,
}

/// The wallpapers offered as thumbnails: the distribution's, and the one in
/// use if it is elsewhere.
fn gallery_images(current: &Path) -> Vec<PathBuf> {
    let mut images: Vec<PathBuf> = std::fs::read_dir(BACKGROUNDS)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().is_some_and(|e| {
                        matches!(e.to_str(), Some("jpg" | "jpeg" | "png" | "webp"))
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    images.sort();
    if current.is_file() && !images.iter().any(|p| p == current) {
        images.push(current.to_path_buf());
    }
    images
}

fn thumbnail(path: &Path) -> Option<gtk::Picture> {
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 176, 99, true).ok()?;
    let picture = gtk::Picture::for_paintable(&gdk::Texture::for_pixbuf(&pixbuf));
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_size_request(176, 99);
    Some(picture)
}

impl Appearance {
    pub fn new(window: &gtk::ApplicationWindow) -> Self {
        let (root, page) = widgets::page();

        let colors = widgets::section(&page, "Colors", None);
        let theme = widgets::dropdown(
            &colors,
            "Desktop theme",
            Some(
                "Dock, menus, the desktop's dialogs and GTK applications that follow the system's dark preference.",
            ),
            &["Dark", "Light"],
        );
        let accent = widgets::entry(
            &colors,
            "Accent color",
            Some("Focus rings, running applications and selections. A six-digit hex color."),
            "#3daee9",
        );
        accent.set_widget_name("accent");
        accent.set_width_chars(10);
        let swatches = gtk::FlowBox::new();
        swatches.set_selection_mode(gtk::SelectionMode::None);
        swatches.set_max_children_per_line(10);
        swatches.set_min_children_per_line(5);
        swatches.set_halign(gtk::Align::Start);
        swatches.set_column_spacing(6);
        swatches.add_css_class("swatches");
        for (index, (name, color)) in ACCENTS.iter().enumerate() {
            let button = gtk::Button::new();
            button.add_css_class("swatch");
            button.set_halign(gtk::Align::Center);
            button.set_valign(gtk::Align::Center);
            button.add_css_class(&format!("swatch-{index}"));
            button.set_tooltip_text(Some(name));
            button.update_property(&[gtk::accessible::Property::Label(name)]);
            let target = accent.clone();
            button.connect_clicked(move |_| target.set_text(color));
            swatches.insert(&button, -1);
        }
        colors.append(&swatches);

        let text = widgets::section(&page, "Text and motion", None);
        let font = widgets::spin(
            &text,
            "Text size",
            Some("Of the dock, menus and Settings, in pixels."),
            (10.0, 24.0),
            1.0,
        );
        let animations = widgets::switch(
            &text,
            "Animations",
            Some("Menus and pages slide and fade. Turn off for a snappier feel on slow computers."),
        );

        let wall = widgets::section(&page, "Wallpaper", None);
        let wallpaper = gtk::Entry::new();
        wallpaper.set_widget_name("wallpaper");
        wallpaper.set_hexpand(true);
        let gallery = gtk::FlowBox::new();
        gallery.set_selection_mode(gtk::SelectionMode::None);
        gallery.set_max_children_per_line(4);
        gallery.set_column_spacing(10);
        gallery.set_row_spacing(10);
        gallery.add_css_class("wallpapers");
        wall.append(&gallery);
        let fill_gallery: Rc<dyn Fn(&Path)> = Rc::new({
            let gallery = gallery.clone();
            let target = wallpaper.clone();
            move |current: &Path| {
                while let Some(child) = gallery.first_child() {
                    gallery.remove(&child);
                }
                for path in gallery_images(current) {
                    let Some(picture) = thumbnail(&path) else {
                        continue;
                    };
                    let button = gtk::Button::new();
                    button.add_css_class("wallpaper-choice");
                    button.set_child(Some(&picture));
                    let name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    button.set_tooltip_text(Some(&name));
                    button.update_property(&[gtk::accessible::Property::Label(&format!(
                        "Wallpaper {name}"
                    ))]);
                    let target = target.clone();
                    button.connect_clicked(move |_| target.set_text(&path.to_string_lossy()));
                    gallery.insert(&button, -1);
                }
                gallery.set_visible(gallery.first_child().is_some());
            }
        });
        let path_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        path_row.add_css_class("settings-row");
        path_row.append(&wallpaper);
        let browse = gtk::Button::with_label("Choose image…");
        path_row.append(&browse);
        wall.append(&path_row);
        let parent = window.downgrade();
        let target = wallpaper.clone();
        let fill = fill_gallery.clone();
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
            if let Some(pictures) = glib::user_special_dir(glib::UserDirectory::Pictures) {
                let _ = dialog.set_current_folder(Some(&gio::File::for_path(pictures)));
            }
            let target = target.clone();
            let fill_gallery = fill.clone();
            dialog.connect_response(move |dialog, response| {
                if response == gtk::ResponseType::Accept
                    && let Some(path) = dialog.file().and_then(|f| f.path())
                {
                    target.set_text(&path.to_string_lossy());
                    fill_gallery(&path);
                }
                dialog.destroy();
            });
            dialog.show();
        });
        let fit = widgets::dropdown(
            &wall,
            "Layout",
            Some("Cover fills the screen and may crop the image."),
            &["Cover", "Fit inside the screen", "Stretch"],
        );

        let dock = widgets::section(&page, "Dock and menus", None);
        let opacity = widgets::scale(
            &dock,
            "Opacity",
            Some("Of the dock's background, in percent: lower shows more of the wallpaper."),
            (30.0, 100.0),
            1.0,
        );
        opacity.set_widget_name("dock-opacity");

        Self {
            root,
            theme,
            accent,
            font,
            animations,
            wallpaper,
            fill_gallery,
            fit,
            opacity,
        }
    }
}

impl Page for Appearance {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn load(&self, state: &State) {
        let c = &state.shell;
        self.theme.set_selected(index_of(&THEMES, &c.theme));
        self.accent.set_text(&c.accent);
        self.font.set_value(c.font_size.into());
        self.animations.set_active(c.animations);
        self.wallpaper.set_text(&c.wallpaper.to_string_lossy());
        (self.fill_gallery)(&c.wallpaper);
        self.fit.set_selected(index_of(&FITS, &c.wallpaper_fit));
        self.opacity.set_value(c.dock_opacity.into());
    }

    fn store(&self, state: &mut State) -> Result<(), String> {
        let c = &mut state.shell;
        c.theme = selected(&THEMES, &self.theme);
        c.accent = self.accent.text().trim().to_lowercase();
        c.font_size = widgets::spin_u32(&self.font);
        c.animations = self.animations.is_active();
        c.wallpaper = PathBuf::from(self.wallpaper.text().trim());
        c.wallpaper_fit = selected(&FITS, &self.fit);
        c.dock_opacity = self.opacity.value().round() as u32;
        Ok(())
    }
}

/// Colors of the accent swatches, for the Settings window's style sheet.
pub fn swatch_css() -> String {
    ACCENTS
        .iter()
        .enumerate()
        .map(|(i, (_, color))| format!(".swatch-{i} {{ background: {color}; }}\n"))
        .collect()
}
