//! Screenshots, as Windows' Print Screen and Snipping Tool (Win+Shift+S):
//! the whole screen or a chosen area, saved in Pictures/Screenshots and
//! copied, ready to paste. grim takes the picture; slurp chooses the area.
//!
//! Also the screen chooser of screen sharing (Discord, Vesktop, browsers):
//! xdg-desktop-portal-wlr runs `seglinux-shell --pick-output` and shares the
//! screen it prints.
use crate::notify::{self, Icon, Request, Urgency};
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use std::path::{Path, PathBuf};

pub fn folder() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Pictures)
        .unwrap_or_else(|| glib::home_dir().join("Pictures"))
        .join("Screenshots")
}

/// A new file in `dir` named after the time: Screenshot_2026-10-19_14-03-07.png,
/// with -2, -3... for several in one second.
pub fn new_path(dir: &Path, stamp: &str) -> PathBuf {
    let mut path = dir.join(format!("Screenshot_{stamp}.png"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("Screenshot_{stamp}-{n}.png"));
        n += 1;
    }
    path
}

fn request(summary: &str, body: &str, icon: Icon, timeout: u32) -> Request {
    Request {
        app_name: "Screenshot".into(),
        replaces_id: 0,
        icon: Some(icon),
        summary: summary.into(),
        body: body.into(),
        actions: Vec::new(),
        urgency: Urgency::Low,
        timeout: Some(timeout),
        transient: true,
        resident: false,
        desktop_entry: None,
    }
}

/// Runs a program to its end; its output when it succeeded.
async fn output(argv: &[&str]) -> Result<String, String> {
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let process = gio::Subprocess::newv(
        &argv,
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    )
    .map_err(|e| e.to_string())?;
    let (out, err) = process
        .communicate_utf8_future(None)
        .await
        .map_err(|e| e.to_string())?;
    if process.is_successful() {
        Ok(out.map(|s| s.to_string()).unwrap_or_default())
    } else {
        Err(err.map(|s| s.trim().to_string()).unwrap_or_default())
    }
}

/// Takes a screenshot: of a chosen area, else of every screen.
pub fn take(area: bool, notifications: &notify::Center) {
    let notifications = notifications.clone();
    glib::spawn_future_local(async move {
        match capture(area).await {
            Ok(Some(path)) => {
                let copied = gdk::Texture::from_file(&gio::File::for_path(&path))
                    .ok()
                    .zip(gdk::Display::default())
                    .map(|(texture, display)| display.clipboard().set_texture(&texture))
                    .is_some();
                let place = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|p| format!("Saved in Pictures/{}", p.to_string_lossy()))
                    .unwrap_or_default();
                let body = if copied {
                    format!("{place}. Copied: paste it with Ctrl+V.")
                } else {
                    format!("{place}.")
                };
                notifications.add(request("Screenshot taken", &body, Icon::File(path), 4000));
            }
            // The area was not chosen (Esc).
            Ok(None) => {}
            Err(message) => {
                notifications.add(request(
                    "No screenshot taken",
                    &message,
                    Icon::Name("dialog-error-symbolic".into()),
                    6000,
                ));
            }
        }
    });
}

async fn capture(area: bool) -> Result<Option<PathBuf>, String> {
    if !crate::common::installed("grim") || (area && !crate::common::installed("slurp")) {
        return Err("Screenshots need grim and slurp: sudo pacman -S grim slurp".into());
    }
    let geometry = if area {
        match output(&["slurp", "-d"]).await {
            Ok(geometry) if !geometry.trim().is_empty() => Some(geometry.trim().to_string()),
            _ => return Ok(None),
        }
    } else {
        None
    };
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp = glib::DateTime::now_local()
        .and_then(|now| now.format("%Y-%m-%d_%H-%M-%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "now".into());
    let path = new_path(&dir, &stamp);
    let target = path.to_string_lossy().to_string();
    let mut argv = vec!["grim"];
    if let Some(geometry) = &geometry {
        argv.extend(["-g", geometry]);
    }
    argv.push(&target);
    output(&argv).await?;
    Ok(Some(path))
}

/// What the screen chooser lists for a screen: "Dell U2419H (DP-1)".
fn describe(monitor: &gdk::Monitor) -> Option<(String, String)> {
    let connector = monitor.connector()?.to_string();
    let name = [monitor.manufacturer(), monitor.model()]
        .into_iter()
        .flatten()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty() && s != "Unknown")
        .collect::<Vec<_>>()
        .join(" ");
    let g = monitor.geometry();
    let label = if name.trim().is_empty() {
        format!("{connector} · {}×{}", g.width(), g.height())
    } else {
        format!("{name} ({connector}) · {}×{}", g.width(), g.height())
    };
    Some((connector, label))
}

/// `--pick-output`: prints the name of the screen to share. With one screen
/// it is that one; with several, the person picks. Fails when cancelled.
pub fn pick_output() -> glib::ExitCode {
    if gtk::init().is_err() {
        return glib::ExitCode::FAILURE;
    }
    let Some(display) = gdk::Display::default() else {
        return glib::ExitCode::FAILURE;
    };
    let mut monitors: Vec<(String, String)> = display
        .monitors()
        .iter::<gdk::Monitor>()
        .flatten()
        .filter_map(|m| describe(&m))
        .collect();
    monitors.sort();
    match monitors.as_slice() {
        [] => return glib::ExitCode::FAILURE,
        [(connector, _)] => {
            print!("{connector}");
            return glib::ExitCode::SUCCESS;
        }
        _ => {}
    }
    let main_loop = glib::MainLoop::new(None, false);
    let chosen: std::rc::Rc<std::cell::RefCell<Option<String>>> = Default::default();
    let window = gtk::Window::builder().title("Share a screen").build();
    window.add_css_class("emoji-window");
    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_namespace(Some("seglinux-share"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.set_margin_top(18);
    column.set_margin_bottom(18);
    column.set_margin_start(18);
    column.set_margin_end(18);
    let title = gtk::Label::new(Some("Which screen do you want to share?"));
    title.add_css_class("title-4");
    column.append(&title);
    for (connector, label) in monitors {
        let button = gtk::Button::with_label(&label);
        let chosen = chosen.clone();
        let window = window.downgrade();
        button.connect_clicked(move |_| {
            chosen.replace(Some(connector.clone()));
            if let Some(window) = window.upgrade() {
                window.close();
            }
        });
        column.append(&button);
    }
    let cancel = gtk::Button::with_label("Cancel");
    let closing = window.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(window) = closing.upgrade() {
            window.close();
        }
    });
    column.append(&cancel);
    window.set_child(Some(&column));
    let keys = gtk::EventControllerKey::new();
    let closing = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape
            && let Some(window) = closing.upgrade()
        {
            window.close();
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    let quit = main_loop.clone();
    window.connect_close_request(move |_| {
        quit.quit();
        glib::Propagation::Proceed
    });
    window.present();
    main_loop.run();
    match chosen.take() {
        Some(connector) => {
            print!("{connector}");
            glib::ExitCode::SUCCESS
        }
        None => glib::ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::new_path;

    #[test]
    fn names_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("seglinux-shots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = new_path(&dir, "2026-10-19_14-03-07");
        assert_eq!(
            first.file_name().unwrap(),
            "Screenshot_2026-10-19_14-03-07.png"
        );
        std::fs::write(&first, "").unwrap();
        let second = new_path(&dir, "2026-10-19_14-03-07");
        assert_eq!(
            second.file_name().unwrap(),
            "Screenshot_2026-10-19_14-03-07-2.png"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
