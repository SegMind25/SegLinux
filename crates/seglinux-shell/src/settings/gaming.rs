//! Gaming: how segrun runs Windows games, gamescope, the performance
//! overlay, variable refresh rate and tearing, the Gaming Mode session,
//! environment variables and per-game profiles.
use super::widgets::{self, index_of, selected};
use super::{Page, State};
use crate::common;
use crate::config::{AdaptiveSync, OverlayDetail, OverlayPosition, Tearing};
use crate::gaming::{self as g, Opt};
use gtk::prelude::*;
use segrun::config::{AppOverride, Toggle};
use std::{cell::RefCell, rc::Rc};

const TOGGLES: [Toggle; 3] = [Toggle::Auto, Toggle::Always, Toggle::Never];
/// For per-game profiles, "Default" keeps the general setting.
const OPTIONAL_TOGGLES: [Option<Toggle>; 4] = [
    None,
    Some(Toggle::Auto),
    Some(Toggle::Always),
    Some(Toggle::Never),
];
const OPTIONAL_BOOLS: [Option<bool>; 3] = [None, Some(true), Some(false)];
const POSITIONS: [OverlayPosition; 5] = [
    OverlayPosition::TopLeft,
    OverlayPosition::TopRight,
    OverlayPosition::BottomLeft,
    OverlayPosition::BottomRight,
    OverlayPosition::TopCenter,
];
const DETAILS: [OverlayDetail; 4] = [
    OverlayDetail::FpsOnly,
    OverlayDetail::Horizontal,
    OverlayDetail::Standard,
    OverlayDetail::Detailed,
];
const SYNC: [AdaptiveSync; 3] = [
    AdaptiveSync::No,
    AdaptiveSync::Fullscreen,
    AdaptiveSync::Yes,
];
const TEARING: [Tearing; 3] = [Tearing::No, Tearing::Yes, Tearing::Fullscreen];

/// The form that adds or changes a per-game profile.
struct ProfileForm {
    pattern: gtk::Entry,
    runner: gtk::Entry,
    prefix: gtk::Entry,
    gamemode: gtk::DropDown,
    dxvk: gtk::DropDown,
    vkd3d: gtk::DropDown,
    gamescope: gtk::DropDown,
    virtual_desktop: gtk::Entry,
    dlls: gtk::TextView,
    env: gtk::TextView,
    /// The profile being changed: its fields without a control are kept.
    editing: RefCell<AppOverride>,
}

fn optional(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

impl ProfileForm {
    fn show(&self, app: &AppOverride) {
        self.pattern.set_text(&app.pattern);
        self.runner.set_text(app.runner.as_deref().unwrap_or(""));
        self.prefix.set_text(app.prefix.as_deref().unwrap_or(""));
        self.gamemode
            .set_selected(index_of(&OPTIONAL_TOGGLES, &app.gamemode));
        self.dxvk
            .set_selected(index_of(&OPTIONAL_TOGGLES, &app.dxvk));
        self.vkd3d
            .set_selected(index_of(&OPTIONAL_TOGGLES, &app.vkd3d));
        self.gamescope
            .set_selected(index_of(&OPTIONAL_BOOLS, &app.gamescope));
        self.virtual_desktop
            .set_text(app.virtual_desktop.as_deref().unwrap_or(""));
        self.dlls.buffer().set_text(&app.dll_overrides.join("\n"));
        self.env.buffer().set_text(&g::format_env(&app.env));
        *self.editing.borrow_mut() = app.clone();
    }

    fn read(&self) -> Result<AppOverride, String> {
        let mut app = self.editing.borrow().clone();
        app.pattern = self.pattern.text().trim().to_string();
        app.runner = optional(&self.runner.text());
        app.prefix = optional(&self.prefix.text());
        app.gamemode = selected(&OPTIONAL_TOGGLES, &self.gamemode);
        app.dxvk = selected(&OPTIONAL_TOGGLES, &self.dxvk);
        app.vkd3d = selected(&OPTIONAL_TOGGLES, &self.vkd3d);
        app.gamescope = selected(&OPTIONAL_BOOLS, &self.gamescope);
        app.virtual_desktop = optional(&self.virtual_desktop.text());
        app.dll_overrides = widgets::text(&self.dlls)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        app.env = g::parse_env(&widgets::text(&self.env))?;
        g::validate_profile(&app)?;
        Ok(app)
    }
}

pub struct Gaming {
    root: gtk::ScrolledWindow,
    /// Controls of segrun's settings, unusable when its file is invalid.
    segrun_parts: Vec<gtk::Widget>,
    runner: gtk::Entry,
    prefix: gtk::Entry,
    isolate: gtk::Switch,
    gamemode: gtk::DropDown,
    dxvk: gtk::DropDown,
    vkd3d: gtk::DropDown,
    esync: gtk::Switch,
    fsync: gtk::Switch,
    debug: gtk::Switch,
    hud: gtk::Switch,
    position: gtk::DropDown,
    detail: gtk::DropDown,
    fps_limit: gtk::SpinButton,
    gamescope: gtk::Switch,
    fullscreen: gtk::Switch,
    borderless: gtk::Switch,
    game_size: (gtk::SpinButton, gtk::SpinButton),
    output_size: (gtk::SpinButton, gtk::SpinButton),
    frame_rate: gtk::SpinButton,
    filter: gtk::DropDown,
    sharpness: gtk::SpinButton,
    gs_adaptive: gtk::Switch,
    gs_hdr: gtk::Switch,
    extra: gtk::TextView,
    vrr: gtk::DropDown,
    tearing: gtk::DropDown,
    mode_size: (gtk::SpinButton, gtk::SpinButton),
    mode_refresh: gtk::SpinButton,
    mode_adaptive: gtk::Switch,
    mode_hdr: gtk::Switch,
    mode_overlay: gtk::Switch,
    env: gtk::TextView,
    apps: Rc<RefCell<Vec<AppOverride>>>,
    render_profiles: Rc<dyn Fn()>,
}

impl Gaming {
    pub fn new(state: &State) -> Self {
        let (root, page) = widgets::page();
        if let Err(error) = &state.games {
            let banner = widgets::note(&format!(
                "The Windows games settings cannot be read: {error}. Correct or remove {} to change them here; the other settings on this page still work.",
                segrun::config::Config::user_path().display()
            ));
            banner.add_css_class("error");
            page.append(&banner);
        }
        let mut segrun_parts: Vec<gtk::Widget> = Vec::new();

        let games = widgets::section(
            &page,
            "Windows games and programs",
            Some(
                "How segrun runs .exe files: double-click one, or use segrun run. Steam and Lutris keep their own settings.",
            ),
        );
        segrun_parts.push(games.clone().upcast());
        let runner = widgets::entry(
            &games,
            "Runner",
            Some("wine, umu (Proton through umu-launcher), proton:/folder or umu:/folder."),
            "wine",
        );
        runner.set_widget_name("runner");
        let mut choices = vec![
            ("Wine from the system".to_string(), "wine".to_string()),
            ("Proton through umu-launcher".to_string(), "umu".to_string()),
        ];
        for (name, dir) in g::proton_builds() {
            choices.push((
                format!("{name} (Proton)"),
                format!("proton:{}", dir.display()),
            ));
            choices.push((
                format!("{name} through umu-launcher"),
                format!("umu:{}", dir.display()),
            ));
        }
        let labels: Vec<&str> = std::iter::once("Choose a runner…")
            .chain(choices.iter().map(|(label, _)| label.as_str()))
            .collect();
        let pick = widgets::dropdown(
            &games,
            "Installed runners",
            Some("Proton builds from Steam and compatibilitytools.d (GE-Proton) appear here."),
            &labels,
        );
        let target = runner.clone();
        pick.connect_selected_notify(move |pick| {
            if let Some((_, value)) = (pick.selected() as usize)
                .checked_sub(1)
                .and_then(|i| choices.get(i))
            {
                target.set_text(value);
            }
        });
        let prefix = widgets::entry(
            &games,
            "Windows environment",
            Some("The shared prefix (\"C: drive\") programs install into."),
            "default",
        );
        let isolate = widgets::switch(
            &games,
            "Separate environment for each installer",
            Some("Keeps installers from affecting each other, at the cost of disk space."),
        );
        let gamemode = widgets::dropdown(
            &games,
            "GameMode",
            Some(
                "CPU, GPU and I/O tuning while a game runs. Automatic: programs that use Direct3D, OpenGL or Vulkan.",
            ),
            &["Automatic", "Always", "Never"],
        );
        let dxvk = widgets::dropdown(
            &games,
            "Direct3D 9, 10 and 11 through Vulkan (DXVK)",
            None,
            &["Automatic", "Always", "Never"],
        );
        let vkd3d = widgets::dropdown(
            &games,
            "Direct3D 12 through Vulkan (VKD3D-Proton)",
            None,
            &["Automatic", "Always", "Never"],
        );
        let esync = widgets::switch(&games, "Esync", Some("Faster multi-threaded games."));
        let fsync = widgets::switch(
            &games,
            "Fsync",
            Some("Like esync, with less CPU use; needs a recent kernel."),
        );
        let debug = widgets::switch(
            &games,
            "Wine debug output",
            Some("For troubleshooting; slows games down."),
        );

        let overlay = widgets::section(
            &page,
            "Performance overlay",
            Some(
                "MangoHud's frame rate and load counters over games. Right Shift+F12 shows or hides it while playing.",
            ),
        );
        let hud = widgets::switch(&overlay, "Show the overlay in Windows games", None);
        hud.set_widget_name("hud");
        segrun_parts.push(hud.clone().upcast());
        let position = widgets::dropdown(
            &overlay,
            "Position",
            None,
            &[
                "Top left",
                "Top right",
                "Bottom left",
                "Bottom right",
                "Top centre",
            ],
        );
        let detail = widgets::dropdown(
            &overlay,
            "Detail",
            None,
            &["Frame rate only", "One line", "Standard", "Detailed"],
        );
        let fps_limit = widgets::spin(
            &overlay,
            "Frame rate limit",
            Some(
                "Frames per second for every game using MangoHud; 0 for no limit. Just under the screen's refresh rate gives the smoothest result with variable refresh rate.",
            ),
            (0.0, 1000.0),
            1.0,
        );
        fps_limit.set_widget_name("fps-limit");

        let scope = widgets::section(
            &page,
            "gamescope",
            Some(
                "Valve's gaming compositor around Windows games: resolution scaling, frame limiting and fullscreen for games that only run in a window.",
            ),
        );
        segrun_parts.push(scope.clone().upcast());
        let gamescope = widgets::switch(&scope, "Run Windows games in gamescope", None);
        gamescope.set_widget_name("gamescope");
        let fullscreen = widgets::switch(&scope, "Fullscreen", None);
        let borderless = widgets::switch(&scope, "Borderless window", None);
        let game_size = widgets::size(
            &scope,
            "Game resolution",
            Some("What the game renders; 0 × 0 for the window's size."),
        );
        let output_size = widgets::size(
            &scope,
            "Output resolution",
            Some(
                "What gamescope shows; 0 × 0 for automatic. Larger than the game resolution upscales.",
            ),
        );
        let frame_rate = widgets::spin(
            &scope,
            "Frame rate limit",
            Some("0 for none."),
            (0.0, 1000.0),
            1.0,
        );
        let filter_labels: Vec<&str> = g::FILTERS.iter().map(|(label, _)| *label).collect();
        let filter = widgets::dropdown(&scope, "Upscaling", None, &filter_labels);
        let sharpness = widgets::spin(
            &scope,
            "Upscaling sharpness",
            Some(
                "For FSR and NIS: 1 is the sharpest, 20 the softest; 0 keeps gamescope's default.",
            ),
            (0.0, 20.0),
            1.0,
        );
        let gs_adaptive = widgets::switch(&scope, "Variable refresh rate", None);
        let gs_hdr = widgets::switch(
            &scope,
            "HDR",
            Some("Needs an HDR display and a game that supports it."),
        );
        let extra = widgets::text_area(
            &scope,
            "More gamescope arguments",
            Some("One per line, such as --force-grab-cursor."),
        );

        let display = widgets::section(
            &page,
            "Display",
            Some("For every fullscreen game on the SegLinux desktop."),
        );
        let vrr = widgets::dropdown(
            &display,
            "Variable refresh rate (FreeSync, G-Sync)",
            None,
            &["Off", "Fullscreen games (recommended)", "Always"],
        );
        let tearing = widgets::dropdown(
            &display,
            "Allow tearing",
            Some("The lowest input latency, with visible tearing."),
            &[
                "Never",
                "When a game asks (recommended)",
                "Every fullscreen window",
            ],
        );

        let mode = widgets::section(
            &page,
            "Gaming Mode",
            Some(
                "Steam's Big Picture in gamescope, like a console. Choose \"SegLinux Gaming Mode\" on the sign-in screen.",
            ),
        );
        let mode_size = widgets::size(&mode, "Resolution", Some("0 × 0 for the display's own."));
        let mode_refresh = widgets::spin(
            &mode,
            "Refresh rate",
            Some("In Hz; 0 for the display's own."),
            (0.0, 1000.0),
            1.0,
        );
        let mode_adaptive = widgets::switch(&mode, "Variable refresh rate", None);
        let mode_hdr = widgets::switch(&mode, "HDR", None);
        let mode_overlay = widgets::switch(&mode, "Performance overlay", None);

        let vars = widgets::section(
            &page,
            "Environment variables",
            Some("For every Windows program segrun starts."),
        );
        segrun_parts.push(vars.clone().upcast());
        let env = widgets::text_area(
            &vars,
            "Variables",
            Some("One KEY=VALUE per line, such as PROTON_ENABLE_NVAPI=1 for DLSS on NVIDIA cards."),
        );

        let profiles = widgets::section(
            &page,
            "Game profiles",
            Some("Settings for one program, chosen by its file name. Saved with Apply."),
        );
        segrun_parts.push(profiles.clone().upcast());
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        profiles.append(&list);
        let toggles = ["Default", "Automatic", "Always", "Never"];
        let form = Rc::new(ProfileForm {
            pattern: widgets::entry(
                &profiles,
                "Program",
                Some("Its file name; * matches anything, as in *Launcher*.exe."),
                "Game.exe",
            ),
            runner: widgets::entry(
                &profiles,
                "Runner",
                Some("Empty for the general runner."),
                "Default",
            ),
            prefix: widgets::entry(
                &profiles,
                "Windows environment",
                Some("Empty for the general one."),
                "Default",
            ),
            gamemode: widgets::dropdown(&profiles, "GameMode", None, &toggles),
            dxvk: widgets::dropdown(&profiles, "DXVK", None, &toggles),
            vkd3d: widgets::dropdown(&profiles, "VKD3D-Proton", None, &toggles),
            gamescope: widgets::dropdown(&profiles, "gamescope", None, &["Default", "On", "Off"]),
            virtual_desktop: widgets::entry(
                &profiles,
                "Virtual desktop",
                Some("WIDTHxHEIGHT, such as 1920x1080, for programs that misbehave fullscreen."),
                "None",
            ),
            dlls: widgets::text_area(
                &profiles,
                "DLL overrides",
                Some("One per line, such as d3dcompiler_47=n,b."),
            ),
            env: widgets::text_area(&profiles, "Variables", Some("One KEY=VALUE per line.")),
            editing: RefCell::default(),
        });
        form.pattern.set_widget_name("profile-pattern");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.add_css_class("settings-row");
        let save_profile = gtk::Button::with_label("Add or update profile");
        save_profile.set_widget_name("save-profile");
        let clear = gtk::Button::with_label("New profile");
        let form_status = widgets::note("");
        form_status.set_hexpand(true);
        buttons.append(&form_status);
        buttons.append(&clear);
        buttons.append(&save_profile);
        profiles.append(&buttons);

        let apps: Rc<RefCell<Vec<AppOverride>>> = Rc::default();
        let slot: widgets::Redraw = Rc::default();
        let render_profiles: Rc<dyn Fn()> = Rc::new({
            let (list, apps, form, slot) =
                (list.downgrade(), apps.clone(), form.clone(), slot.clone());
            move || {
                let Some(list) = list.upgrade() else {
                    return;
                };
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                for app in apps.borrow().iter() {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                    row.add_css_class("settings-row");
                    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
                    text.set_hexpand(true);
                    let name = gtk::Label::new(Some(&app.pattern));
                    name.set_xalign(0.0);
                    text.append(&name);
                    let what = widgets::note(&g::summary(app));
                    what.add_css_class("caption");
                    text.append(&what);
                    row.append(&text);
                    let edit = gtk::Button::with_label("Edit");
                    let (form, chosen) = (form.clone(), app.clone());
                    edit.connect_clicked(move |_| form.show(&chosen));
                    row.append(&edit);
                    let remove = gtk::Button::with_label("Remove");
                    remove.update_property(&[gtk::accessible::Property::Label(&format!(
                        "Remove the profile of {}",
                        app.pattern
                    ))]);
                    let (apps, slot, pattern) = (apps.clone(), slot.clone(), app.pattern.clone());
                    remove.connect_clicked(move |_| {
                        g::remove(&mut apps.borrow_mut(), &pattern);
                        widgets::redraw(&slot);
                    });
                    row.append(&remove);
                    list.append(&row);
                }
                if apps.borrow().is_empty() {
                    list.append(&widgets::note("No profiles yet."));
                }
            }
        });
        *slot.borrow_mut() = Some(render_profiles.clone());
        {
            let (form, apps, render, status) = (
                form.clone(),
                apps.clone(),
                render_profiles.clone(),
                form_status.clone(),
            );
            save_profile.connect_clicked(move |_| match form.read() {
                Ok(app) => {
                    let pattern = app.pattern.clone();
                    g::upsert(&mut apps.borrow_mut(), app);
                    render();
                    form.show(&AppOverride::default());
                    status.set_text(&format!(
                        "Profile of {pattern} ready; select Apply to save it."
                    ));
                }
                Err(error) => status.set_text(&error),
            });
        }
        {
            let form = form.clone();
            clear.connect_clicked(move |_| form.show(&AppOverride::default()));
        }

        let tools = widgets::section(&page, "Installed gaming tools", None);
        for (command, what) in [
            ("wine", "Wine"),
            ("umu-run", "umu-launcher"),
            ("gamemoderun", "GameMode"),
            ("mangohud", "MangoHud"),
            ("gamescope", "gamescope"),
            ("steam", "Steam"),
            ("lutris", "Lutris"),
        ] {
            let found = common::installed(command);
            let state = gtk::Label::new(Some(if found { "Installed" } else { "Not installed" }));
            state.add_css_class(if found { "success" } else { "dim-label" });
            widgets::row(&tools, what, None, &state);
        }

        let this = Self {
            root,
            segrun_parts,
            runner,
            prefix,
            isolate,
            gamemode,
            dxvk,
            vkd3d,
            esync,
            fsync,
            debug,
            hud,
            position,
            detail,
            fps_limit,
            gamescope,
            fullscreen,
            borderless,
            game_size,
            output_size,
            frame_rate,
            filter,
            sharpness,
            gs_adaptive,
            gs_hdr,
            extra,
            vrr,
            tearing,
            mode_size,
            mode_refresh,
            mode_adaptive,
            mode_hdr,
            mode_overlay,
            env,
            apps,
            render_profiles,
        };
        if state.games.is_err() {
            for part in &this.segrun_parts {
                part.set_sensitive(false);
            }
        }
        this
    }
}

fn number_pair(args: &[String], w: Opt, h: Opt, spins: &(gtk::SpinButton, gtk::SpinButton)) {
    spins.0.set_value(g::number(args, w).into());
    spins.1.set_value(g::number(args, h).into());
}

impl Page for Gaming {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn load(&self, state: &State) {
        let shell = &state.shell;
        self.position
            .set_selected(index_of(&POSITIONS, &shell.overlay.position));
        self.detail
            .set_selected(index_of(&DETAILS, &shell.overlay.detail));
        self.fps_limit.set_value(shell.overlay.fps_limit.into());
        self.vrr
            .set_selected(index_of(&SYNC, &shell.windows.adaptive_sync));
        self.tearing
            .set_selected(index_of(&TEARING, &shell.windows.allow_tearing));
        let m = &shell.gaming_mode;
        self.mode_size.0.set_value(m.width.into());
        self.mode_size.1.set_value(m.height.into());
        self.mode_refresh.set_value(m.refresh_rate.into());
        self.mode_adaptive.set_active(m.adaptive_sync);
        self.mode_hdr.set_active(m.hdr);
        self.mode_overlay.set_active(m.performance_overlay);

        let Ok(games) = &state.games else {
            return;
        };
        let d = &games.defaults;
        self.runner.set_text(&d.runner);
        self.prefix.set_text(&d.prefix);
        self.isolate.set_active(d.isolate_installers);
        self.gamemode.set_selected(index_of(&TOGGLES, &d.gamemode));
        self.dxvk.set_selected(index_of(&TOGGLES, &d.dxvk));
        self.vkd3d.set_selected(index_of(&TOGGLES, &d.vkd3d));
        self.esync.set_active(d.esync);
        self.fsync.set_active(d.fsync);
        self.debug.set_active(d.debug);
        self.hud.set_active(d.hud);
        self.gamescope.set_active(d.gamescope);
        let args = &d.gamescope_args;
        self.fullscreen.set_active(g::has_flag(args, g::FULLSCREEN));
        self.borderless.set_active(g::has_flag(args, g::BORDERLESS));
        number_pair(args, g::GAME_WIDTH, g::GAME_HEIGHT, &self.game_size);
        number_pair(args, g::OUTPUT_WIDTH, g::OUTPUT_HEIGHT, &self.output_size);
        self.frame_rate
            .set_value(g::number(args, g::FRAME_RATE).into());
        let filter = g::value(args, g::FILTER).unwrap_or_default();
        let values: Vec<&str> = g::FILTERS.iter().map(|(_, v)| *v).collect();
        self.filter
            .set_selected(index_of(&values, &filter.as_str()));
        self.sharpness
            .set_value(g::number(args, g::SHARPNESS).into());
        self.gs_adaptive
            .set_active(g::has_flag(args, g::ADAPTIVE_SYNC));
        self.gs_hdr.set_active(g::has_flag(args, g::HDR));
        self.extra
            .buffer()
            .set_text(&g::extra_args(args).join("\n"));
        self.env.buffer().set_text(&g::format_env(&games.env));
        *self.apps.borrow_mut() = games.apps.clone();
        (self.render_profiles)();
    }

    fn store(&self, state: &mut State) -> Result<(), String> {
        let shell = &mut state.shell;
        shell.overlay.position = selected(&POSITIONS, &self.position);
        shell.overlay.detail = selected(&DETAILS, &self.detail);
        shell.overlay.fps_limit = widgets::spin_u32(&self.fps_limit);
        shell.windows.adaptive_sync = selected(&SYNC, &self.vrr);
        shell.windows.allow_tearing = selected(&TEARING, &self.tearing);
        let m = &mut shell.gaming_mode;
        m.width = widgets::spin_u32(&self.mode_size.0);
        m.height = widgets::spin_u32(&self.mode_size.1);
        m.refresh_rate = widgets::spin_u32(&self.mode_refresh);
        m.adaptive_sync = self.mode_adaptive.is_active();
        m.hdr = self.mode_hdr.is_active();
        m.performance_overlay = self.mode_overlay.is_active();

        let Ok(games) = &mut state.games else {
            return Ok(());
        };
        let runner = self.runner.text().trim().to_string();
        runner.parse::<segrun::runner::Runner>()?;
        let prefix = self.prefix.text().trim().to_string();
        segrun::prefix::validate_name(&prefix).map_err(|e| e.to_string())?;
        let d = &mut games.defaults;
        d.runner = runner;
        d.prefix = prefix;
        d.isolate_installers = self.isolate.is_active();
        d.gamemode = selected(&TOGGLES, &self.gamemode);
        d.dxvk = selected(&TOGGLES, &self.dxvk);
        d.vkd3d = selected(&TOGGLES, &self.vkd3d);
        d.esync = self.esync.is_active();
        d.fsync = self.fsync.is_active();
        d.debug = self.debug.is_active();
        d.hud = self.hud.is_active();
        d.gamescope = self.gamescope.is_active();
        let mut args: Vec<String> = widgets::text(&self.extra)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        if args.iter().any(|a| a.contains('\0')) {
            return Err("gamescope arguments cannot contain NUL characters.".into());
        }
        // The controls win over the same options typed as arguments.
        args = g::extra_args(&args);
        g::set_number(
            &mut args,
            g::GAME_WIDTH,
            widgets::spin_u32(&self.game_size.0),
        );
        g::set_number(
            &mut args,
            g::GAME_HEIGHT,
            widgets::spin_u32(&self.game_size.1),
        );
        g::set_number(
            &mut args,
            g::OUTPUT_WIDTH,
            widgets::spin_u32(&self.output_size.0),
        );
        g::set_number(
            &mut args,
            g::OUTPUT_HEIGHT,
            widgets::spin_u32(&self.output_size.1),
        );
        g::set_number(
            &mut args,
            g::FRAME_RATE,
            widgets::spin_u32(&self.frame_rate),
        );
        let (_, filter) = g::FILTERS[(self.filter.selected() as usize).min(g::FILTERS.len() - 1)];
        g::set_value(&mut args, g::FILTER, optional(filter));
        let sharp = widgets::spin_u32(&self.sharpness);
        g::set_value(
            &mut args,
            g::SHARPNESS,
            (!filter.is_empty() && sharp > 0).then(|| sharp.to_string()),
        );
        g::set_flag(&mut args, g::FULLSCREEN, self.fullscreen.is_active());
        g::set_flag(&mut args, g::BORDERLESS, self.borderless.is_active());
        g::set_flag(&mut args, g::ADAPTIVE_SYNC, self.gs_adaptive.is_active());
        g::set_flag(&mut args, g::HDR, self.gs_hdr.is_active());
        d.gamescope_args = args;
        games.env = g::parse_env(&widgets::text(&self.env))?;
        games.apps = self.apps.borrow().clone();
        Ok(())
    }
}
