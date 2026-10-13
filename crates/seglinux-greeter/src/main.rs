//! SegLinux's sign-in screen: a GTK 4 greeter for greetd, in place of SDDM.
//!
//! greetd starts it as its `greeter` user, inside labwc
//! (`seglinux-greeter-session`). It shows the clock over the wallpaper, the
//! people's accounts, a password field, the desktop session to start, and
//! sleep, restart and shut down. greetd checks the password with PAM; once
//! it agrees, the greeter names the session and exits, and greetd starts it.
//!
//! `seglinux-greeter --demo` runs anywhere, in a window, without greetd: the
//! password "demo" signs in, and the session it would start is printed.

mod greetd;
mod sessions;
mod state;
mod users;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::Deserialize;

use greetd::Failure;
use sessions::Session;
use users::User;

const APP_ID: &str = "org.seglinux.Greeter";
const CSS: &str = include_str!("style.css");
const CONFIG: &str = "/etc/seglinux/greeter.toml";
const SESSIONS: &str = "/usr/share/wayland-sessions";
const DEFAULT_WALLPAPER: &str = "/usr/share/backgrounds/seglinux/seglinux-dark.jpg";

/// /etc/seglinux/greeter.toml: how the sign-in screen looks.
#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct Config {
    wallpaper: String,
    /// The color of the focus ring and buttons, as #rrggbb.
    accent: String,
    /// The clock, in strftime(3) notation: "%H:%M" (24-hour) or "%I:%M %p".
    clock_format: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wallpaper: DEFAULT_WALLPAPER.into(),
            accent: "#3daee9".into(),
            clock_format: "%H:%M".into(),
        }
    }
}

impl Config {
    fn load(path: &Path) -> Config {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Config::default();
        };
        match toml::from_str::<Config>(&text) {
            Ok(config) if is_color(&config.accent) => config,
            Ok(_) => {
                eprintln!("{}: accent must be a color such as #3daee9", path.display());
                Config::default()
            }
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                Config::default()
            }
        }
    }

    fn css(&self) -> String {
        CSS.replace("#3daee9", &self.accent)
    }
}

fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Where sign-ins go: greetd, or nowhere in a demonstration.
#[derive(Clone)]
enum Backend {
    Greetd(PathBuf),
    Demo,
}

impl Backend {
    /// Signs in (in a worker thread: PAM can take seconds to refuse a
    /// password). Returns PAM's notices, or why it did not work.
    async fn sign_in(
        &self,
        user: String,
        password: String,
        session: Session,
    ) -> Result<Vec<String>, Failure> {
        let backend = self.clone();
        gio::spawn_blocking(move || {
            let mut notices = Vec::new();
            match backend {
                Backend::Greetd(socket) => {
                    let mut client = greetd::Client::connect(&socket).map_err(|e| {
                        Failure::Broken(format!("The sign-in service cannot be reached: {e}"))
                    })?;
                    let env = session.env();
                    client.sign_in(&user, &password, session.command, env, |n| {
                        notices.push(n.to_string())
                    })?;
                }
                Backend::Demo => {
                    std::thread::sleep(Duration::from_millis(700));
                    if password != "demo" {
                        return Err(Failure::WrongPassword);
                    }
                    println!(
                        "Would start {:?} for {user} with {:?}",
                        session.command,
                        session.env()
                    );
                }
            }
            Ok(notices)
        })
        .await
        .unwrap_or_else(|_| Err(Failure::Broken("Signing in stopped unexpectedly.".into())))
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!(
            "Usage: seglinux-greeter [--demo]\n\n\
             SegLinux's sign-in screen, started by greetd (seglinux-greeter-session).\n\
             --demo  run in a window without greetd; the password \"demo\" signs in\n\n\
             Its look is set in {CONFIG}: wallpaper, accent, clock_format."
        );
        return glib::ExitCode::SUCCESS;
    }
    let demo = args.iter().any(|a| a == "--demo");
    let backend = match std::env::var_os("GREETD_SOCK") {
        _ if demo => Backend::Demo,
        Some(socket) => Backend::Greetd(socket.into()),
        None => {
            eprintln!(
                "seglinux-greeter: GREETD_SOCK is not set; greetd starts this program. Use --demo to try it."
            );
            return glib::ExitCode::FAILURE;
        }
    };
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| activate(app, backend.clone()));
    // Its own arguments only: GTK does not need to see --demo.
    app.run_with_args(&["seglinux-greeter"])
}

/// GTK 4.20 replaced gtk-application-prefer-dark-theme with
/// gtk-interface-color-scheme and warns when the old setting is used.
fn prefer_dark(settings: &gtk::Settings) {
    const SCHEME: &str = "gtk-interface-color-scheme";
    let scheme = settings
        .find_property(SCHEME)
        .and_then(|pspec| glib::EnumClass::with_type(pspec.value_type()))
        .and_then(|class| class.to_value_by_nick("dark"));
    match scheme {
        Some(value) => settings.set_property_from_value(SCHEME, &value),
        None => settings.set_gtk_application_prefer_dark_theme(true),
    }
}

fn activate(app: &gtk::Application, backend: Backend) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let config = Config::load(Path::new(CONFIG));
    let css = gtk::CssProvider::new();
    css.load_from_data(&config.css());
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    if let Some(settings) = gtk::Settings::default() {
        prefer_dark(&settings);
    }

    let demo = matches!(backend, Backend::Demo);
    let mut people = users::list();
    let mut desktops = sessions::list(Path::new(SESSIONS));
    if demo {
        people = users::parse(
            "alice:x:1000:1000:Alice Martin:/home/alice:/bin/bash\nsam:x:1001:1001:Sam:/home/sam:/bin/bash\n",
        );
        if desktops.is_empty() {
            desktops = [
                "SegLinux:seglinux-session",
                "Plasma (Wayland):startplasma-wayland",
            ]
            .iter()
            .filter_map(|s| s.split_once(':'))
            .filter_map(|(name, exec)| {
                sessions::parse(
                    exec,
                    &format!("[Desktop Entry]\nName={name}\nExec={exec}\n"),
                )
            })
            .collect();
        }
    }
    let state = state::State::load(Path::new(state::PATH));
    let layer = !demo && gtk4_layer_shell::is_supported();
    let monitors = display.monitors();
    // The wallpaper, decoded once at the size of the largest monitor and
    // shared by all of them.
    let largest = (0..monitors.n_items())
        .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
        .map(|m| {
            let g = m.geometry();
            (g.width() * m.scale_factor(), g.height() * m.scale_factor())
        })
        .max_by_key(|(w, h)| i64::from(*w) * i64::from(*h))
        .filter(|_| !demo)
        .unwrap_or((1280, 800));
    let background = Background::load(&config.wallpaper, largest);
    let screen = Screen::new(app, &config, &background, backend, people, desktops, &state);

    let main = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Sign in to SegLinux")
        .child(&screen.root)
        .build();
    if layer {
        // Over the whole screen, on every monitor: the sign-in on the
        // first, the wallpaper on the others.
        cover(
            &main,
            monitors.item(0).and_downcast::<gdk::Monitor>().as_ref(),
        );
        main.set_keyboard_mode(KeyboardMode::Exclusive);
        for index in 1..monitors.n_items() {
            let other = gtk::ApplicationWindow::builder()
                .application(app)
                .child(&background.widget())
                .build();
            cover(
                &other,
                monitors.item(index).and_downcast::<gdk::Monitor>().as_ref(),
            );
            other.present();
        }
    } else if demo {
        main.set_default_size(1280, 800);
    } else {
        main.fullscreen();
    }
    main.present();
    screen.focus();
    // The handlers hold the screen weakly; the window keeps it.
    main.connect_destroy(move |_| {
        let _ = &screen;
    });
}

/// Makes `window` a layer-shell surface over all of `monitor`.
fn cover(window: &gtk::ApplicationWindow, monitor: Option<&gdk::Monitor>) {
    window.init_layer_shell();
    window.set_namespace(Some("seglinux-greeter"));
    window.set_layer(Layer::Overlay);
    window.set_monitor(monitor);
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
}

/// The picture behind the sign-in screen.
struct Background(Option<gdk::Texture>);

impl Background {
    fn load(path: &str, screen: (i32, i32)) -> Background {
        use gtk::gdk_pixbuf::Pixbuf;
        let texture = Pixbuf::file_info(path)
            .filter(|(_, w, h)| *w > 0 && *h > 0)
            .and_then(|(_, w, h)| {
                let (w, h) = cover_size((w, h), screen);
                Pixbuf::from_file_at_scale(path, w, h, true).ok()
            })
            .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf));
        Background(texture)
    }

    fn widget(&self) -> gtk::Widget {
        match &self.0 {
            Some(texture) => {
                let picture = gtk::Picture::for_paintable(texture);
                picture.set_content_fit(gtk::ContentFit::Cover);
                picture.set_can_shrink(true);
                picture.upcast()
            }
            None => {
                // The desktop's own colors when there is no picture.
                let fallback = gtk::Box::new(gtk::Orientation::Vertical, 0);
                fallback.add_css_class("no-wallpaper");
                fallback.upcast()
            }
        }
    }
}

/// The smallest size, keeping the image's proportions, that covers the
/// screen; never larger than the image.
fn cover_size(image: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
    let scale = (f64::from(screen.0) / f64::from(image.0))
        .max(f64::from(screen.1) / f64::from(image.1))
        .min(1.0);
    (
        (f64::from(image.0) * scale).ceil() as i32,
        (f64::from(image.1) * scale).ceil() as i32,
    )
}

/// The sign-in screen and its state.
struct Screen {
    root: gtk::Overlay,
    users: Vec<User>,
    sessions: Vec<Session>,
    selected: Cell<usize>,
    user_buttons: Vec<gtk::Button>,
    avatar: gtk::Label,
    name: gtk::Label,
    other_name: gtk::Entry,
    password: gtk::PasswordEntry,
    sign_in: gtk::Button,
    spinner: gtk::Spinner,
    message: gtk::Label,
    session: gtk::DropDown,
    error: RefCell<Option<String>>,
    busy: Cell<bool>,
    backend: Backend,
}

impl Screen {
    fn new(
        app: &gtk::Application,
        config: &Config,
        background: &Background,
        backend: Backend,
        users: Vec<User>,
        sessions: Vec<Session>,
        state: &state::State,
    ) -> Rc<Screen> {
        let root = gtk::Overlay::new();
        root.add_css_class("greeter");
        root.set_child(Some(&background.widget()));
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("scrim");
        root.add_overlay(&scrim);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.set_halign(gtk::Align::Center);
        column.set_valign(gtk::Align::Center);
        column.add_css_class("column");
        root.add_overlay(&column);

        let clock = gtk::Label::new(None);
        clock.add_css_class("clock");
        let date = gtk::Label::new(None);
        date.add_css_class("date");
        column.append(&clock);
        column.append(&date);
        {
            let format = config.clock_format.clone();
            let tick = move || {
                if let Ok(now) = glib::DateTime::now_local() {
                    clock.set_text(&now.format(&format).unwrap_or_default());
                    date.set_text(
                        &now.format("%A %e %B")
                            .unwrap_or_default()
                            .replace("  ", " "),
                    );
                }
            };
            tick();
            glib::timeout_add_seconds_local(1, move || {
                tick();
                glib::ControlFlow::Continue
            });
        }

        let avatar = gtk::Label::new(None);
        avatar.add_css_class("avatar");
        avatar.set_halign(gtk::Align::Center);
        avatar.set_valign(gtk::Align::Center);
        let name = gtk::Label::new(None);
        name.add_css_class("user-name");
        let other_name = gtk::Entry::builder()
            .placeholder_text("User name")
            .visible(false)
            .build();
        other_name.add_css_class("field");
        let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
        card.add_css_class("card");
        card.append(&avatar);
        card.append(&name);
        card.append(&other_name);

        let password = gtk::PasswordEntry::builder()
            .placeholder_text("Password")
            .show_peek_icon(true)
            .hexpand(true)
            .build();
        password.add_css_class("field");
        let sign_in = gtk::Button::from_icon_name("go-next-symbolic");
        sign_in.set_tooltip_text(Some("Sign in"));
        sign_in.add_css_class("suggested-action");
        sign_in.add_css_class("sign-in");
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        let entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        entry_row.append(&password);
        entry_row.append(&sign_in);
        entry_row.append(&spinner);
        card.append(&entry_row);

        let message = gtk::Label::new(None);
        message.add_css_class("message");
        message.set_wrap(true);
        message.set_max_width_chars(40);
        message.set_justify(gtk::Justification::Center);
        card.append(&message);
        column.append(&card);

        // Several accounts: a row of them to choose from, and "Other" for
        // one that is not listed.
        let chooser = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chooser.add_css_class("users");
        chooser.set_halign(gtk::Align::Center);
        let mut user_buttons = Vec::new();
        for user in &users {
            let button = person_button(&users::initial(user), &user.display_name);
            chooser.append(&button);
            user_buttons.push(button);
        }
        let other = person_button("+", "Other user");
        chooser.append(&other);
        // One account: nothing to choose, unless there are none at all.
        chooser.set_visible(users.len() != 1);
        column.append(&chooser);

        // The bottom bar: the session on the left, power on the right.
        let bar = gtk::CenterBox::new();
        bar.add_css_class("bar");
        bar.set_valign(gtk::Align::End);
        let session_names: Vec<&str> = sessions.iter().map(|s| s.name.as_str()).collect();
        let session = gtk::DropDown::from_strings(&session_names);
        session.set_tooltip_text(Some("The desktop to start"));
        session.add_css_class("session");
        let session_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let session_icon = gtk::Image::from_icon_name("preferences-desktop-display-symbolic");
        session_box.append(&session_icon);
        session_box.append(&session);
        bar.set_start_widget(Some(&session_box));
        let power = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        power.add_css_class("power");
        bar.set_end_widget(Some(&power));
        root.add_overlay(&bar);

        if let Some(i) = sessions.iter().position(|s| s.id == state.session) {
            session.set_selected(i as u32);
        }
        let first = users.iter().position(|u| u.name == state.user).unwrap_or(0);

        let screen = Rc::new(Screen {
            root,
            users,
            sessions,
            selected: Cell::new(first),
            user_buttons,
            avatar,
            name,
            other_name,
            password,
            sign_in,
            spinner,
            message,
            session,
            error: RefCell::default(),
            busy: Cell::new(false),
            backend,
        });

        for (index, button) in screen.user_buttons.iter().enumerate() {
            let s = Rc::downgrade(&screen);
            button.connect_clicked(move |_| {
                if let Some(s) = s.upgrade() {
                    s.choose(Some(index));
                }
            });
        }
        {
            let s = Rc::downgrade(&screen);
            other.connect_clicked(move |_| {
                if let Some(s) = s.upgrade() {
                    s.choose(None);
                }
            });
        }
        for (icon, label, verb) in [
            ("weather-clear-night-symbolic", "Sleep", "suspend"),
            ("system-reboot-symbolic", "Restart", "reboot"),
            ("system-shutdown-symbolic", "Shut down", "poweroff"),
        ] {
            let button = gtk::Button::new();
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.append(&gtk::Image::from_icon_name(icon));
            content.append(&gtk::Label::new(Some(label)));
            button.set_child(Some(&content));
            let s = Rc::downgrade(&screen);
            button.connect_clicked(move |_| {
                if let Some(s) = s.upgrade() {
                    s.power(verb);
                }
            });
            power.append(&button);
        }
        {
            let s = Rc::downgrade(&screen);
            let app = app.clone();
            let go = move || {
                if let Some(s) = s.upgrade() {
                    let app = app.clone();
                    glib::spawn_future_local(async move { s.submit(&app).await });
                }
            };
            let go = Rc::new(go);
            let g = go.clone();
            screen.password.connect_activate(move |_| g());
            let g = go.clone();
            screen.sign_in.connect_clicked(move |_| g());
            let s = Rc::downgrade(&screen);
            screen.other_name.connect_activate(move |_| {
                if let Some(s) = s.upgrade() {
                    s.password.grab_focus();
                }
            });
        }
        {
            // A new attempt clears the last error.
            let s = Rc::downgrade(&screen);
            screen.password.connect_changed(move |_| {
                if let Some(s) = s.upgrade()
                    && s.error.borrow_mut().take().is_some()
                {
                    s.show_message();
                }
            });
        }
        // Caps Lock, which makes so many passwords "wrong".
        if let Some(keyboard) = gdk::Display::default()
            .and_then(|d| d.default_seat())
            .and_then(|seat| seat.keyboard())
        {
            let s = Rc::downgrade(&screen);
            keyboard.connect_caps_lock_state_notify(move |_| {
                if let Some(s) = s.upgrade() {
                    s.show_message();
                }
            });
        }
        let first = (!screen.users.is_empty()).then_some(first);
        screen.choose(first);
        screen
    }

    fn focus(&self) {
        if WidgetExt::is_visible(&self.other_name) {
            self.other_name.grab_focus();
        } else {
            self.password.grab_focus();
        }
    }

    /// Shows the account `index`, or a field for a user name (None).
    fn choose(&self, index: Option<usize>) {
        for (i, button) in self.user_buttons.iter().enumerate() {
            if Some(i) == index {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
        match index.and_then(|i| self.users.get(i).map(|u| (i, u))) {
            Some((i, user)) => {
                self.selected.set(i);
                self.avatar.set_text(&users::initial(user));
                self.name.set_text(&user.display_name);
                self.name.set_visible(true);
                self.other_name.set_visible(false);
            }
            None => {
                self.selected.set(usize::MAX);
                self.avatar.set_text("+");
                self.name.set_visible(false);
                self.other_name.set_visible(true);
            }
        }
        self.password.set_text("");
        self.error.replace(None);
        self.show_message();
        self.focus();
    }

    fn user_name(&self) -> String {
        match self.users.get(self.selected.get()) {
            Some(user) => user.name.clone(),
            None => self.other_name.text().trim().to_string(),
        }
    }

    /// The error of the last attempt, else a Caps Lock warning.
    fn show_message(&self) {
        let caps = gdk::Display::default()
            .and_then(|d| d.default_seat())
            .and_then(|seat| seat.keyboard())
            .is_some_and(|k| k.is_caps_locked());
        let (text, class) = match &*self.error.borrow() {
            Some(error) => (error.clone(), Some("error")),
            None if caps => ("Caps Lock is on.".to_string(), Some("warning")),
            None => (String::new(), None),
        };
        self.message.set_text(&text);
        self.message.remove_css_class("error");
        self.message.remove_css_class("warning");
        if let Some(class) = class {
            self.message.add_css_class(class);
        }
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.spinner.set_visible(busy);
        self.spinner.set_spinning(busy);
        self.sign_in.set_visible(!busy);
        self.password.set_sensitive(!busy);
        self.other_name.set_sensitive(!busy);
        self.session.set_sensitive(!busy);
        for button in &self.user_buttons {
            button.set_sensitive(!busy);
        }
    }

    async fn submit(self: Rc<Self>, app: &gtk::Application) {
        if self.busy.get() {
            return;
        }
        let user = self.user_name();
        if user.is_empty() {
            self.error.replace(Some("Type the user name first.".into()));
            self.show_message();
            self.other_name.grab_focus();
            return;
        }
        let Some(session) = self.sessions.get(self.session.selected() as usize).cloned() else {
            self.error
                .replace(Some("No desktop session is installed.".into()));
            self.show_message();
            return;
        };
        self.error.replace(None);
        self.show_message();
        self.set_busy(true);
        let password = self.password.text().to_string();
        let result = self
            .backend
            .sign_in(user.clone(), password, session.clone())
            .await;
        match result {
            Ok(_notices) => {
                state::State {
                    user,
                    session: session.id,
                }
                .save(Path::new(state::PATH));
                // greetd starts the session once the greeter has gone.
                app.quit();
            }
            Err(failure) => {
                self.set_busy(false);
                self.password.set_text("");
                self.error.replace(Some(failure.to_string()));
                self.show_message();
                self.focus();
                if failure == Failure::WrongPassword {
                    self.password.add_css_class("shake");
                    let p = self.password.clone();
                    glib::timeout_add_local_once(Duration::from_millis(450), move || {
                        p.remove_css_class("shake");
                    });
                }
            }
        }
    }

    /// Sleep, restart or shut down, through elogind (the active session
    /// on a seat may, without a password).
    fn power(self: &Rc<Self>, verb: &'static str) {
        if matches!(self.backend, Backend::Demo) {
            self.error
                .replace(Some(format!("Demo: this would run loginctl {verb}.")));
            self.show_message();
            return;
        }
        let s = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                std::process::Command::new("loginctl").arg(verb).output()
            })
            .await;
            let problem = match result {
                Ok(Ok(out)) if out.status.success() => None,
                Ok(Ok(out)) => Some(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                Ok(Err(e)) => Some(e.to_string()),
                Err(_) => Some("it stopped unexpectedly".into()),
            };
            if let (Some(problem), Some(s)) = (problem, s.upgrade()) {
                s.error
                    .replace(Some(format!("That did not work: {problem}")));
                s.show_message();
            }
        });
    }
}

/// A round picture with a letter and a name under it.
fn person_button(letter: &str, name: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let face = gtk::Label::new(Some(letter));
    face.add_css_class("small-avatar");
    face.set_halign(gtk::Align::Center);
    let label = gtk::Label::new(Some(name));
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(12);
    content.append(&face);
    content.append(&label);
    let button = gtk::Button::new();
    button.add_css_class("person");
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(name));
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greeter.toml");
        assert_eq!(Config::load(&path), Config::default());
        std::fs::write(&path, "accent = \"#ff8800\"\nclock_format = \"%I:%M %p\"\n").unwrap();
        let config = Config::load(&path);
        assert_eq!(config.accent, "#ff8800");
        assert_eq!(config.wallpaper, DEFAULT_WALLPAPER);
        assert!(config.css().contains("#ff8800") && !config.css().contains("#3daee9"));
        // Anything else than a color could break the style sheet.
        std::fs::write(&path, "accent = \"red; } * { color: red\"\n").unwrap();
        assert_eq!(Config::load(&path), Config::default());
        std::fs::write(&path, "wallpaer = \"typo\"\n").unwrap();
        assert_eq!(Config::load(&path), Config::default());
    }

    #[test]
    fn wallpaper_sizes() {
        // A 2560x1440 picture on a 1280x720 screen: halved.
        assert_eq!(cover_size((2560, 1440), (1280, 720)), (1280, 720));
        // On a portrait screen it covers the height.
        assert_eq!(cover_size((2560, 1440), (1080, 1920)), (2560, 1440));
        assert_eq!(cover_size((2560, 1440), (720, 1280)), (2276, 1280));
        // Never enlarged.
        assert_eq!(cover_size((1280, 720), (3840, 2160)), (1280, 720));
    }

    #[test]
    fn the_example_configuration_is_the_default() {
        let example = include_str!("../../../data/greeter/greeter.toml");
        let uncommented: String = example
            .lines()
            .filter_map(|l| l.strip_prefix("# ").filter(|l| l.contains(" = ")))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(
            toml::from_str::<Config>(&uncommented).unwrap(),
            Config::default()
        );
    }
}
