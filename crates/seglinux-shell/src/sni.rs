//! Other applications' icons in the tray, as in Plasma's system tray: the
//! StatusNotifierItem protocol (KDE's, also used by libappindicator and
//! Ayatana: Steam, Discord, Telegram, nm-applet, ...) with their menus
//! (com.canonical.dbusmenu).
//!
//! The shell is the watcher (`org.kde.StatusNotifierWatcher`, with which
//! applications register their items) and the host that shows them. Without
//! one, an application that hides in the tray (Steam, when its window is
//! closed) could not be brought back.
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const XML: &str = r#"<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
    <method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
    <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
    <signal name="StatusNotifierHostRegistered"/>
    <signal name="StatusNotifierHostUnregistered"/>
  </interface>
</node>"#;

/// Where an item lives: `service` as the application registered it (a bus
/// name, or an object path on the sender's connection).
pub fn address(service: &str, sender: &str) -> (String, String) {
    if service.starts_with('/') {
        (sender.to_string(), service.to_string())
    } else {
        (service.to_string(), "/StatusNotifierItem".to_string())
    }
}

/// The best of an item's pixmaps for `size` pixels: the smallest at least
/// that large, else the largest. Pixmaps are (width, height, ARGB32 in
/// network byte order).
pub fn pick_pixmap(pixmaps: &[(i32, i32, Vec<u8>)], size: i32) -> Option<&(i32, i32, Vec<u8>)> {
    let valid = pixmaps
        .iter()
        .filter(|(w, h, data)| *w > 0 && *h > 0 && data.len() >= (*w * *h * 4) as usize);
    valid
        .clone()
        .filter(|(w, _, _)| *w >= size)
        .min_by_key(|(w, _, _)| *w)
        .or_else(|| valid.max_by_key(|(w, _, _)| *w))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Props {
    pub id: String,
    pub title: String,
    /// Passive (hidden, as in Plasma), Active or NeedsAttention.
    pub status: String,
    pub icon_name: String,
    pub attention_icon_name: String,
    pub icon_theme_path: String,
    pub pixmaps: Vec<(i32, i32, Vec<u8>)>,
    pub attention_pixmaps: Vec<(i32, i32, Vec<u8>)>,
    pub tooltip: String,
    pub menu: Option<String>,
    pub item_is_menu: bool,
}

impl Props {
    /// From org.freedesktop.DBus.Properties.GetAll.
    pub fn parse(all: &HashMap<String, glib::Variant>) -> Props {
        let text = |key: &str| {
            all.get(key)
                .and_then(|v| v.str().map(str::to_string))
                .unwrap_or_default()
        };
        let pixmaps = |key: &str| {
            all.get(key)
                .and_then(|v| v.get::<Vec<(i32, i32, Vec<u8>)>>())
                .unwrap_or_default()
        };
        let tooltip = all
            .get("ToolTip")
            .and_then(|v| v.get::<(String, Vec<(i32, i32, Vec<u8>)>, String, String)>())
            .map(
                |(_, _, title, body)| match (title.is_empty(), body.is_empty()) {
                    (false, false) => format!("{title}\n{body}"),
                    (false, true) => title,
                    (true, _) => body,
                },
            )
            .unwrap_or_default();
        let menu = all
            .get("Menu")
            .and_then(|v| v.str().map(str::to_string))
            .filter(|p| p.starts_with('/') && p != "/");
        Props {
            id: text("Id"),
            title: text("Title"),
            status: text("Status"),
            icon_name: text("IconName"),
            attention_icon_name: text("AttentionIconName"),
            icon_theme_path: text("IconThemePath"),
            pixmaps: pixmaps("IconPixmap"),
            attention_pixmaps: pixmaps("AttentionIconPixmap"),
            tooltip,
            menu,
            item_is_menu: all
                .get("ItemIsMenu")
                .and_then(|v| v.get::<bool>())
                .unwrap_or(false),
        }
    }

    pub fn visible(&self) -> bool {
        self.status != "Passive"
    }

    /// What the tray shows for it on hover.
    pub fn label(&self) -> String {
        [&self.tooltip, &self.title, &self.id]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default()
    }
}

/// One entry of an application's menu.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    /// Some(checked) for check boxes and radio items.
    pub toggle: Option<bool>,
    pub children: Vec<MenuNode>,
}

/// A dbusmenu label without its mnemonic underscores ("_Quit" is Quit;
/// "__" is an underscore).
pub fn plain_label(label: &str) -> String {
    let mut out = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl MenuNode {
    /// A node of GetLayout's (ia{sv}av) tree.
    pub fn parse(node: &glib::Variant) -> Option<MenuNode> {
        let (id, props, children): (i32, HashMap<String, glib::Variant>, Vec<glib::Variant>) =
            node.get()?;
        let text = |key: &str| props.get(key).and_then(|v| v.str().map(str::to_string));
        let flag = |key: &str, default: bool| {
            props
                .get(key)
                .and_then(|v| v.get::<bool>())
                .unwrap_or(default)
        };
        let toggle = match text("toggle-type").as_deref() {
            Some("checkmark" | "radio") => Some(
                props
                    .get("toggle-state")
                    .and_then(|v| v.get::<i32>())
                    .is_some_and(|s| s == 1),
            ),
            _ => None,
        };
        Some(MenuNode {
            id,
            label: plain_label(&text("label").unwrap_or_default()),
            enabled: flag("enabled", true),
            visible: flag("visible", true),
            separator: text("type").as_deref() == Some("separator"),
            toggle,
            // Children are variants holding nodes.
            children: children
                .iter()
                .filter_map(|c| MenuNode::parse(&c.as_variant().unwrap_or_else(|| c.clone())))
                .collect(),
        })
    }
}

pub struct Item {
    pub key: String,
    pub bus: String,
    pub path: String,
    pub props: Props,
    /// Stops watching for the application to leave the bus.
    unwatch: Option<Box<dyn FnOnce()>>,
    _signals: Option<gio::SignalSubscription>,
}

struct Inner {
    connection: RefCell<Option<gio::DBusConnection>>,
    items: RefCell<Vec<Item>>,
    listeners: RefCell<Vec<Box<dyn Fn() -> bool>>>,
    pending: RefCell<HashMap<String, glib::SourceId>>,
}

/// The tray's applications: the watcher with which they register, and
/// their current properties. Cheap to clone.
#[derive(Clone)]
pub struct Host(Rc<Inner>);

impl Host {
    pub fn start() -> Host {
        let host = Host(Rc::new(Inner {
            connection: RefCell::default(),
            items: RefCell::default(),
            listeners: RefCell::default(),
            pending: RefCell::default(),
        }));
        let weak = Rc::downgrade(&host.0);
        gio::bus_own_name(
            gio::BusType::Session,
            WATCHER,
            gio::BusNameOwnerFlags::NONE,
            move |connection, _| {
                if let Some(inner) = weak.upgrade() {
                    Host(inner).register(&connection);
                }
            },
            |connection, _| {
                // Items wait for a host before they show themselves.
                let _ = connection.emit_signal(
                    None,
                    WATCHER_PATH,
                    WATCHER,
                    "StatusNotifierHostRegistered",
                    None,
                );
            },
            |_, name| eprintln!("{name}: another system tray runs; this one shows nothing"),
        );
        host
    }

    fn register(&self, connection: &gio::DBusConnection) {
        let Some(info) = gio::DBusNodeInfo::for_xml(XML)
            .ok()
            .and_then(|node| node.lookup_interface(WATCHER))
        else {
            return;
        };
        let calls = Rc::downgrade(&self.0);
        let props = Rc::downgrade(&self.0);
        let registered = connection
            .register_object(WATCHER_PATH, &info)
            .method_call(move |_, sender, _, _, method, params, invocation| {
                let host = calls.upgrade().map(Host);
                match (method, host, params.get::<(String,)>()) {
                    ("RegisterStatusNotifierItem", Some(host), Some((service,))) => {
                        host.add(&service, sender.unwrap_or_default());
                        invocation.return_value(None);
                    }
                    ("RegisterStatusNotifierHost", _, _) => invocation.return_value(None),
                    _ => invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "Unknown method or wrong arguments",
                    ),
                }
            })
            .property(move |_, _, _, _, name| match name {
                "RegisteredStatusNotifierItems" => props
                    .upgrade()
                    .map(|inner| {
                        inner
                            .items
                            .borrow()
                            .iter()
                            .map(|i| i.key.clone())
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default()
                    .to_variant(),
                "IsStatusNotifierHostRegistered" => true.to_variant(),
                "ProtocolVersion" => 0i32.to_variant(),
                _ => ().to_variant(),
            })
            .build();
        match registered {
            Ok(_) => {
                self.0.connection.replace(Some(connection.clone()));
            }
            Err(error) => eprintln!("Cannot offer a system tray: {error}"),
        }
    }

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

    /// The visible items: (key, properties), in the order they registered.
    pub fn items(&self) -> Vec<(String, Props)> {
        self.0
            .items
            .borrow()
            .iter()
            .filter(|i| i.props.visible())
            .map(|i| (i.key.clone(), i.props.clone()))
            .collect()
    }

    fn connection(&self) -> Option<gio::DBusConnection> {
        self.0.connection.borrow().clone()
    }

    fn add(&self, service: &str, sender: &str) {
        let Some(connection) = self.connection() else {
            return;
        };
        let (bus, path) = address(service, sender);
        let key = format!("{bus}{path}");
        if self.0.items.borrow().iter().any(|i| i.key == key) {
            return;
        }
        let gone = Rc::downgrade(&self.0);
        let gone_key = key.clone();
        let vanish = gio::bus_watch_name_on_connection(
            &connection,
            &bus,
            gio::BusNameWatcherFlags::NONE,
            |_, _, _| {},
            move |_, _| {
                if let Some(host) = gone.upgrade().map(Host) {
                    host.remove(&gone_key);
                }
            },
        );
        let refresh = Rc::downgrade(&self.0);
        let refresh_key = key.clone();
        let signals = connection.subscribe_to_signal(
            Some(&bus),
            Some(ITEM),
            None,
            Some(&path),
            None,
            gio::DBusSignalFlags::NONE,
            move |_| {
                if let Some(host) = refresh.upgrade().map(Host) {
                    host.refresh_soon(&refresh_key);
                }
            },
        );
        self.0.items.borrow_mut().push(Item {
            key: key.clone(),
            bus,
            path,
            props: Props::default(),
            unwatch: Some(Box::new(move || gio::bus_unwatch_name(vanish))),
            _signals: Some(signals),
        });
        let _ = connection.emit_signal(
            None,
            WATCHER_PATH,
            WATCHER,
            "StatusNotifierItemRegistered",
            Some(&(key.as_str(),).to_variant()),
        );
        self.refresh(&key);
    }

    fn remove(&self, key: &str) {
        let removed = {
            let mut items = self.0.items.borrow_mut();
            let before = items.len();
            let gone: Vec<Item> = items.extract_if(.., |i| i.key == key).collect();
            for mut item in gone {
                if let Some(unwatch) = item.unwatch.take() {
                    unwatch();
                }
            }
            items.len() != before
        };
        if removed {
            if let Some(connection) = self.connection() {
                let _ = connection.emit_signal(
                    None,
                    WATCHER_PATH,
                    WATCHER,
                    "StatusNotifierItemUnregistered",
                    Some(&(key,).to_variant()),
                );
            }
            self.changed();
        }
    }

    /// Items send several signals at once (new icon, title, tooltip): one
    /// refresh for them.
    fn refresh_soon(&self, key: &str) {
        if self.0.pending.borrow().contains_key(key) {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        let owned = key.to_string();
        let id = glib::timeout_add_local_once(Duration::from_millis(100), move || {
            if let Some(host) = weak.upgrade().map(Host) {
                host.0.pending.borrow_mut().remove(&owned);
                host.refresh(&owned);
            }
        });
        self.0.pending.borrow_mut().insert(key.to_string(), id);
    }

    fn refresh(&self, key: &str) {
        let Some(connection) = self.connection() else {
            return;
        };
        let Some((bus, path)) = self
            .0
            .items
            .borrow()
            .iter()
            .find(|i| i.key == key)
            .map(|i| (i.bus.clone(), i.path.clone()))
        else {
            return;
        };
        let weak = Rc::downgrade(&self.0);
        let key = key.to_string();
        connection.call(
            Some(&bus),
            &path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(ITEM,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
            move |result| {
                let Some(host) = weak.upgrade().map(Host) else {
                    return;
                };
                let Some((all,)) = result
                    .ok()
                    .and_then(|v| v.get::<(HashMap<String, glib::Variant>,)>())
                else {
                    return;
                };
                let props = Props::parse(&all);
                if !props.icon_theme_path.is_empty()
                    && let Some(display) = gdk::Display::default()
                {
                    let theme = gtk::IconTheme::for_display(&display);
                    if !theme
                        .search_path()
                        .iter()
                        .any(|p| p.as_os_str() == props.icon_theme_path.as_str())
                    {
                        theme.add_search_path(&props.icon_theme_path);
                    }
                }
                if let Some(item) = host.0.items.borrow_mut().iter_mut().find(|i| i.key == key) {
                    item.props = props;
                }
                host.changed();
            },
        );
    }

    fn call(&self, key: &str, interface: &str, method: &str, args: glib::Variant) {
        let Some(connection) = self.connection() else {
            return;
        };
        let Some((bus, path)) = self.target(key) else {
            return;
        };
        connection.call(
            Some(&bus),
            &path,
            interface,
            method,
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
            |_| {},
        );
    }

    fn target(&self, key: &str) -> Option<(String, String)> {
        self.0
            .items
            .borrow()
            .iter()
            .find(|i| i.key == key)
            .map(|i| (i.bus.clone(), i.path.clone()))
    }

    /// A left click: the application's window, or its menu when it has
    /// nothing else to show.
    pub fn activate(&self, key: &str, anchor: &gtk::Widget) {
        let Some(props) = self.props(key) else {
            return;
        };
        if props.item_is_menu && props.menu.is_some() {
            self.open_menu(key, anchor);
            return;
        }
        let Some(connection) = self.connection() else {
            return;
        };
        let Some((bus, path)) = self.target(key) else {
            return;
        };
        let host = self.clone();
        let (key, anchor) = (key.to_string(), anchor.downgrade());
        connection.call(
            Some(&bus),
            &path,
            ITEM,
            "Activate",
            Some(&(0i32, 0i32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
            move |result| {
                // Items without a window of their own (Ayatana) only have
                // their menu.
                if result.is_err()
                    && let Some(anchor) = anchor.upgrade()
                {
                    host.open_menu(&key, &anchor);
                }
            },
        );
    }

    pub fn secondary_activate(&self, key: &str) {
        self.call(key, ITEM, "SecondaryActivate", (0i32, 0i32).to_variant());
    }

    pub fn scroll(&self, key: &str, delta: i32) {
        self.call(key, ITEM, "Scroll", (delta, "vertical").to_variant());
    }

    fn props(&self, key: &str) -> Option<Props> {
        self.0
            .items
            .borrow()
            .iter()
            .find(|i| i.key == key)
            .map(|i| i.props.clone())
    }

    /// A right click: the application's menu, built from its dbusmenu
    /// layout, or the application's own context menu.
    pub fn open_menu(&self, key: &str, anchor: &gtk::Widget) {
        let Some(props) = self.props(key) else {
            return;
        };
        let (Some(connection), Some((bus, _))) = (self.connection(), self.target(key)) else {
            return;
        };
        let Some(menu_path) = props.menu else {
            self.call(key, ITEM, "ContextMenu", (0i32, 0i32).to_variant());
            return;
        };
        // AboutToShow lets the application fill the menu in; its answer
        // does not matter.
        connection.call(
            Some(&bus),
            &menu_path,
            MENU,
            "AboutToShow",
            Some(&(0i32,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
            |_| {},
        );
        let anchor = anchor.downgrade();
        let events = (connection.clone(), bus.clone(), menu_path.clone());
        connection.call(
            Some(&bus),
            &menu_path,
            MENU,
            "GetLayout",
            Some(&(0i32, -1i32, Vec::<String>::new()).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
            move |result| {
                let Some(anchor) = anchor.upgrade() else {
                    return;
                };
                // (u(ia{sv}av)): the revision, then the root node.
                let root = result
                    .ok()
                    .filter(|reply| reply.n_children() == 2)
                    .and_then(|reply| MenuNode::parse(&reply.child_value(1)));
                match root {
                    Some(root) if !root.children.is_empty() => {
                        show_menu(&anchor, &root, Rc::new(events));
                    }
                    _ => eprintln!("The tray item's menu could not be read"),
                }
            },
        );
    }
}

type MenuTarget = Rc<(gio::DBusConnection, String, String)>;

/// Sends "clicked" for a menu entry.
fn clicked(target: &MenuTarget, id: i32) {
    let (connection, bus, path) = &**target;
    let args = (id, "clicked", 0i32.to_variant(), 0u32).to_variant();
    connection.call(
        Some(bus),
        path,
        MENU,
        "Event",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
        |_| {},
    );
}

/// The menu in a popover above the tray icon; submenus open in place,
/// with a way back.
fn show_menu(anchor: &gtk::Widget, root: &MenuNode, target: MenuTarget) {
    let popover = gtk::Popover::new();
    popover.add_css_class("menu");
    popover.set_parent(anchor);
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    let stack = gtk::Stack::new();
    stack.set_vhomogeneous(false);
    stack.set_hhomogeneous(false);
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    fill_page(&stack, &popover, root, None, &target);
    popover.set_child(Some(&stack));
    // The icon's tooltip would cover the menu.
    let had_tooltip = anchor.has_tooltip();
    anchor.set_has_tooltip(false);
    let restore = anchor.downgrade();
    popover.connect_closed(move |_| {
        if let Some(anchor) = restore.upgrade() {
            anchor.set_has_tooltip(had_tooltip);
        }
    });
    popover.popup();
}

fn fill_page(
    stack: &gtk::Stack,
    popover: &gtk::Popover,
    node: &MenuNode,
    back: Option<&str>,
    target: &MenuTarget,
) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let name = format!("menu-{}", node.id);
    if let Some(back_to) = back {
        let button = crate::common::menu_item(&format!("‹  {}", node.label));
        let (stack, back_to) = (stack.clone(), back_to.to_string());
        button.connect_clicked(move |_| stack.set_visible_child_name(&back_to));
        page.append(&button);
        page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    }
    for child in node.children.iter().filter(|c| c.visible) {
        if child.separator {
            page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            continue;
        }
        let mark = match child.toggle {
            Some(true) => "✓  ",
            Some(false) => "    ",
            None => "",
        };
        let has_submenu = !child.children.is_empty();
        let label = format!(
            "{mark}{}{}",
            child.label,
            if has_submenu { "  ›" } else { "" }
        );
        let button = crate::common::menu_item(&label);
        button.set_sensitive(child.enabled);
        if has_submenu {
            fill_page(stack, popover, child, Some(&name), target);
            let (stack, sub) = (stack.clone(), format!("menu-{}", child.id));
            button.connect_clicked(move |_| stack.set_visible_child_name(&sub));
        } else {
            let (popover, target, id) = (popover.clone(), target.clone(), child.id);
            button.connect_clicked(move |_| {
                clicked(&target, id);
                popover.popdown();
            });
        }
        page.append(&button);
    }
    stack.add_named(&page, Some(&name));
    if back.is_none() {
        stack.set_visible_child_name(&name);
    }
}

/// The picture of an item: its icon from the theme (or a file), else its
/// pixmap.
pub fn image(props: &Props, size: i32) -> gtk::Image {
    let attention = props.status == "NeedsAttention";
    let name = if attention && !props.attention_icon_name.is_empty() {
        &props.attention_icon_name
    } else {
        &props.icon_name
    };
    let pixmaps = if attention && !props.attention_pixmaps.is_empty() {
        &props.attention_pixmaps
    } else {
        &props.pixmaps
    };
    let theme = gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    let image = if name.starts_with('/') {
        gtk::Image::from_file(name)
    } else if !name.is_empty() && theme.is_some_and(|t| t.has_icon(name)) {
        gtk::Image::from_icon_name(name)
    } else if let Some((w, h, data)) = pick_pixmap(pixmaps, size) {
        // ARGB32 in network byte order: GDK's A8r8g8b8.
        let bytes = glib::Bytes::from(&data[..(*w * *h * 4) as usize]);
        let texture = gdk::MemoryTexture::new(
            *w,
            *h,
            gdk::MemoryFormat::A8r8g8b8,
            &bytes,
            (*w * 4) as usize,
        );
        gtk::Image::from_paintable(Some(&texture))
    } else if !name.is_empty() {
        gtk::Image::from_icon_name(name)
    } else {
        gtk::Image::from_icon_name("application-x-executable-symbolic")
    };
    image.set_pixel_size(size);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_addresses() {
        assert_eq!(
            address("org.kde.StatusNotifierItem-4242-1", ":1.7"),
            (
                "org.kde.StatusNotifierItem-4242-1".to_string(),
                "/StatusNotifierItem".to_string()
            )
        );
        // libappindicator / Ayatana (Steam, Discord) register an object path.
        assert_eq!(
            address("/org/ayatana/NotificationItem/steam", ":1.42"),
            (
                ":1.42".to_string(),
                "/org/ayatana/NotificationItem/steam".to_string()
            )
        );
    }

    #[test]
    fn pixmap_choice() {
        let px = |w: i32| (w, w, vec![0u8; (w * w * 4) as usize]);
        let all = vec![px(16), px(22), px(48)];
        assert_eq!(pick_pixmap(&all, 20).unwrap().0, 22);
        assert_eq!(pick_pixmap(&all, 64).unwrap().0, 48);
        let broken = vec![(32, 32, vec![0u8; 10])];
        assert!(pick_pixmap(&broken, 22).is_none());
    }

    #[test]
    fn properties() {
        let mut all: HashMap<String, glib::Variant> = HashMap::new();
        all.insert("Id".into(), "steam".to_variant());
        all.insert("Title".into(), "Steam".to_variant());
        all.insert("Status".into(), "Active".to_variant());
        all.insert("IconName".into(), "steam_tray_mono".to_variant());
        all.insert(
            "Menu".into(),
            glib::variant::ObjectPath::try_from("/MenuBar".to_string())
                .unwrap()
                .to_variant(),
        );
        all.insert(
            "ToolTip".into(),
            (
                "",
                Vec::<(i32, i32, Vec<u8>)>::new(),
                "Steam",
                "2 friends online",
            )
                .to_variant(),
        );
        let props = Props::parse(&all);
        assert_eq!(props.icon_name, "steam_tray_mono");
        assert_eq!(props.menu.as_deref(), Some("/MenuBar"));
        assert_eq!(props.label(), "Steam\n2 friends online");
        assert!(props.visible());
        all.insert("Status".into(), "Passive".to_variant());
        assert!(!Props::parse(&all).visible());
        // "/" is no menu (Electron sends it).
        all.insert(
            "Menu".into(),
            glib::variant::ObjectPath::try_from("/".to_string())
                .unwrap()
                .to_variant(),
        );
        assert_eq!(Props::parse(&all).menu, None);
    }

    fn node(
        id: i32,
        props: &[(&str, glib::Variant)],
        children: Vec<glib::Variant>,
    ) -> glib::Variant {
        let props: HashMap<String, glib::Variant> = props
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let children: Vec<glib::Variant> = children
            .into_iter()
            .map(|c| glib::Variant::from_variant(&c))
            .collect();
        (id, props, children).to_variant()
    }

    #[test]
    fn menus() {
        let quit = node(3, &[("label", "_Quit Steam".to_variant())], vec![]);
        let line = node(4, &[("type", "separator".to_variant())], vec![]);
        let hidden = node(
            5,
            &[("label", "x".to_variant()), ("visible", false.to_variant())],
            vec![],
        );
        let check = node(
            6,
            &[
                ("label", "Big __Picture".to_variant()),
                ("toggle-type", "checkmark".to_variant()),
                ("toggle-state", 1i32.to_variant()),
                ("enabled", false.to_variant()),
            ],
            vec![],
        );
        let library = node(7, &[("label", "Library".to_variant())], vec![]);
        let sub = node(
            2,
            &[
                ("label", "View".to_variant()),
                ("children-display", "submenu".to_variant()),
            ],
            vec![library],
        );
        let root = MenuNode::parse(&node(0, &[], vec![sub, check, hidden, line, quit])).unwrap();
        assert_eq!(root.children.len(), 5);
        assert_eq!(root.children[0].label, "View");
        assert_eq!(root.children[0].children[0].label, "Library");
        assert_eq!(root.children[1].label, "Big _Picture");
        assert_eq!(root.children[1].toggle, Some(true));
        assert!(!root.children[1].enabled);
        assert!(!root.children[2].visible);
        assert!(root.children[3].separator);
        assert_eq!(root.children[4].label, "Quit Steam");
        assert_eq!(root.children[4].toggle, None);
    }

    #[test]
    fn layout_reply() {
        // GetLayout answers (u(ia{sv}av)): the node is a structure, not a
        // variant, so it is read by position.
        let root = node(
            0,
            &[],
            vec![node(1, &[("label", "Show".to_variant())], vec![])],
        );
        let reply = glib::Variant::tuple_from_iter([7u32.to_variant(), root]);
        assert_eq!(reply.type_().as_str(), "(u(ia{sv}av))");
        assert!(
            reply.get::<(u32, glib::Variant)>().is_none(),
            "the trap this avoids"
        );
        let parsed = MenuNode::parse(&reply.child_value(1)).unwrap();
        assert_eq!(parsed.children[0].label, "Show");
    }
}
