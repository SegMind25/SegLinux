//! Notifications, as Plasma shows them: the freedesktop notification
//! service (`org.freedesktop.Notifications`, which applications reach through
//! libnotify, `notify-send`, Brave, Steam, Discord...), pop-ups above the
//! dock, and a history in the tray with Do Not Disturb.
//!
//! Without such a service a session loses every notification: labwc, unlike
//! Plasma, has none of its own.
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const XML: &str = r#"<node>
  <interface name="org.freedesktop.Notifications">
    <method name="GetCapabilities"><arg type="as" direction="out"/></method>
    <method name="Notify">
      <arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
      <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/>
      <arg type="a{sv}" direction="in"/><arg type="i" direction="in"/>
      <arg type="u" direction="out"/>
    </method>
    <method name="CloseNotification"><arg type="u" direction="in"/></method>
    <method name="GetServerInformation">
      <arg type="s" direction="out"/><arg type="s" direction="out"/>
      <arg type="s" direction="out"/><arg type="s" direction="out"/>
    </method>
    <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
    <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
  </interface>
</node>"#;

/// How long a pop-up stays when the application leaves it to the server.
const DEFAULT_TIMEOUT_MS: u32 = 6000;
/// Pop-ups shown at once; older ones go to the history only.
const MAX_POPUPS: usize = 4;
/// Notifications the history keeps.
const MAX_HISTORY: usize = 100;

/// Why a notification closed (the specification's codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    /// An icon of the theme.
    Name(String),
    /// An image file.
    File(std::path::PathBuf),
    /// Raw pixels from the `image-data` hint: width, height, rowstride,
    /// alpha, pixels.
    Pixels(i32, i32, i32, bool, Vec<u8>),
}

/// What an application asked for with Notify.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub app_name: String,
    pub replaces_id: u32,
    pub icon: Option<Icon>,
    pub summary: String,
    pub body: String,
    /// (key, label) pairs; "default" is a click on the notification.
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    /// Milliseconds the pop-up stays; None: until dismissed.
    pub timeout: Option<u32>,
    pub transient: bool,
    pub resident: bool,
    pub desktop_entry: Option<String>,
}

/// An icon given as `app_icon` or `image-path`: a theme name, a path or a
/// file:// URI.
fn icon_from_string(text: &str) -> Option<Icon> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else if let Some(path) = text.strip_prefix("file://") {
        Some(Icon::File(
            glib::Uri::unescape_string(path, None::<&str>)
                .map(|p| p.to_string())
                .unwrap_or_else(|| path.to_string())
                .into(),
        ))
    } else if text.starts_with('/') {
        Some(Icon::File(text.into()))
    } else {
        Some(Icon::Name(text.to_string()))
    }
}

/// The parameters of Notify, as the specification defines them; None when
/// they have the wrong types.
pub fn parse(params: &glib::Variant) -> Option<Request> {
    #[allow(clippy::type_complexity)]
    let (app_name, replaces_id, app_icon, summary, body, actions, hints, expire): (
        String,
        u32,
        String,
        String,
        String,
        Vec<String>,
        HashMap<String, glib::Variant>,
        i32,
    ) = params.get()?;
    let hint = |names: &[&str]| names.iter().find_map(|n| hints.get(*n)).cloned();
    let urgency = match hint(&["urgency"]).and_then(|v| v.get::<u8>()) {
        Some(0) => Urgency::Low,
        Some(2) => Urgency::Critical,
        _ => Urgency::Normal,
    };
    let flag = |name: &str| hint(&[name]).and_then(|v| v.get::<bool>()).unwrap_or(false);
    // The specification's order: image-data, image-path, app_icon.
    let pixels = hint(&["image-data", "image_data", "icon_data"]).and_then(|v| {
        let (w, h, stride, alpha, bits, channels, data): (i32, i32, i32, bool, i32, i32, Vec<u8>) =
            v.get()?;
        let expected = if alpha { 4 } else { 3 };
        (bits == 8
            && channels == expected
            && w > 0
            && h > 0
            && stride >= w * channels
            && data.len() >= (stride * (h - 1) + w * channels) as usize)
            .then_some(Icon::Pixels(w, h, stride, alpha, data))
    });
    let icon = pixels
        .or_else(|| {
            hint(&["image-path", "image_path"])
                .and_then(|v| v.get::<String>())
                .and_then(|s| icon_from_string(&s))
        })
        .or_else(|| icon_from_string(&app_icon));
    Some(Request {
        app_name,
        replaces_id,
        icon,
        summary,
        body,
        actions: actions
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect(),
        urgency,
        timeout: match (urgency, expire) {
            (Urgency::Critical, _) | (_, 0) => None,
            (_, ms) if ms < 0 => Some(DEFAULT_TIMEOUT_MS),
            (_, ms) => Some(ms as u32),
        },
        transient: flag("transient"),
        resident: flag("resident"),
        desktop_entry: hint(&["desktop-entry"]).and_then(|v| v.get::<String>()),
    })
}

/// A notification of the history.
#[derive(Clone, Debug)]
pub struct Notice {
    pub id: u32,
    pub request: Request,
    /// Seconds since the epoch.
    pub time: i64,
    pub read: bool,
}

impl Notice {
    /// The application's name for display: its desktop entry's, else the
    /// name it gave.
    pub fn app_label(&self) -> String {
        self.desktop_app()
            .map(|a| a.display_name().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.request.app_name.clone())
    }

    fn desktop_app(&self) -> Option<gio::DesktopAppInfo> {
        self.request
            .desktop_entry
            .as_deref()
            .and_then(|id| crate::apps::lookup(id.trim_end_matches(".desktop")))
    }

    /// The picture for the notification: its own, else its application's.
    pub fn image(&self, size: i32) -> gtk::Image {
        let image = match &self.request.icon {
            Some(Icon::Name(name)) => gtk::Image::from_icon_name(name),
            Some(Icon::File(path)) => gtk::Image::from_file(path),
            Some(Icon::Pixels(w, h, stride, alpha, data)) => {
                let format = if *alpha {
                    gdk::MemoryFormat::R8g8b8a8
                } else {
                    gdk::MemoryFormat::R8g8b8
                };
                let bytes = glib::Bytes::from(data.as_slice());
                let texture = gdk::MemoryTexture::new(*w, *h, format, &bytes, *stride as usize);
                gtk::Image::from_paintable(Some(&texture))
            }
            None => match self.desktop_app().and_then(|a| a.icon()) {
                Some(icon) => gtk::Image::from_gicon(&icon),
                None => gtk::Image::from_icon_name("dialog-information-symbolic"),
            },
        };
        image.set_pixel_size(size);
        image
    }
}

/// "now", "5 min ago", "14:05" or "12 Oct": when a notification came.
pub fn age(time: i64, now: i64) -> String {
    let seconds = (now - time).max(0);
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        _ => {
            let format = if seconds < 86400 { "%H:%M" } else { "%e %b" };
            glib::DateTime::from_unix_local(time)
                .ok()
                .and_then(|t| t.format(format).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        }
    }
}

fn now() -> i64 {
    glib::real_time() / 1_000_000
}

struct Inner {
    history: RefCell<Vec<Notice>>,
    /// Ids shown as pop-ups, oldest first.
    popups: RefCell<Vec<u32>>,
    timers: RefCell<HashMap<u32, glib::SourceId>>,
    next_id: Cell<u32>,
    dnd: Cell<bool>,
    connection: RefCell<Option<gio::DBusConnection>>,
    listeners: RefCell<Vec<Box<dyn Fn() -> bool>>>,
    window: RefCell<Option<gtk::Window>>,
    app: gtk::Application,
    layer: bool,
}

/// The notification service and its history; cheap to clone.
#[derive(Clone)]
pub struct Center(Rc<Inner>);

impl Center {
    /// Starts the service on the session bus. `layer`: show pop-ups as a
    /// layer-shell surface (else as a window, in the preview).
    pub fn start(app: &gtk::Application, layer: bool) -> Center {
        let center = Center(Rc::new(Inner {
            history: RefCell::default(),
            popups: RefCell::default(),
            timers: RefCell::default(),
            next_id: Cell::new(1),
            dnd: Cell::new(false),
            connection: RefCell::default(),
            listeners: RefCell::default(),
            window: RefCell::default(),
            app: app.clone(),
            layer,
        }));
        let weak = Rc::downgrade(&center.0);
        gio::bus_own_name(
            gio::BusType::Session,
            INTERFACE,
            gio::BusNameOwnerFlags::NONE,
            move |connection, _| {
                if let Some(inner) = weak.upgrade() {
                    Center(inner).register(&connection);
                }
            },
            |_, _| {},
            |_, name| {
                eprintln!("{name}: another notification service runs; this desktop shows none");
            },
        );
        center
    }

    fn register(&self, connection: &gio::DBusConnection) {
        let info = match gio::DBusNodeInfo::for_xml(XML)
            .ok()
            .and_then(|node| node.lookup_interface(INTERFACE))
        {
            Some(info) => info,
            None => return,
        };
        let weak = Rc::downgrade(&self.0);
        let registered = connection
            .register_object(PATH, &info)
            .method_call(move |_, _, _, _, method, params, invocation| {
                let Some(center) = weak.upgrade().map(Center) else {
                    return;
                };
                match method {
                    "GetCapabilities" => {
                        let caps = vec!["actions", "body", "icon-static", "persistence"];
                        invocation.return_value(Some(&(caps,).to_variant()));
                    }
                    "GetServerInformation" => invocation.return_value(Some(
                        &("SegLinux", "SegLinux", env!("CARGO_PKG_VERSION"), "1.2").to_variant(),
                    )),
                    "Notify" => match parse(&params) {
                        Some(request) => {
                            let id = center.add(request);
                            invocation.return_value(Some(&(id,).to_variant()));
                        }
                        None => invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.InvalidArgs",
                            "Notify: wrong arguments",
                        ),
                    },
                    "CloseNotification" => {
                        if let Some((id,)) = params.get::<(u32,)>() {
                            center.close(id, Reason::Closed);
                        }
                        invocation.return_value(None);
                    }
                    _ => invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "Unknown method",
                    ),
                }
            })
            .build();
        match registered {
            Ok(_) => {
                self.0.connection.replace(Some(connection.clone()));
            }
            Err(error) => eprintln!("Cannot offer notifications: {error}"),
        }
    }

    /// Calls `listener` after every change, until it returns false.
    pub fn subscribe(&self, listener: impl Fn() -> bool + 'static) {
        self.0.listeners.borrow_mut().push(Box::new(listener));
    }

    fn changed(&self) {
        let listeners: Vec<_> = self.0.listeners.borrow_mut().drain(..).collect();
        let kept: Vec<_> = listeners.into_iter().filter(|l| l()).collect();
        let mut current = self.0.listeners.borrow_mut();
        let added: Vec<_> = current.drain(..).collect();
        current.extend(kept);
        current.extend(added);
    }

    /// The history, newest first.
    pub fn history(&self) -> Vec<Notice> {
        self.0.history.borrow().iter().rev().cloned().collect()
    }

    pub fn unread(&self) -> usize {
        self.0.history.borrow().iter().filter(|n| !n.read).count()
    }

    pub fn mark_read(&self) {
        let mut changed = false;
        for notice in self.0.history.borrow_mut().iter_mut() {
            changed |= !notice.read;
            notice.read = true;
        }
        if changed {
            self.changed();
        }
    }

    pub fn dnd(&self) -> bool {
        self.0.dnd.get()
    }

    /// Do Not Disturb: notifications go to the history without pop-ups
    /// (critical ones still show).
    pub fn set_dnd(&self, on: bool) {
        self.0.dnd.set(on);
        self.changed();
    }

    /// Clears the history (pop-ups included).
    pub fn clear(&self) {
        let ids: Vec<u32> = self.0.history.borrow().iter().map(|n| n.id).collect();
        for id in ids {
            self.close(id, Reason::Dismissed);
        }
    }

    /// Adds or replaces a notification; returns its id.
    pub fn add(&self, request: Request) -> u32 {
        let replaced = request.replaces_id != 0
            && self
                .0
                .history
                .borrow()
                .iter()
                .any(|n| n.id == request.replaces_id);
        let id = if replaced {
            request.replaces_id
        } else {
            let id = self.0.next_id.get();
            self.0.next_id.set(id.wrapping_add(1).max(1));
            id
        };
        let popup = !self.0.dnd.get() || request.urgency == Urgency::Critical;
        let timeout = request.timeout;
        let notice = Notice {
            id,
            request,
            time: now(),
            read: false,
        };
        {
            let mut history = self.0.history.borrow_mut();
            match history.iter_mut().find(|n| n.id == id) {
                Some(existing) => *existing = notice,
                None => {
                    history.push(notice);
                    let excess = history.len().saturating_sub(MAX_HISTORY);
                    history.drain(..excess);
                }
            }
        }
        if popup {
            {
                let mut popups = self.0.popups.borrow_mut();
                if !popups.contains(&id) {
                    popups.push(id);
                }
                while popups.len() > MAX_POPUPS {
                    let old = popups.remove(0);
                    if let Some(timer) = self.0.timers.borrow_mut().remove(&old) {
                        timer.remove();
                    }
                }
            }
            self.arm(id, timeout);
        }
        self.show_popups();
        self.changed();
        id
    }

    /// Hides the pop-up `id` after `timeout` ms.
    fn arm(&self, id: u32, timeout: Option<u32>) {
        if let Some(timer) = self.0.timers.borrow_mut().remove(&id) {
            timer.remove();
        }
        let Some(ms) = timeout else {
            return;
        };
        let weak = Rc::downgrade(&self.0);
        let timer = glib::timeout_add_local_once(Duration::from_millis(ms.into()), move || {
            if let Some(center) = weak.upgrade().map(Center) {
                center.0.timers.borrow_mut().remove(&id);
                center.expire(id);
            }
        });
        self.0.timers.borrow_mut().insert(id, timer);
    }

    /// A pop-up times out: it leaves the screen and stays in the history,
    /// except transient notifications, which go.
    fn expire(&self, id: u32) {
        self.0.popups.borrow_mut().retain(|p| *p != id);
        let transient = self
            .0
            .history
            .borrow()
            .iter()
            .any(|n| n.id == id && n.request.transient);
        if transient {
            self.0.history.borrow_mut().retain(|n| n.id != id);
        }
        self.emit(
            "NotificationClosed",
            &(id, Reason::Expired as u32).to_variant(),
        );
        self.show_popups();
        self.changed();
    }

    /// Removes a notification from the screen and the history.
    pub fn close(&self, id: u32, reason: Reason) {
        let known = self.0.history.borrow().iter().any(|n| n.id == id);
        self.0.history.borrow_mut().retain(|n| n.id != id);
        self.0.popups.borrow_mut().retain(|p| *p != id);
        if let Some(timer) = self.0.timers.borrow_mut().remove(&id) {
            timer.remove();
        }
        if known {
            self.emit("NotificationClosed", &(id, reason as u32).to_variant());
        }
        self.show_popups();
        self.changed();
    }

    /// Runs one of the notification's actions in its application.
    pub fn invoke(&self, id: u32, action: &str) {
        self.emit("ActionInvoked", &(id, action).to_variant());
        let resident = self
            .0
            .history
            .borrow()
            .iter()
            .any(|n| n.id == id && n.request.resident);
        if !resident {
            self.close(id, Reason::Dismissed);
        }
    }

    /// A click on a notification: its default action, if any, else it
    /// opens its application when it has one.
    pub fn activate(&self, notice: &Notice) {
        if notice
            .request
            .actions
            .iter()
            .any(|(key, _)| key == "default")
        {
            self.invoke(notice.id, "default");
            return;
        }
        if let Some(app) = notice.desktop_app() {
            crate::common::launch_app(&app);
        }
        self.close(notice.id, Reason::Dismissed);
    }

    fn emit(&self, signal: &str, args: &glib::Variant) {
        if let Some(connection) = self.0.connection.borrow().as_ref()
            && let Err(error) = connection.emit_signal(None, PATH, INTERFACE, signal, Some(args))
        {
            eprintln!("Cannot send {signal}: {error}");
        }
    }

    /// Shows the current pop-ups (newest at the bottom, next to the dock),
    /// or hides their window when there are none.
    fn show_popups(&self) {
        let ids = self.0.popups.borrow().clone();
        let notices: Vec<Notice> = {
            let history = self.0.history.borrow();
            ids.iter()
                .filter_map(|id| history.iter().find(|n| n.id == *id).cloned())
                .collect()
        };
        // A new window each time: a layer surface keeps the size of its
        // largest content otherwise.
        if let Some(old) = self.0.window.take() {
            old.destroy();
        }
        if notices.is_empty() {
            return;
        }
        let window = gtk::Window::builder()
            .title("Notifications")
            .decorated(false)
            .resizable(false)
            .build();
        window.add_css_class("notifications");
        if self.0.layer {
            window.init_layer_shell();
            window.set_namespace(Some("seglinux-notifications"));
            window.set_layer(Layer::Overlay);
            window.set_keyboard_mode(KeyboardMode::None);
            // The compositor keeps them clear of the dock (its exclusive
            // zone), at the bottom right as in Plasma.
            window.set_anchor(Edge::Bottom, true);
            window.set_anchor(Edge::Right, true);
            window.set_margin(Edge::Bottom, 10);
            window.set_margin(Edge::Right, 10);
        } else {
            window.set_application(Some(&self.0.app));
        }
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        for notice in &notices {
            column.append(&self.card(notice, true));
        }
        window.set_child(Some(&column));
        window.present();
        self.0.window.replace(Some(window));
    }

    /// A notification as a card: picture, application and time, title,
    /// text and action buttons. Pop-ups also close by themselves.
    pub fn card(&self, notice: &Notice, popup: bool) -> gtk::Box {
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        card.add_css_class("notification");
        if notice.request.urgency == Urgency::Critical {
            card.add_css_class("critical");
        }
        card.set_size_request(360, -1);
        let image = notice.image(36);
        image.set_valign(gtk::Align::Start);
        card.append(&image);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let app = gtk::Label::new(Some(&notice.app_label()));
        app.add_css_class("notification-app");
        app.set_xalign(0.0);
        app.set_hexpand(true);
        app.set_ellipsize(gtk::pango::EllipsizeMode::End);
        head.append(&app);
        let when = gtk::Label::new(Some(&age(notice.time, now())));
        when.add_css_class("notification-app");
        head.append(&when);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("notification-close");
        close.set_tooltip_text(Some("Dismiss"));
        let id = notice.id;
        let center = self.clone();
        close.connect_clicked(move |_| center.close(id, Reason::Dismissed));
        head.append(&close);
        text.append(&head);

        if !notice.request.summary.is_empty() {
            let summary = gtk::Label::new(Some(&notice.request.summary));
            summary.add_css_class("notification-summary");
            summary.set_xalign(0.0);
            summary.set_wrap(true);
            summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            summary.set_max_width_chars(40);
            summary.set_lines(2);
            summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.append(&summary);
        }
        if !notice.request.body.is_empty() {
            let body = gtk::Label::new(Some(&notice.request.body));
            body.add_css_class("notification-body");
            body.set_xalign(0.0);
            body.set_wrap(true);
            body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            body.set_max_width_chars(44);
            body.set_lines(if popup { 4 } else { 3 });
            body.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.append(&body);
        }
        let buttons: Vec<_> = notice
            .request
            .actions
            .iter()
            .filter(|(key, label)| key != "default" && !label.is_empty())
            .collect();
        if !buttons.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.add_css_class("notification-actions");
            for (key, label) in buttons {
                let button = gtk::Button::with_label(label);
                let (center, key) = (self.clone(), key.clone());
                button.connect_clicked(move |_| center.invoke(id, &key));
                row.append(&button);
            }
            text.append(&row);
        }
        card.append(&text);

        // A click on the card (not on its buttons) activates it.
        let click = gtk::GestureClick::new();
        let (center, clicked) = (self.clone(), notice.clone());
        click.connect_released(move |gesture, _, x, y| {
            let Some(widget) = gesture.widget() else {
                return;
            };
            let on_button = widget.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|w| {
                w.is::<gtk::Button>() || w.ancestor(gtk::Button::static_type()).is_some()
            });
            if !on_button {
                center.activate(&clicked);
            }
        });
        card.add_controller(click);

        if popup {
            // Kept on screen while the pointer is over it.
            let hover = gtk::EventControllerMotion::new();
            let center = self.clone();
            hover.connect_enter(move |_, _, _| {
                if let Some(timer) = center.0.timers.borrow_mut().remove(&id) {
                    timer.remove();
                }
            });
            let center = self.clone();
            let timeout = notice.request.timeout;
            hover.connect_leave(move |_| {
                if timeout.is_some() && center.0.popups.borrow().contains(&id) {
                    center.arm(id, Some(2500));
                }
            });
            card.add_controller(hover);
        }
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notify(
        app_icon: &str,
        actions: &[&str],
        hints: HashMap<String, glib::Variant>,
        expire: i32,
    ) -> glib::Variant {
        (
            "Brave",
            0u32,
            app_icon,
            "Download complete",
            "photo.jpg",
            actions.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            hints,
            expire,
        )
            .to_variant()
    }

    #[test]
    fn parses_notify() {
        let request = parse(&notify(
            "brave-browser",
            &["default", "Open", "show", "Show in folder"],
            HashMap::new(),
            -1,
        ))
        .unwrap();
        assert_eq!(request.app_name, "Brave");
        assert_eq!(request.icon, Some(Icon::Name("brave-browser".into())));
        assert_eq!(request.summary, "Download complete");
        assert_eq!(
            request.actions,
            [
                ("default".to_string(), "Open".to_string()),
                ("show".to_string(), "Show in folder".to_string())
            ]
        );
        assert_eq!(request.urgency, Urgency::Normal);
        assert_eq!(request.timeout, Some(DEFAULT_TIMEOUT_MS));
        assert!(!request.transient && !request.resident);
        // Wrong types are refused, not guessed.
        assert_eq!(parse(&("only a string",).to_variant()), None);
    }

    #[test]
    fn timeouts_and_urgency() {
        let r = parse(&notify("", &[], HashMap::new(), 0)).unwrap();
        assert_eq!(r.timeout, None, "0 means until dismissed");
        let r = parse(&notify("", &[], HashMap::new(), 1500)).unwrap();
        assert_eq!(r.timeout, Some(1500));
        let mut hints = HashMap::new();
        hints.insert("urgency".to_string(), 2u8.to_variant());
        hints.insert("transient".to_string(), true.to_variant());
        hints.insert("desktop-entry".to_string(), "steam".to_variant());
        let r = parse(&notify("", &[], hints, 3000)).unwrap();
        assert_eq!(r.urgency, Urgency::Critical);
        assert_eq!(r.timeout, None, "critical notifications stay");
        assert!(r.transient);
        assert_eq!(r.desktop_entry.as_deref(), Some("steam"));
        assert_eq!(r.icon, None);
    }

    #[test]
    fn icons_in_the_specifications_order() {
        assert_eq!(
            icon_from_string("file:///tmp/a%20b.png"),
            Some(Icon::File("/tmp/a b.png".into()))
        );
        assert_eq!(
            icon_from_string("/usr/share/x.svg"),
            Some(Icon::File("/usr/share/x.svg".into()))
        );
        assert_eq!(icon_from_string("  "), None);

        let mut hints = HashMap::new();
        hints.insert("image-path".to_string(), "/tmp/cover.png".to_variant());
        let r = parse(&notify("steam", &[], hints.clone(), -1)).unwrap();
        assert_eq!(r.icon, Some(Icon::File("/tmp/cover.png".into())));
        // Pixels win over everything; malformed pixels are ignored.
        let pixels = (2i32, 1i32, 8i32, true, 8i32, 4i32, vec![255u8; 8]).to_variant();
        hints.insert("image-data".to_string(), pixels);
        let r = parse(&notify("steam", &[], hints.clone(), -1)).unwrap();
        assert!(matches!(r.icon, Some(Icon::Pixels(2, 1, 8, true, _))));
        let short = (2i32, 2i32, 8i32, true, 8i32, 4i32, vec![255u8; 8]).to_variant();
        hints.insert("image-data".to_string(), short);
        let r = parse(&notify("steam", &[], hints, -1)).unwrap();
        assert_eq!(r.icon, Some(Icon::File("/tmp/cover.png".into())));
    }

    #[test]
    fn ages() {
        assert_eq!(age(1000, 1030), "now");
        assert_eq!(age(1000, 1000 + 5 * 60 + 10), "5 min ago");
        assert!(!age(1000, 1000 + 7200).is_empty());
        assert_eq!(age(2000, 1000), "now", "a clock set back is not the future");
    }
}
