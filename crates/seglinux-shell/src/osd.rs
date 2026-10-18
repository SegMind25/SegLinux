//! The volume and brightness keys, as in Plasma: the change, then a bar
//! above the dock for a moment (an on-screen display).
//!
//! labwc's key bindings call the shell's `volume` and `brightness` actions
//! (`gapplication action org.seglinux.Shell volume "'up'"`). The volume is
//! PipeWire's (wpctl); the brightness is the backlight's, set through
//! elogind, which allows the user at the computer without root.
use gtk::{gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// "Volume: 0.45" or "Volume: 0.45 [MUTED]" (wpctl get-volume).
pub fn parse_volume(text: &str) -> Option<(f64, bool)> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let level: f64 = rest.split_whitespace().next()?.parse().ok()?;
    Some((level, rest.contains("[MUTED]")))
}

pub fn volume_icon(level: f64, muted: bool) -> &'static str {
    match level {
        _ if muted || level <= 0.0 => "audio-volume-muted-symbolic",
        l if l < 0.34 => "audio-volume-low-symbolic",
        l if l < 0.67 => "audio-volume-medium-symbolic",
        _ => "audio-volume-high-symbolic",
    }
}

/// The wpctl commands for a volume key, then the device to read back.
pub fn volume_commands(change: &str) -> (Vec<Vec<&'static str>>, &'static str) {
    const SINK: &str = "@DEFAULT_AUDIO_SINK@";
    const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";
    match change {
        // Raising the volume of a muted output unmutes it, as in Plasma.
        "up" => (
            vec![
                vec!["set-mute", SINK, "0"],
                vec!["set-volume", "-l", "1.0", SINK, "5%+"],
            ],
            SINK,
        ),
        "down" => (vec![vec!["set-volume", SINK, "5%-"]], SINK),
        "mute" => (vec![vec!["set-mute", SINK, "toggle"]], SINK),
        "mic-mute" => (vec![vec!["set-mute", SOURCE, "toggle"]], SOURCE),
        _ => (vec![], SINK),
    }
}

/// The next backlight level: 5% steps of the maximum, never fully dark.
pub fn brightness_step(current: u32, max: u32, up: bool) -> u32 {
    let step = (max / 20).max(1);
    let lowest = (max / 100).max(1).min(max);
    if up {
        current.saturating_add(step).min(max)
    } else {
        current.saturating_sub(step).max(lowest)
    }
}

/// The screen's backlight (/sys/class/backlight/NAME): laptops have one,
/// desktop screens usually not.
fn backlight() -> Option<(PathBuf, String, u32, u32)> {
    let read = |p: &Path| -> Option<u32> { std::fs::read_to_string(p).ok()?.trim().parse().ok() };
    let mut entries: Vec<_> = std::fs::read_dir("/sys/class/backlight")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    // Firmware (ACPI) interfaces last: the GPU's own are preferred.
    entries.sort_by_key(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("acpi"))
    });
    entries.into_iter().find_map(|path| {
        let max = read(&path.join("max_brightness")).filter(|m| *m > 0)?;
        let current = read(&path.join("brightness"))?;
        let name = path.file_name()?.to_string_lossy().to_string();
        Some((path, name, current, max))
    })
}

/// The display's window and the widgets updated in it.
struct Shown {
    window: gtk::Window,
    image: gtk::Image,
    bar: gtk::LevelBar,
    label: gtk::Label,
}

struct Inner {
    shown: RefCell<Option<Shown>>,
    timer: RefCell<Option<glib::SourceId>>,
    /// Volume keys waiting their turn: one change at a time, or two quick
    /// presses read the same level and one step is lost.
    pending: RefCell<VecDeque<String>>,
    busy: Cell<bool>,
    app: gtk::Application,
    layer: bool,
}

#[derive(Clone)]
pub struct Osd(Rc<Inner>);

impl Osd {
    pub fn new(app: &gtk::Application, layer: bool) -> Osd {
        Osd(Rc::new(Inner {
            shown: RefCell::default(),
            timer: RefCell::default(),
            pending: RefCell::default(),
            busy: Cell::new(false),
            app: app.clone(),
            layer,
        }))
    }

    fn window(&self) -> Shown {
        let window = gtk::Window::builder()
            .decorated(false)
            .resizable(false)
            .build();
        window.add_css_class("osd-window");
        if self.0.layer {
            window.init_layer_shell();
            window.set_namespace(Some("seglinux-osd"));
            window.set_layer(Layer::Overlay);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_anchor(Edge::Bottom, true);
            // Above the dock, whose area the compositor keeps clear.
            window.set_margin(Edge::Bottom, 40);
        } else {
            window.set_application(Some(&self.0.app));
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("osd");
        let image = gtk::Image::new();
        image.set_pixel_size(22);
        row.append(&image);
        let bar = gtk::LevelBar::for_interval(0.0, 1.0);
        bar.set_size_request(200, -1);
        bar.set_valign(gtk::Align::Center);
        row.append(&bar);
        let label = gtk::Label::new(None);
        label.set_width_chars(5);
        row.append(&label);
        window.set_child(Some(&row));
        Shown {
            window,
            image,
            bar,
            label,
        }
    }

    /// Shows `icon`, a bar at `level` (0 to 1) and `text` for 1.5 s. Keys
    /// pressed in a row update the display on screen (no flicker).
    pub fn show(&self, icon: &str, level: f64, text: &str) {
        if let Some(timer) = self.0.timer.take() {
            timer.remove();
        }
        let mut shown = self.0.shown.borrow_mut();
        let parts = shown.get_or_insert_with(|| self.window());
        parts.image.set_icon_name(Some(icon));
        parts.bar.set_value(level.clamp(0.0, 1.0));
        parts.label.set_text(text);
        parts.window.present();
        drop(shown);
        let weak = Rc::downgrade(&self.0);
        let timer = glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            if let Some(inner) = weak.upgrade() {
                inner.timer.take();
                if let Some(parts) = inner.shown.take() {
                    parts.window.destroy();
                }
            }
        });
        self.0.timer.replace(Some(timer));
    }

    /// A volume key: "up", "down", "mute", "mic-mute" (or "show").
    pub fn volume(&self, change: &str) {
        if !crate::common::installed("wpctl") {
            return;
        }
        {
            let mut pending = self.0.pending.borrow_mut();
            // A held key repeats faster than PipeWire answers: a short
            // queue, so the volume stops soon after the key is released.
            if pending.len() < 3 {
                pending.push_back(change.to_string());
            }
        }
        if !self.0.busy.replace(true) {
            self.next_volume();
        }
    }

    fn next_volume(&self) {
        let Some(change) = self.0.pending.borrow_mut().pop_front() else {
            self.0.busy.set(false);
            return;
        };
        let change = change.as_str();
        let (commands, device) = volume_commands(change);
        let commands: Vec<Vec<String>> = commands
            .into_iter()
            .map(|c| c.into_iter().map(String::from).collect())
            .collect();
        let microphone = change == "mic-mute";
        let osd = self.clone();
        glib::spawn_future_local(async move {
            let output = gio::spawn_blocking(move || {
                for args in commands {
                    let _ = std::process::Command::new("wpctl").args(&args).status();
                }
                std::process::Command::new("wpctl")
                    .args(["get-volume", device])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            })
            .await
            .ok()
            .flatten();
            if let Some((level, muted)) = output.as_deref().and_then(parse_volume) {
                let (icon, text) = if microphone {
                    (
                        if muted {
                            "microphone-sensitivity-muted-symbolic"
                        } else {
                            "audio-input-microphone-symbolic"
                        },
                        if muted {
                            "Off".to_string()
                        } else {
                            "On".to_string()
                        },
                    )
                } else {
                    (
                        volume_icon(level, muted),
                        if muted {
                            "Muted".to_string()
                        } else {
                            format!("{:.0}%", level * 100.0)
                        },
                    )
                };
                osd.show(icon, if muted { 0.0 } else { level }, &text);
            }
            osd.next_volume();
        });
    }

    /// A brightness key: "up" or "down".
    pub fn brightness(&self, change: &str) {
        let Some((path, name, current, max)) = backlight() else {
            return;
        };
        let value = brightness_step(current, max, change == "up");
        let osd = self.clone();
        let shown = move || {
            let now = std::fs::read_to_string(path.join("brightness"))
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
                .unwrap_or(value);
            let level = f64::from(now) / f64::from(max);
            osd.show(
                "display-brightness-symbolic",
                level,
                &format!("{:.0}%", level * 100.0),
            );
        };
        let Ok(system) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) else {
            return;
        };
        system.call(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1/session/auto",
            "org.freedesktop.login1.Session",
            "SetBrightness",
            Some(&("backlight", name.as_str(), value).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
            move |result| {
                if let Err(error) = result {
                    // Without elogind's help, brightnessctl if installed.
                    if crate::common::installed("brightnessctl") {
                        let _ = std::process::Command::new("brightnessctl")
                            .args(["-q", "set", &value.to_string()])
                            .status();
                    } else {
                        eprintln!("Cannot change the brightness: {error}");
                        return;
                    }
                }
                shown();
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes() {
        assert_eq!(parse_volume("Volume: 0.45\n"), Some((0.45, false)));
        assert_eq!(parse_volume("Volume: 0.00 [MUTED]"), Some((0.0, true)));
        assert_eq!(parse_volume("No such device"), None);
        assert_eq!(volume_icon(0.5, true), "audio-volume-muted-symbolic");
        assert_eq!(volume_icon(0.2, false), "audio-volume-low-symbolic");
        assert_eq!(volume_icon(0.5, false), "audio-volume-medium-symbolic");
        assert_eq!(volume_icon(1.0, false), "audio-volume-high-symbolic");
        let (up, device) = volume_commands("up");
        assert_eq!(up[0], ["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
        assert_eq!(device, "@DEFAULT_AUDIO_SINK@");
        assert_eq!(volume_commands("mic-mute").1, "@DEFAULT_AUDIO_SOURCE@");
        assert!(volume_commands("nonsense").0.is_empty());
    }

    #[test]
    fn brightness_steps() {
        assert_eq!(brightness_step(500, 1000, true), 550);
        assert_eq!(brightness_step(980, 1000, true), 1000);
        assert_eq!(brightness_step(500, 1000, false), 450);
        assert_eq!(brightness_step(30, 1000, false), 10, "never fully dark");
        assert_eq!(brightness_step(3, 7, false), 2);
        assert_eq!(brightness_step(1, 1, false), 1);
    }
}
