//! Push-to-talk in Discord, and other X11 applications' global shortcuts
//! (OBS, overlays), while a Wayland window has the focus. X11 applications
//! hear keys only through XWayland, which gets them only while one of its
//! windows is focused; the keys such shortcuts use are passed on to it as
//! well (XTEST). No X11 window has the focus then, so nothing is typed
//! anywhere: only applications listening for shortcuts notice. In
//! compositor.toml:
//!
//! ```toml
//! x11_keys = "shortcuts"  # the default: Ctrl, Alt, Shift or Super alone,
//!                         # F1-F24, Pause, Scroll Lock, Insert, Menu, any key
//!                         # held with Ctrl, Alt or Super, mouse side buttons
//! x11_keys = "all"        # every key (X11 applications could read typing)
//! x11_keys = "off"
//! ```

use std::{
    collections::HashSet,
    sync::mpsc::{self, Receiver, Sender},
};

use x11rb::{
    connection::Connection,
    protocol::{xproto, xtest::ConnectionExt},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Forward {
    #[default]
    Shortcuts,
    All,
    Off,
}

impl Forward {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "shortcuts" => Some(Forward::Shortcuts),
            "all" => Some(Forward::All),
            "off" => Some(Forward::Off),
            _ => None,
        }
    }
}

/// Linux key codes (+8: X11's) of keys used alone as shortcuts.
const SHORTCUT_KEYS: &[u32] = &[
    29, 97, 42, 54, 56, 100, 125, 126, // Ctrl, Shift, Alt, Super (left, right)
    59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88, // F1-F12
    183, 184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, // F13-F24
    70, 119, 110, 127, // Scroll Lock, Pause, Insert, Menu
];
/// Mouse side buttons (Linux) and their X11 numbers.
const SIDE_BUTTONS: &[(u32, u8)] = &[(0x113, 8), (0x114, 9), (0x116, 8), (0x115, 9)];

#[derive(Default)]
pub struct X11Keys {
    tx: Option<Sender<(u8, u8)>>,
    /// Passed on and still held: their release follows, wherever the
    /// focus went (else a key stays down for X11).
    down: HashSet<(bool, u32)>,
}

impl X11Keys {
    /// XWayland is up on `display` (":0").
    pub fn start(&mut self, display: String) {
        let (tx, rx) = mpsc::channel();
        // Its own thread: XWayland may be waiting for the compositor.
        std::thread::spawn(move || run(&display, rx));
        self.tx = Some(tx);
        self.down.clear();
    }

    /// A key (X11 key code) went down or up. `pass`: a Wayland window
    /// has the focus and the screen is not locked; `mods`: Ctrl, Alt or
    /// Super is held.
    pub fn key(&mut self, mode: Forward, keycode: u32, pressed: bool, pass: bool, mods: bool) {
        let wanted = match mode {
            Forward::Off => false,
            Forward::All => true,
            Forward::Shortcuts => mods || SHORTCUT_KEYS.contains(&keycode.saturating_sub(8)),
        };
        let Ok(detail) = u8::try_from(keycode) else {
            return;
        };
        self.send((false, keycode), pressed, pass && wanted, detail);
    }

    /// A mouse button (Linux code) went down or up.
    pub fn button(&mut self, mode: Forward, button: u32, pressed: bool, pass: bool) {
        let Some(&(_, detail)) = SIDE_BUTTONS.iter().find(|(b, _)| *b == button) else {
            return;
        };
        self.send(
            (true, button),
            pressed,
            pass && mode != Forward::Off,
            detail,
        );
    }

    fn send(&mut self, id: (bool, u32), pressed: bool, wanted: bool, detail: u8) {
        let Some(tx) = &self.tx else {
            return;
        };
        let go = if pressed {
            wanted && self.down.insert(id)
        } else {
            self.down.remove(&id)
        };
        if go {
            let kind = match (id.0, pressed) {
                (false, true) => xproto::KEY_PRESS_EVENT,
                (false, false) => xproto::KEY_RELEASE_EVENT,
                (true, true) => xproto::BUTTON_PRESS_EVENT,
                (true, false) => xproto::BUTTON_RELEASE_EVENT,
            };
            let _ = tx.send((kind, detail));
        }
    }
}

fn run(display: &str, rx: Receiver<(u8, u8)>) {
    let conn = match x11rb::connect(Some(display)) {
        Ok((conn, _)) => conn,
        Err(e) => {
            tracing::warn!("keys for X11 applications: {e}");
            return;
        }
    };
    for (kind, detail) in rx {
        let sent = conn
            .xtest_fake_input(kind, detail, x11rb::CURRENT_TIME, x11rb::NONE, 0, 0, 0)
            .map(|_| ())
            .and_then(|_| conn.flush());
        if let Err(e) = sent {
            tracing::warn!("keys for X11 applications: {e}");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_follow_presses() {
        let (tx, rx) = mpsc::channel();
        let mut k = X11Keys {
            tx: Some(tx),
            ..Default::default()
        };
        let f = Forward::Shortcuts;
        // A letter alone stays private; F9 and Ctrl+K are passed on.
        k.key(f, 30 + 8, true, true, false);
        k.key(f, 67 + 8, true, true, false);
        k.key(f, 37 + 8, true, true, true);
        // K let go after the focus moved to an X11 window: still released.
        k.key(f, 37 + 8, false, false, false);
        k.key(f, 30 + 8, false, true, false);
        k.button(f, 0x113, true, true);
        k.button(f, 0x110, true, true);
        let sent: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            sent,
            [
                (xproto::KEY_PRESS_EVENT, 75),
                (xproto::KEY_PRESS_EVENT, 45),
                (xproto::KEY_RELEASE_EVENT, 45),
                (xproto::BUTTON_PRESS_EVENT, 8),
            ]
        );
    }
}
