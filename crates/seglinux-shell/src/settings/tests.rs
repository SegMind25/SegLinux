use super::*;
use crate::config::{AdaptiveSync, OverlayDetail};
use segrun::config::AppOverride;

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut all = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        all.extend(descendants(&current));
        child = current.next_sibling();
    }
    all
}

/// The Settings window's widgets; `find("accent")` by widget name.
struct Ui(Vec<gtk::Widget>);
impl Ui {
    fn find<T: IsA<gtk::Widget>>(&self, name: &str) -> T {
        self.0
            .iter()
            .find(|w| w.widget_name() == name)
            .and_then(|w| w.clone().downcast::<T>().ok())
            .unwrap_or_else(|| panic!("no widget named {name}"))
    }
    fn click(&self, name: &str) {
        self.find::<gtk::Button>(name).emit_clicked();
    }
    fn status(&self) -> String {
        self.find::<gtk::Label>("status").text().to_string()
    }
}

fn read_shell() -> Config {
    Config::parse(&std::fs::read_to_string(Config::path()).unwrap()).unwrap()
}

#[test]
#[ignore = "requires an isolated XDG_CONFIG_HOME and GTK display"]
fn settings_apply_validates_and_preserves_game_overrides() {
    let root =
        std::env::var("SEGLINUX_SETTINGS_TEST_ROOT").expect("isolated test directory required");
    assert!(Config::path().starts_with(&root));
    assert!(GameConfig::user_path().starts_with(&root));
    assert!(compositor::user_dir().starts_with(&root));
    assert!(crate::gaming::mangohud_path().starts_with(&root));
    // SAFETY: the test runs alone (--test-threads=1) before GTK starts threads.
    unsafe { std::env::remove_var("LABWC_PID") };
    gtk::init().unwrap();
    let app = gtk::Application::builder()
        .application_id("org.seglinux.Settings.Test")
        .build();
    app.register(gio::Cancellable::NONE).unwrap();
    let game = GameConfig::parse(
        "[env]\nKEEP_ME = 'yes'\n[[app]]\nmatch = 'game.exe'\nrunner = 'umu'\nargs = ['-dx11']\n",
    )
    .unwrap();
    save_text(&GameConfig::user_path(), &toml::to_string(&game).unwrap()).unwrap();
    let config = Config::default();
    for theme in [crate::config::Theme::Dark, crate::config::Theme::Light] {
        let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = errors.clone();
        let provider = gtk::CssProvider::new();
        provider.connect_parsing_error(move |_, _, error| {
            captured.borrow_mut().push(error.to_string())
        });
        provider.load_from_data(
            &Config {
                theme,
                ..config.clone()
            }
            .css(),
        );
        assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
    }
    let css = gtk::CssProvider::new();
    open(&app, &config, "gaming", &css);
    let window = app
        .windows()
        .into_iter()
        .find(|w| w.title().as_deref() == Some("SegLinux Settings"))
        .unwrap();
    let ui = Ui(descendants(window.upcast_ref()));

    // Nothing changed: nothing is written, not even segrun's file again.
    let games_before = std::fs::read_to_string(GameConfig::user_path()).unwrap();
    ui.click("apply");
    assert!(ui.status().starts_with("No changes"), "{}", ui.status());
    assert!(!Config::path().exists());
    assert_eq!(
        std::fs::read_to_string(GameConfig::user_path()).unwrap(),
        games_before
    );

    // Appearance is saved; an invalid accent is refused without writing.
    let accent: gtk::Entry = ui.find("accent");
    accent.set_text("#ABC123");
    ui.click("apply");
    assert_eq!(read_shell().accent, "#abc123");
    let before = std::fs::read_to_string(Config::path()).unwrap();
    accent.set_text("invalid; CSS");
    ui.click("apply");
    assert!(
        ui.status().starts_with("Not saved: Accent"),
        "{}",
        ui.status()
    );
    assert_eq!(std::fs::read_to_string(Config::path()).unwrap(), before);
    accent.set_text("#abc123");

    // Window settings go to the compositor's own file.
    ui.find::<gtk::SpinButton>("workspaces").set_value(4.0);
    ui.find::<gtk::Entry>("keyboard-layout").set_text("us, ara");
    ui.click("apply");
    let shell = read_shell();
    assert_eq!(shell.windows.workspaces, 4);
    assert_eq!(shell.input.keyboard_layout, "us,ara");
    let labwc = compositor::user_dir();
    assert!(
        std::fs::read_to_string(labwc.join("rc.xml"))
            .unwrap()
            .contains("<desktops number=\"4\" />")
    );
    assert!(
        std::fs::read_to_string(labwc.join("environment"))
            .unwrap()
            .contains("XKB_DEFAULT_LAYOUT=us,ara")
    );
    assert!(
        std::fs::symlink_metadata(labwc.join("autostart"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    ui.find::<gtk::Entry>("keyboard-layout")
        .set_text("us;rm -rf");
    ui.click("apply");
    assert!(
        ui.status().starts_with("Not saved: Keyboard"),
        "{}",
        ui.status()
    );
    ui.find::<gtk::Entry>("keyboard-layout").set_text("us,ara");

    // Gaming: segrun keeps its profiles and variables, MangoHud gets a file.
    ui.find::<gtk::Switch>("gamescope").set_active(true);
    ui.find::<gtk::SpinButton>("fps-limit").set_value(141.0);
    ui.find::<gtk::Entry>("profile-pattern")
        .set_text("Other*.exe");
    ui.click("save-profile");
    ui.click("apply");
    assert!(ui.status().starts_with("Saved"), "{}", ui.status());
    let saved = GameConfig::load(None).unwrap().0;
    assert!(saved.defaults.gamescope);
    assert_eq!(saved.env, game.env);
    assert_eq!(saved.apps[0], game.apps[0]);
    assert_eq!(
        saved.apps[1],
        AppOverride {
            pattern: "Other*.exe".into(),
            ..AppOverride::default()
        }
    );
    assert_eq!(read_shell().overlay.fps_limit, 141);
    let mangohud = std::fs::read_to_string(crate::gaming::mangohud_path()).unwrap();
    assert!(mangohud.contains("fps_limit=141"));

    // A hand-written MangoHud configuration is never replaced.
    std::fs::write(crate::gaming::mangohud_path(), "fps_limit=30\n").unwrap();
    ui.find::<gtk::SpinButton>("fps-limit").set_value(60.0);
    ui.click("apply");
    assert!(ui.status().contains("your own MangoHud"), "{}", ui.status());
    assert_eq!(
        std::fs::read_to_string(crate::gaming::mangohud_path()).unwrap(),
        "fps_limit=30\n"
    );

    // Restore defaults shows a page's defaults; Apply saves them.
    ui.click("reset");
    ui.click("apply");
    let shell = read_shell();
    assert_eq!(shell.overlay.detail, OverlayDetail::Standard);
    assert_eq!(shell.windows.adaptive_sync, AdaptiveSync::Fullscreen);
    assert_eq!(
        shell.windows.workspaces, 4,
        "other pages keep their settings"
    );
    let saved = GameConfig::load(None).unwrap().0;
    assert!(!saved.defaults.gamescope);
    assert_eq!(saved.apps.len(), 2, "profiles are not settings to reset");
    window.close();
}
