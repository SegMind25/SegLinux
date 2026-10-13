//! Gaming settings: gamescope's options (edited without discarding the
//! user's other arguments), the MangoHud overlay's configuration and
//! segrun's per-game profiles.
use crate::config::{Overlay, OverlayDetail, OverlayPosition};
use crate::managed::{MARK, Wanted};
use segrun::config::{AppOverride, Toggle};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A gamescope option, by its short and long names.
#[derive(Clone, Copy, Debug)]
pub struct Opt {
    pub short: &'static str,
    pub long: &'static str,
}
pub const OUTPUT_WIDTH: Opt = Opt {
    short: "-W",
    long: "--output-width",
};
pub const OUTPUT_HEIGHT: Opt = Opt {
    short: "-H",
    long: "--output-height",
};
pub const GAME_WIDTH: Opt = Opt {
    short: "-w",
    long: "--nested-width",
};
pub const GAME_HEIGHT: Opt = Opt {
    short: "-h",
    long: "--nested-height",
};
pub const FRAME_RATE: Opt = Opt {
    short: "-r",
    long: "--nested-refresh",
};
pub const FILTER: Opt = Opt {
    short: "-F",
    long: "--filter",
};
pub const SHARPNESS: Opt = Opt {
    short: "--sharpness",
    long: "--fsr-sharpness",
};
pub const FULLSCREEN: Opt = Opt {
    short: "-f",
    long: "--fullscreen",
};
pub const BORDERLESS: Opt = Opt {
    short: "-b",
    long: "--borderless",
};
pub const ADAPTIVE_SYNC: Opt = Opt {
    short: "--adaptive-sync",
    long: "--adaptive-sync",
};
pub const HDR: Opt = Opt {
    short: "--hdr-enabled",
    long: "--hdr-enabled",
};

/// gamescope's upscaling filters, as offered in Settings: label and value.
pub const FILTERS: &[(&str, &str)] = &[
    ("Default (linear)", ""),
    ("AMD FidelityFX Super Resolution (FSR)", "fsr"),
    ("NVIDIA Image Scaling (NIS)", "nis"),
    ("Sharp pixels (integer scaling)", "pixel"),
    ("Nearest neighbour", "nearest"),
];

impl Opt {
    fn names(&self, arg: &str) -> bool {
        arg == self.short || arg == self.long
    }
    fn inline<'a>(&self, arg: &'a str) -> Option<&'a str> {
        arg.strip_prefix(self.long)?.strip_prefix('=')
    }
}

/// The value of an option: `-W 1920`, `--output-width 1920` or
/// `--output-width=1920`.
pub fn value(args: &[String], opt: Opt) -> Option<String> {
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if let Some(v) = opt.inline(arg) {
            return Some(v.to_string());
        }
        if opt.names(arg) {
            return iter
                .peek()
                .filter(|v| !v.starts_with('-'))
                .map(|v| v.to_string());
        }
    }
    None
}

/// A numeric option, 0 when absent or not a number.
pub fn number(args: &[String], opt: Opt) -> u32 {
    value(args, opt).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Sets an option's value (or removes it with `None`), keeping the others.
pub fn set_value(args: &mut Vec<String>, opt: Opt, new: Option<String>) {
    let mut output = Vec::new();
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if opt.inline(arg).is_some() {
            continue;
        }
        if opt.names(arg) {
            if iter.peek().is_some_and(|next| !next.starts_with('-')) {
                iter.next();
            }
        } else {
            output.push(arg.clone());
        }
    }
    if let Some(new) = new {
        output.extend([opt.short.to_string(), new]);
    }
    *args = output;
}

/// Sets a numeric option; 0 removes it.
pub fn set_number(args: &mut Vec<String>, opt: Opt, value: u32) {
    set_value(args, opt, (value > 0).then(|| value.to_string()));
}

pub fn has_flag(args: &[String], opt: Opt) -> bool {
    args.iter().any(|a| opt.names(a))
}

pub fn set_flag(args: &mut Vec<String>, opt: Opt, on: bool) {
    args.retain(|a| !opt.names(a));
    if on {
        args.push(opt.short.to_string());
    }
}

/// The options Settings shows as controls of their own.
pub const VALUE_OPTS: [Opt; 7] = [
    OUTPUT_WIDTH,
    OUTPUT_HEIGHT,
    GAME_WIDTH,
    GAME_HEIGHT,
    FRAME_RATE,
    FILTER,
    SHARPNESS,
];
pub const FLAG_OPTS: [Opt; 4] = [FULLSCREEN, BORDERLESS, ADAPTIVE_SYNC, HDR];

/// The arguments that have no control of their own in Settings.
pub fn extra_args(args: &[String]) -> Vec<String> {
    let mut rest = args.to_vec();
    for opt in VALUE_OPTS {
        set_value(&mut rest, opt, None);
    }
    for opt in FLAG_OPTS {
        set_flag(&mut rest, opt, false);
    }
    rest
}

/// Proton builds installed by Steam or by hand (GE-Proton and others in
/// `compatibilitytools.d`): name and folder, by name.
pub fn proton_builds() -> Vec<(String, PathBuf)> {
    let home = seglinux_core::xdg::data_home();
    let dot_steam = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".steam");
    proton_builds_in(&[
        home.join("Steam/steamapps/common"),
        home.join("Steam/compatibilitytools.d"),
        dot_steam.join("root/compatibilitytools.d"),
        dot_steam.join("steam/steamapps/common"),
    ])
}

pub fn proton_builds_in(roots: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.join("proton").is_file() {
                continue;
            }
            // The same build is often reachable through ~/.steam links.
            let real = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            if found
                .iter()
                .any(|(_, d)| d.canonicalize().unwrap_or_else(|_| d.clone()) == real)
            {
                continue;
            }
            found.push((entry.file_name().to_string_lossy().into_owned(), dir));
        }
    }
    found.sort_by_key(|(name, _)| name.to_lowercase());
    found
}

/// `~/.config/MangoHud/MangoHud.conf`
pub fn mangohud_path() -> PathBuf {
    seglinux_core::xdg::config_home().join("MangoHud/MangoHud.conf")
}

/// MangoHud's configuration for the overlay settings; absent for the
/// defaults, so MangoHud keeps its own.
pub fn mangohud_conf(overlay: &Overlay) -> Wanted {
    if *overlay == Overlay::default() {
        return Wanted::Absent;
    }
    let mut lines = vec![
        format!("# {MARK}: changes made here are replaced when settings are saved."),
        "# Remove these lines to keep your own MangoHud configuration.".to_string(),
    ];
    // MangoHud's presets: 1 the frame rate, 2 one horizontal line,
    // 4 detailed; without one, its usual overlay.
    match overlay.detail {
        OverlayDetail::FpsOnly => lines.push("preset=1".into()),
        OverlayDetail::Horizontal => lines.push("preset=2".into()),
        OverlayDetail::Standard => {}
        OverlayDetail::Detailed => lines.push("preset=4".into()),
    }
    let position = match overlay.position {
        OverlayPosition::TopLeft => "top-left",
        OverlayPosition::TopRight => "top-right",
        OverlayPosition::BottomLeft => "bottom-left",
        OverlayPosition::BottomRight => "bottom-right",
        OverlayPosition::TopCenter => "top-center",
    };
    lines.push(format!("position={position}"));
    if overlay.fps_limit > 0 {
        lines.push(format!("fps_limit={}", overlay.fps_limit));
    }
    lines.push("toggle_hud=Shift_R+F12".into());
    Wanted::Text(lines.join("\n") + "\n")
}

/// Environment variables from `KEY=VALUE` lines; blank lines and `#`
/// comments are skipped.
pub fn parse_env(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("`{line}` is not KEY=VALUE."));
        };
        let key = key.trim();
        let valid = key
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
        if !valid || value.contains('\0') {
            return Err(format!(
                "`{key}` is not a variable name: use letters, digits and underscores."
            ));
        }
        env.insert(key.to_string(), value.trim().to_string());
    }
    Ok(env)
}

pub fn format_env(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect::<String>()
}

/// Checks a per-game profile before it is saved.
pub fn validate_profile(app: &AppOverride) -> Result<(), String> {
    let pattern = app.pattern.trim();
    if pattern.is_empty() || pattern.len() > 128 || pattern.contains('/') || pattern.contains('\0')
    {
        return Err("Enter the program's file name, such as Game.exe or *Launcher*.exe.".into());
    }
    if let Some(runner) = &app.runner {
        runner.parse::<segrun::runner::Runner>()?;
    }
    if let Some(prefix) = &app.prefix {
        segrun::prefix::validate_name(prefix).map_err(|e| e.to_string())?;
    }
    if let Some(size) = &app.virtual_desktop {
        segrun::launch::validate_resolution(size).map_err(|e| e.to_string())?;
    }
    if app
        .dll_overrides
        .iter()
        .any(|o| !o.contains('=') || o.contains(';') || o.chars().any(char::is_whitespace))
    {
        return Err("DLL overrides look like d3dcompiler_47=n,b (one per line).".into());
    }
    if app
        .gamescope_args
        .iter()
        .chain(&app.args)
        .any(|a| a.contains('\0'))
    {
        return Err("Arguments cannot contain NUL characters.".into());
    }
    Ok(())
}

/// Adds the profile, or replaces the one for the same program.
pub fn upsert(apps: &mut Vec<AppOverride>, app: AppOverride) {
    match apps
        .iter_mut()
        .find(|a| a.pattern.eq_ignore_ascii_case(&app.pattern))
    {
        Some(existing) => *existing = app,
        None => apps.push(app),
    }
}

pub fn remove(apps: &mut Vec<AppOverride>, pattern: &str) {
    apps.retain(|a| !a.pattern.eq_ignore_ascii_case(pattern));
}

fn toggle_word(t: Toggle) -> &'static str {
    match t {
        Toggle::Auto => "auto",
        Toggle::Always => "on",
        Toggle::Never => "off",
    }
}

/// One line describing what a profile changes.
pub fn summary(app: &AppOverride) -> String {
    let mut parts = Vec::new();
    if let Some(r) = &app.runner {
        parts.push(format!("runner {r}"));
    }
    if let Some(p) = &app.prefix {
        parts.push(format!("environment {p}"));
    }
    for (name, value) in [
        ("GameMode", app.gamemode),
        ("DXVK", app.dxvk),
        ("VKD3D", app.vkd3d),
    ] {
        if let Some(v) = value {
            parts.push(format!("{name} {}", toggle_word(v)));
        }
    }
    match app.gamescope {
        Some(true) => parts.push("gamescope".into()),
        Some(false) => parts.push("no gamescope".into()),
        None => {}
    }
    if let Some(v) = &app.virtual_desktop {
        parts.push(format!("virtual desktop {v}"));
    }
    if !app.env.is_empty() {
        parts.push(format!("{} variables", app.env.len()));
    }
    if parts.is_empty() {
        "the defaults".into()
    } else {
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn edits_frame_rate_without_discarding_custom_options() {
        let mut a = args(&["-f", "-r", "60", "--adaptive-sync", "-W", "1920"]);
        assert_eq!(number(&a, FRAME_RATE), 60);
        set_number(&mut a, FRAME_RATE, 144);
        assert_eq!(
            a,
            args(&["-f", "--adaptive-sync", "-W", "1920", "-r", "144"])
        );
        set_number(&mut a, FRAME_RATE, 0);
        assert_eq!(number(&a, FRAME_RATE), 0);
        assert!(a.contains(&"--adaptive-sync".into()));
    }

    #[test]
    fn missing_value_does_not_swallow_next_option() {
        let mut a = args(&["-r", "--adaptive-sync"]);
        set_number(&mut a, FRAME_RATE, 60);
        assert_eq!(a, args(&["--adaptive-sync", "-r", "60"]));
    }

    #[test]
    fn long_names_and_inline_values() {
        let mut a = args(&["--output-width", "2560", "--filter=fsr", "--fullscreen"]);
        assert_eq!(number(&a, OUTPUT_WIDTH), 2560);
        assert_eq!(value(&a, FILTER).as_deref(), Some("fsr"));
        assert!(has_flag(&a, FULLSCREEN));
        set_value(&mut a, FILTER, Some("nis".into()));
        set_flag(&mut a, FULLSCREEN, false);
        set_flag(&mut a, BORDERLESS, true);
        assert_eq!(a, args(&["--output-width", "2560", "-F", "nis", "-b"]));
        set_value(&mut a, FILTER, None);
        assert_eq!(value(&a, FILTER), None);
        // -h (game height) is not -H (output height).
        let b = args(&["-h", "720", "-H", "1440"]);
        assert_eq!(
            (number(&b, GAME_HEIGHT), number(&b, OUTPUT_HEIGHT)),
            (720, 1440)
        );
    }

    #[test]
    fn extra_arguments_leave_out_the_controlled_ones() {
        let a = args(&[
            "-f",
            "--expose-wayland",
            "-W",
            "1920",
            "--filter=fsr",
            "--force-grab-cursor",
            "-b",
        ]);
        assert_eq!(
            extra_args(&a),
            args(&["--expose-wayland", "--force-grab-cursor"])
        );
    }

    #[test]
    fn finds_proton_builds_once() {
        let temp = tempfile::tempdir().unwrap();
        let common = temp.path().join("common");
        let tools = temp.path().join("compatibilitytools.d");
        for dir in [
            common.join("Proton 9.0"),
            tools.join("GE-Proton10-4"),
            common.join("Not Proton"),
        ] {
            std::fs::create_dir_all(&dir).unwrap();
        }
        std::fs::write(common.join("Proton 9.0/proton"), "").unwrap();
        std::fs::write(tools.join("GE-Proton10-4/proton"), "").unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&common, &link).unwrap();
        let names: Vec<String> =
            proton_builds_in(&[common, tools, link, temp.path().join("missing")])
                .into_iter()
                .map(|(name, _)| name)
                .collect();
        assert_eq!(names, ["GE-Proton10-4", "Proton 9.0"]);
    }

    #[test]
    fn mangohud_configuration() {
        assert_eq!(mangohud_conf(&Overlay::default()), Wanted::Absent);
        let Wanted::Text(text) = mangohud_conf(&Overlay {
            position: OverlayPosition::TopRight,
            detail: OverlayDetail::FpsOnly,
            fps_limit: 141,
        }) else {
            panic!("expected a file");
        };
        assert!(text.lines().next().unwrap().contains(MARK));
        for line in ["preset=1", "position=top-right", "fps_limit=141"] {
            assert!(text.lines().any(|l| l == line), "{line}");
        }
        let Wanted::Text(text) = mangohud_conf(&Overlay {
            fps_limit: 60,
            ..Overlay::default()
        }) else {
            panic!("expected a file");
        };
        assert!(!text.contains("preset="));
    }

    #[test]
    fn environment_lines() {
        let env = parse_env("# NVIDIA\nPROTON_ENABLE_NVAPI=1\n\n DXVK_HUD = fps \n").unwrap();
        assert_eq!(env["PROTON_ENABLE_NVAPI"], "1");
        assert_eq!(env["DXVK_HUD"], "fps");
        assert_eq!(parse_env(&format_env(&env)).unwrap(), env);
        assert!(parse_env("NOEQUALS").is_err());
        assert!(parse_env("1BAD=x").is_err());
        assert!(parse_env("BAD-NAME=x").is_err());
    }

    #[test]
    fn profiles_are_validated_replaced_and_removed() {
        let mut apps = Vec::new();
        let game = AppOverride {
            pattern: "Game.exe".into(),
            runner: Some("umu".into()),
            gamescope: Some(true),
            ..AppOverride::default()
        };
        assert!(validate_profile(&game).is_ok());
        upsert(&mut apps, game.clone());
        upsert(
            &mut apps,
            AppOverride {
                pattern: "game.EXE".into(),
                prefix: Some("games".into()),
                ..AppOverride::default()
            },
        );
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].prefix.as_deref(), Some("games"));
        assert_eq!(summary(&apps[0]), "environment games");
        assert_eq!(summary(&game), "runner umu · gamescope");
        remove(&mut apps, "GAME.exe");
        assert!(apps.is_empty());
        for bad in [
            AppOverride {
                pattern: " ".into(),
                ..AppOverride::default()
            },
            AppOverride {
                pattern: "a/b.exe".into(),
                ..AppOverride::default()
            },
            AppOverride {
                pattern: "a.exe".into(),
                runner: Some("steam".into()),
                ..AppOverride::default()
            },
            AppOverride {
                pattern: "a.exe".into(),
                prefix: Some("../x".into()),
                ..AppOverride::default()
            },
            AppOverride {
                pattern: "a.exe".into(),
                virtual_desktop: Some("big".into()),
                ..AppOverride::default()
            },
            AppOverride {
                pattern: "a.exe".into(),
                dll_overrides: vec!["d3d9".into()],
                ..AppOverride::default()
            },
        ] {
            assert!(validate_profile(&bad).is_err(), "{bad:?}");
        }
    }
}
