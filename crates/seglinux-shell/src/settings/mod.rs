//! SegLinux Settings: the desktop's appearance, desktop and dock, windows
//! and input devices, gaming, and the system tools, in one window.
//!
//! Every page reads its controls into a copy of the settings; **Apply**
//! checks the whole copy and only then writes what changed: the shell's
//! `shell.toml` (which the running desktop applies at once), the
//! compositor's files, segrun's `segrun.toml` and MangoHud's configuration.
mod appearance;
mod desktop;
mod gaming;
mod system;
mod widgets;
mod windows;

use crate::config::Config;
use crate::{compositor, managed};
use gtk::{gio, glib, prelude::*};
use segrun::config::Config as GameConfig;
use std::path::Path;
use std::{cell::RefCell, rc::Rc};

/// Everything the Settings window edits.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub shell: Config,
    /// segrun's configuration, or why it could not be read.
    pub games: Result<GameConfig, String>,
}

impl State {
    fn load(shell: &Config) -> Self {
        Self {
            shell: shell.clone(),
            games: GameConfig::load(None)
                .map(|(config, _)| config)
                .map_err(|e| format!("{e:#}")),
        }
    }

    /// The defaults, keeping the user's own data: game profiles and
    /// environment variables are not settings to reset.
    fn defaults(&self) -> Self {
        Self {
            shell: Config::default(),
            games: self
                .games
                .as_ref()
                .map_err(Clone::clone)
                .map(|games| GameConfig {
                    defaults: Default::default(),
                    ..games.clone()
                }),
        }
    }
}

/// One page of the window.
pub trait Page {
    /// The page's content.
    fn widget(&self) -> gtk::Widget;
    /// Shows `state` in the page's controls.
    fn load(&self, state: &State);
    /// Copies the page's controls into `state`.
    fn store(&self, state: &mut State) -> Result<(), String>;
}

/// Writes `text` to `path` atomically (a new file renamed over the old one).
pub fn save_text(path: &Path, text: &str) -> Result<(), String> {
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
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Checks what the pages cannot check alone.
fn check(next: &State, previous: &State) -> Result<(), String> {
    next.shell.validate()?;
    glib::DateTime::now_local()
        .and_then(|now| now.format(&next.shell.clock_format))
        .map_err(|_| "The clock format is not valid.".to_string())?;
    let wallpaper = &next.shell.wallpaper;
    // An unchanged missing default wallpaper should not block unrelated preferences.
    if *wallpaper != previous.shell.wallpaper
        && (!wallpaper.is_absolute() || gtk::gdk::Texture::from_filename(wallpaper).is_err())
    {
        return Err("Choose an existing image for the wallpaper.".into());
    }
    Ok(())
}

/// Saves `next` (already checked) where it differs from `previous`;
/// returns notes for the user.
fn write(next: &State, previous: &State) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    if next.shell != previous.shell {
        save_text(&Config::path(), &next.shell.to_toml())?;
    }
    let synced = compositor::sync(&next.shell).map_err(|e| format!("Window settings: {e}"))?;
    for path in synced.kept {
        notes.push(format!(
            "{} was edited by hand and was not changed.",
            path.display()
        ));
    }
    if let (Ok(games), Ok(before)) = (&next.games, &previous.games)
        && games != before
    {
        save_text(
            &GameConfig::user_path(),
            &toml::to_string_pretty(games).map_err(|e| e.to_string())?,
        )?;
    }
    let mangohud = crate::gaming::mangohud_path();
    match managed::update(&mangohud, &crate::gaming::mangohud_conf(&next.shell.overlay)) {
        Ok(managed::Outcome::Kept) if next.shell.overlay != previous.shell.overlay => notes.push(format!(
            "{} is your own MangoHud configuration; the overlay settings were not applied to it.",
            mangohud.display()
        )),
        Ok(_) => {}
        Err(e) => return Err(format!("{}: {e}", mangohud.display())),
    }
    Ok(notes)
}

/// Pages in the order of the sidebar: id, title, page.
type Pages = Vec<(&'static str, &'static str, Rc<dyn Page>)>;

pub fn open(app: &gtk::Application, config: &Config, page: &str, css: &gtk::CssProvider) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("SegLinux Settings")
        .default_width(980)
        .default_height(720)
        .build();
    window.add_css_class("settings-window");
    let swatches = gtk::CssProvider::new();
    swatches.load_from_data(&appearance::swatch_css());
    gtk::style_context_add_provider_for_display(
        &WidgetExt::display(&window),
        &swatches,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let state = Rc::new(RefCell::new(State::load(config)));

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_hexpand(true);
    let pages: Pages = vec![
        (
            "appearance",
            "Appearance",
            Rc::new(appearance::Appearance::new(&window)),
        ),
        (
            "desktop",
            "Desktop & dock",
            Rc::new(desktop::Desktop::new()),
        ),
        (
            "windows",
            "Windows & input",
            Rc::new(windows::Windows::new()),
        ),
        (
            "gaming",
            "Gaming",
            Rc::new(gaming::Gaming::new(&state.borrow())),
        ),
        ("system", "System", Rc::new(system::System::new())),
    ];
    for (id, title, page) in &pages {
        page.load(&state.borrow());
        stack.add_titled(&page.widget(), Some(id), title);
    }
    if stack.child_by_name(page).is_some() {
        stack.set_visible_child_name(page);
    }

    let sidebar = gtk::StackSidebar::new();
    sidebar.set_stack(&stack);
    sidebar.set_size_request(210, -1);
    sidebar.add_css_class("settings-sidebar");

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&sidebar);
    body.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    body.append(&stack);
    body.set_vexpand(true);

    let status = gtk::Label::new(Some("Changes take effect when you select Apply."));
    status.set_widget_name("status");
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.set_hexpand(true);
    status.add_css_class("dim-label");
    let reset = gtk::Button::with_label("Restore page defaults");
    reset.set_widget_name("reset");
    reset.set_tooltip_text(Some(
        "Shows the defaults of this page; nothing is saved until you select Apply.",
    ));
    let apply = gtk::Button::with_label("Apply");
    apply.set_widget_name("apply");
    apply.add_css_class("suggested-action");
    let bar = gtk::ActionBar::new();
    bar.pack_start(&status);
    bar.pack_end(&apply);
    bar.pack_end(&reset);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&body);
    root.append(&bar);
    window.set_child(Some(&root));

    let pages = Rc::new(pages);
    {
        let (pages, stack, state, status) =
            (pages.clone(), stack.clone(), state.clone(), status.clone());
        reset.connect_clicked(move |_| {
            let visible = stack.visible_child_name();
            if let Some((_, title, page)) = pages
                .iter()
                .find(|(id, _, _)| visible.as_deref() == Some(*id))
            {
                page.load(&state.borrow().defaults());
                status.set_text(&format!(
                    "{title}: defaults shown. Select Apply to use them."
                ));
            }
        });
    }
    let css = css.clone();
    apply.connect_clicked(move |_| {
        let previous = state.borrow().clone();
        let mut next = previous.clone();
        let result = pages
            .iter()
            .try_for_each(|(_, title, page)| {
                page.store(&mut next).map_err(|e| format!("{title}: {e}"))
            })
            .and_then(|()| check(&next, &previous))
            .and_then(|()| write(&next, &previous));
        match result {
            Ok(notes) => {
                // The Settings window follows the new look at once, too.
                css.load_from_data(&next.shell.css());
                if let Some(settings) = gtk::Settings::default() {
                    crate::set_dark_theme(&settings, next.shell.theme == crate::config::Theme::Dark);
                }
                let changed = next != previous;
                *state.borrow_mut() = next;
                let mut text = if changed {
                    "Saved. The desktop uses the new settings now; games use them from their next start.".to_string()
                } else {
                    "No changes to save.".to_string()
                };
                for note in notes {
                    text.push(' ');
                    text.push_str(&note);
                }
                status.set_text(&text);
                status.remove_css_class("error");
            }
            Err(error) => {
                status.set_text(&format!("Not saved: {error}"));
                status.add_css_class("error");
            }
        }
    });
    window.present();
}

#[cfg(test)]
mod tests;
