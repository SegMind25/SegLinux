//! A stand-in tray application for testing the shell's system tray: it
//! registers a StatusNotifierItem (as Steam or Discord do, through
//! libappindicator) with a menu, and prints what the tray asks of it.
//!
//!   cargo run -p seglinux-shell --example tray-test-item
use gtk::{gio, glib, prelude::*};
use std::collections::HashMap;

const ITEM_XML: &str = r#"<node><interface name="org.kde.StatusNotifierItem">
  <method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
  <method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
  <property name="Id" type="s" access="read"/><property name="Title" type="s" access="read"/>
  <property name="Status" type="s" access="read"/><property name="IconName" type="s" access="read"/>
  <property name="ToolTip" type="(sa(iiay)ss)" access="read"/><property name="Menu" type="o" access="read"/>
  <property name="ItemIsMenu" type="b" access="read"/>
</interface></node>"#;
const MENU_XML: &str = r#"<node><interface name="com.canonical.dbusmenu">
  <method name="GetLayout"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/>
    <arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/></method>
  <method name="Event"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/></method>
  <method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
</interface></node>"#;

fn node(id: i32, props: &[(&str, glib::Variant)], children: Vec<glib::Variant>) -> glib::Variant {
    let props: HashMap<String, glib::Variant> = props
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let children: Vec<glib::Variant> = children.iter().map(glib::Variant::from_variant).collect();
    (id, props, children).to_variant()
}

fn layout() -> glib::Variant {
    let library = node(11, &[("label", "_Library".to_variant())], vec![]);
    let store = node(12, &[("label", "Store".to_variant())], vec![]);
    let view = node(
        10,
        &[
            ("label", "View".to_variant()),
            ("children-display", "submenu".to_variant()),
        ],
        vec![library, store],
    );
    let big = node(
        20,
        &[
            ("label", "Big Picture".to_variant()),
            ("toggle-type", "checkmark".to_variant()),
            ("toggle-state", 1i32.to_variant()),
        ],
        vec![],
    );
    let line = node(30, &[("type", "separator".to_variant())], vec![]);
    let quit = node(40, &[("label", "_Quit Test".to_variant())], vec![]);
    node(0, &[], vec![view, big, line, quit])
}

fn main() {
    let main_loop = glib::MainLoop::new(None, false);
    let connection =
        gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).expect("a session bus");
    let item = gio::DBusNodeInfo::for_xml(ITEM_XML).unwrap();
    let item = item.lookup_interface("org.kde.StatusNotifierItem").unwrap();
    let path = "/org/ayatana/NotificationItem/test";
    connection
        .register_object(path, &item)
        .method_call(|_, _, _, _, method, params, invocation| {
            println!("item {method} {params}");
            invocation.return_value(None);
        })
        .property(|_, _, _, _, name| match name {
            "Id" => "tray-test".to_variant(),
            "Title" => "Tray test".to_variant(),
            "Status" => "Active".to_variant(),
            "IconName" => "applications-games".to_variant(),
            "ToolTip" => (
                "",
                Vec::<(i32, i32, Vec<u8>)>::new(),
                "Tray test",
                "2 friends online",
            )
                .to_variant(),
            "Menu" => glib::variant::ObjectPath::try_from("/MenuBar".to_string())
                .unwrap()
                .to_variant(),
            "ItemIsMenu" => false.to_variant(),
            _ => ().to_variant(),
        })
        .build()
        .unwrap();
    let menu = gio::DBusNodeInfo::for_xml(MENU_XML).unwrap();
    let menu = menu.lookup_interface("com.canonical.dbusmenu").unwrap();
    let quit = main_loop.clone();
    connection
        .register_object("/MenuBar", &menu)
        .method_call(move |_, _, _, _, method, params, invocation| {
            println!("menu {method} {params}");
            match method {
                "GetLayout" => invocation.return_value(Some(&glib::Variant::tuple_from_iter([
                    1u32.to_variant(),
                    layout(),
                ]))),
                "AboutToShow" => invocation.return_value(Some(&(false,).to_variant())),
                _ => {
                    if params
                        .get::<(i32, String, glib::Variant, u32)>()
                        .is_some_and(|(id, ..)| id == 40)
                    {
                        quit.quit();
                    }
                    invocation.return_value(None);
                }
            }
        })
        .build()
        .unwrap();
    connection
        .call_sync(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
            "RegisterStatusNotifierItem",
            Some(&(path,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
        )
        .expect("the tray accepts the item");
    println!("registered");
    main_loop.run();
}
