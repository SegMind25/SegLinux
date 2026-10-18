//! The emoji selector, as Plasma's (Meta+.): GTK's emoji chooser, with
//! search, categories and the recently used ones; the chosen emoji is
//! copied, ready to paste with Ctrl+V.
use crate::notify::{self, Icon, Request, Urgency};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;

/// The notification confirming the copy.
pub fn copied(emoji: &str) -> Request {
    Request {
        app_name: "Emoji Selector".into(),
        replaces_id: 0,
        icon: Some(Icon::Name("face-smile-symbolic".into())),
        summary: format!("{emoji} copied"),
        body: "Paste it with Ctrl+V.".into(),
        actions: Vec::new(),
        urgency: Urgency::Low,
        timeout: Some(2500),
        transient: true,
        resident: false,
        desktop_entry: None,
    }
}

thread_local! {
    /// The selector on screen, if any: the shortcut closes it again.
    static OPEN: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

pub fn toggle(app: &gtk::Application, layer: bool, notifications: &notify::Center) {
    if let Some(window) = OPEN.with(|open| open.borrow_mut().take()) {
        window.close();
        return;
    }
    let window = gtk::Window::builder()
        .title("Emoji Selector")
        .decorated(false)
        .resizable(false)
        .build();
    window.add_css_class("emoji-window");
    if layer {
        // Centred above everything, taking the keyboard to type a search.
        window.init_layer_shell();
        window.set_namespace(Some("seglinux-emoji"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        // Centred near the top: the chooser opens downwards from there,
        // clear of the dock.
        window.set_anchor(Edge::Top, true);
        window.set_margin(Edge::Top, 90);
    } else {
        window.set_application(Some(app));
    }
    // The chooser is a popover: it opens from a point in the middle.
    let anchor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    anchor.set_size_request(1, 1);
    window.set_child(Some(&anchor));
    let chooser = gtk::EmojiChooser::new();
    chooser.set_has_arrow(false);
    chooser.set_parent(&anchor);
    let picked: Rc<RefCell<Option<String>>> = Rc::default();
    let chosen = picked.clone();
    chooser.connect_emoji_picked(move |chooser, emoji| {
        // The clipboard keeps it after the selector closes: the desktop
        // shell owns it until something else is copied.
        chooser.clipboard().set_text(emoji);
        chosen.replace(Some(emoji.to_string()));
    });
    let closing = window.downgrade();
    let notifications = notifications.clone();
    chooser.connect_closed(move |chooser| {
        chooser.unparent();
        if let Some(window) = closing.upgrade() {
            window.close();
        }
        if let Some(emoji) = picked.take() {
            notifications.add(copied(&emoji));
        }
    });
    window.connect_close_request(|_| {
        OPEN.with(|open| open.borrow_mut().take());
        gtk::glib::Propagation::Proceed
    });
    let popup = chooser.downgrade();
    window.connect_map(move |_| {
        let popup = popup.clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(chooser) = popup.upgrade() {
                chooser.popup();
            }
        });
    });
    OPEN.with(|open| open.replace(Some(window.clone())));
    window.present();
}

#[cfg(test)]
mod tests {
    use super::copied;

    #[test]
    fn copy_is_confirmed_briefly() {
        let request = copied("😀");
        assert_eq!(request.summary, "😀 copied");
        assert!(request.transient);
        assert_eq!(request.timeout, Some(2500));
    }
}
