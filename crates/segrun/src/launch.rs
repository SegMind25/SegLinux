//! Turn "run this file" into concrete commands.
//!
//! Planning is a pure function of the file, the settings and a few facts
//! about the machine, so `segrun run --dry-run` shows exactly what would
//! happen and the logic is unit tested without Wine installed.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use seglinux_core::cmd::Cmd;
use seglinux_pe::FileKind;

use crate::advice::{self, Level};
use crate::config::{Config, Toggle};
use crate::prefix::{self, PrefixState};
use crate::runner::{Runner, path_str};

/// Options given on the command line; `None` means "use the config".
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub prefix: Option<String>,
    pub isolate: bool,
    pub runner: Option<String>,
    pub dxvk: Option<Toggle>,
    pub vkd3d: Option<Toggle>,
    pub silent: bool,
    pub virtual_desktop: Option<String>,
    pub hud: bool,
    pub debug: bool,
    pub gamemode: Option<Toggle>,
    pub gamescope: bool,
}

/// Effective settings after merging defaults, the per-app override and the
/// command line (in that order of increasing priority).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub runner: Runner,
    pub prefix: String,
    pub dxvk: Toggle,
    pub vkd3d: Toggle,
    pub esync: bool,
    pub fsync: bool,
    pub debug: bool,
    pub hud: bool,
    pub gamemode: Toggle,
    pub gamescope: bool,
    pub gamescope_args: Vec<String>,
    pub silent: bool,
    pub virtual_desktop: Option<String>,
    pub extra_args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub dll_overrides: Vec<String>,
}

impl Settings {
    pub fn resolve(
        config: &Config,
        file: &Path,
        kind: &FileKind,
        opts: &RunOptions,
    ) -> Result<Settings> {
        let d = &config.defaults;
        let file_name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let app = config.app_for(&file_name);

        let is_installer = match kind {
            FileKind::Pe(info) => info.installer.is_some(),
            FileKind::Msi => true,
            _ => false,
        };
        let prefix = if let Some(p) = &opts.prefix {
            p.clone()
        } else if let Some(p) = app.and_then(|a| a.prefix.clone()) {
            p
        } else if opts.isolate || (is_installer && d.isolate_installers) {
            prefix::slug_for(file)
        } else {
            d.prefix.clone()
        };
        prefix::validate_name(&prefix)?;

        let runner_spec = opts
            .runner
            .clone()
            .or_else(|| app.and_then(|a| a.runner.clone()))
            .unwrap_or_else(|| d.runner.clone());
        let runner: Runner = runner_spec.parse().map_err(anyhow::Error::msg)?;

        let mut env: Vec<(String, String)> = config
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Some(a) = app {
            env.extend(a.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        }

        Ok(Settings {
            runner,
            prefix,
            dxvk: opts.dxvk.or(app.and_then(|a| a.dxvk)).unwrap_or(d.dxvk),
            vkd3d: opts.vkd3d.or(app.and_then(|a| a.vkd3d)).unwrap_or(d.vkd3d),
            esync: d.esync,
            fsync: d.fsync,
            debug: opts.debug || d.debug,
            hud: opts.hud || d.hud,
            gamemode: opts
                .gamemode
                .or(app.and_then(|a| a.gamemode))
                .unwrap_or(d.gamemode),
            gamescope: opts.gamescope || app.and_then(|a| a.gamescope).unwrap_or(d.gamescope),
            gamescope_args: app
                .map(|a| a.gamescope_args.clone())
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| d.gamescope_args.clone()),
            silent: opts.silent,
            virtual_desktop: opts
                .virtual_desktop
                .clone()
                .or_else(|| app.and_then(|a| a.virtual_desktop.clone())),
            extra_args: app.map(|a| a.args.clone()).unwrap_or_default(),
            env,
            dll_overrides: app.map(|a| a.dll_overrides.clone()).unwrap_or_default(),
        })
    }
}

/// Facts about the machine that influence the plan.
#[derive(Debug, Clone, Copy)]
pub struct Host {
    pub vulkan: bool,
    /// Highest Vulkan version a real GPU offers (from `vulkaninfo`);
    /// `Some((0, 0))` when only the software rasteriser is available, `None`
    /// when it could not be determined.
    pub vulkan_api: Option<(u32, u32)>,
    pub winetricks: bool,
    /// `gamemoderun` from Feral GameMode.
    pub gamemode: bool,
    pub mangohud: bool,
    pub gamescope: bool,
}

impl Host {
    pub fn detect() -> Host {
        let has = |p: &str| seglinux_core::which(p).is_some();
        let vulkan = has_vulkan_icd();
        Host {
            vulkan,
            vulkan_api: if vulkan { detect_vulkan_api() } else { None },
            winetricks: has("winetricks"),
            gamemode: has("gamemoderun"),
            mangohud: has("mangohud"),
            gamescope: has("gamescope"),
        }
    }
}

/// Ask `vulkaninfo` (vulkan-tools) which Vulkan version the GPUs support.
pub fn detect_vulkan_api() -> Option<(u32, u32)> {
    let out = Cmd::new("vulkaninfo").arg("--summary").output().ok()?;
    parse_vulkan_summary(&out)
}

/// Highest `apiVersion` among the hardware devices in `vulkaninfo
/// --summary` output. Software renderers (llvmpipe, device type CPU) do
/// not count: DXVK on them is unusably slow.
pub fn parse_vulkan_summary(text: &str) -> Option<(u32, u32)> {
    // (apiVersion, is a CPU device) per GPU block.
    let mut devices: Vec<(Option<(u32, u32)>, bool)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("GPU") && line.ends_with(':') {
            devices.push((None, false));
        } else if let Some(device) = devices.last_mut() {
            if let Some(value) = line.strip_prefix("apiVersion") {
                device.0 = parse_version(value);
            } else if let Some(value) = line.strip_prefix("deviceType") {
                device.1 = value.contains("CPU");
            }
        }
    }
    if devices.is_empty() {
        return None;
    }
    Some(
        devices
            .iter()
            .filter(|(_, cpu)| !cpu)
            .filter_map(|(api, _)| *api)
            .max()
            .unwrap_or((0, 0)),
    )
}

/// `= 1.3.289` or `= 4206847 (1.3.255)` -> (1, 3).
fn parse_version(value: &str) -> Option<(u32, u32)> {
    value
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|token| token.contains('.'))
        .find_map(|token| {
            let mut parts = token.split('.');
            Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
        })
}

pub fn has_vulkan_icd() -> bool {
    ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d"]
        .iter()
        .any(|dir| {
            std::fs::read_dir(dir).is_ok_and(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| e.path().extension().is_some_and(|x| x == "json"))
            })
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// Create the prefix. Failing here aborts the launch.
    InitPrefix,
    /// Optional graphics layers. Failing here (e.g. offline) only means the
    /// program starts with Wine's own Direct3D implementation.
    Dxvk,
    Vkd3d,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub kind: StepKind,
    pub what: String,
    pub cmd: Cmd,
}

#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub prefix_dir: PathBuf,
    /// One-time preparation of the prefix.
    pub setup: Vec<Step>,
    pub launch: Cmd,
    pub notes: Vec<String>,
    /// Prefix state to record once setup succeeded.
    pub state: PrefixState,
}

pub fn plan(
    file: &Path,
    kind: &FileKind,
    args: &[String],
    s: &Settings,
    prefix_dir: &Path,
    state: Option<PrefixState>,
    host: Host,
) -> Result<LaunchPlan> {
    let mut notes = Vec::new();
    let (inner, cwd) = inner_command(file, kind, args, s, &mut notes)?;
    let wineprefix = s.runner.wineprefix(prefix_dir);
    let mut state = state.unwrap_or_else(|| PrefixState {
        created_by: format!("segrun {}", env!("CARGO_PKG_VERSION")),
        runner: s.runner.to_string(),
        ..Default::default()
    });

    let mut setup = Vec::new();
    if matches!(s.runner, Runner::Wine { .. }) && !prefix::is_initialized(&wineprefix) {
        setup.push(Step {
            kind: StepKind::InitPrefix,
            what: format!("Creating the Windows environment `{}`", s.prefix),
            cmd: s
                .runner
                .command(prefix_dir, &["wineboot".into(), "--init".into()])
                .env("WINEDEBUG", "-all"),
        });
    }

    let pe = match kind {
        FileKind::Pe(info) => Some(info.as_ref()),
        _ => None,
    };
    let wants = |toggle: Toggle, detect: fn(&seglinux_pe::PeInfo) -> bool| match toggle {
        Toggle::Always => true,
        Toggle::Never => false,
        Toggle::Auto => pe.is_some_and(detect),
    };
    let want_dxvk = wants(s.dxvk, advice::uses_d3d_dxvk);
    let want_vkd3d = wants(s.vkd3d, advice::uses_d3d12);

    let mut dll_overrides = s.dll_overrides.clone();
    if !s.runner.bundles_dxvk() {
        let layers = [
            (StepKind::Dxvk, want_dxvk, state.dxvk, "dxvk", "DXVK"),
            (
                StepKind::Vkd3d,
                want_vkd3d,
                state.vkd3d,
                "vkd3d",
                "VKD3D-Proton",
            ),
        ];
        // DXVK 2 and VKD3D-Proton need Vulkan 1.3. Older GPUs (for example
        // Intel HD Graphics before Skylake) still run DXVK 1.10.3, which
        // needs Vulkan 1.1.
        let api = host.vulkan_api;
        let usable = host.vulkan && !matches!(api, Some(v) if v < (1, 1));
        let legacy = matches!(api, Some(v) if v < (1, 3));
        for (kind, wanted, installed, verb, label) in layers {
            if !wanted || installed {
                continue;
            }
            let (verb, label) = match kind {
                StepKind::Dxvk if legacy => ("dxvk1103", "DXVK 1.10.3"),
                _ => (verb, label),
            };
            if !usable {
                notes.push(format!(
                    "No Vulkan 1.1 GPU driver found: {label} skipped, Wine falls back to OpenGL (WineD3D)."
                ));
            } else if kind == StepKind::Vkd3d && legacy {
                notes.push(
                    "This GPU has no Vulkan 1.3 driver: Direct3D 12 (VKD3D-Proton) cannot run on it."
                        .into(),
                );
            } else if !host.winetricks {
                notes.push(format!("winetricks is not installed: {label} skipped."));
            } else {
                setup.push(Step {
                    kind,
                    what: format!("Installing {label} into `{}`", s.prefix),
                    cmd: winetricks(&s.runner, prefix_dir, &[verb]),
                });
                state.mark(kind, true);
            }
        }
        if s.dxvk == Toggle::Never && state.dxvk {
            // DXVK's DLLs are already in the prefix: prefer Wine's builtins.
            dll_overrides.push("d3d9,d3d10core,d3d11,dxgi=b".into());
        }
    }

    let mut launch = s.runner.command(prefix_dir, &inner);
    if let Some(dir) = cwd {
        launch = launch.cwd(dir);
    }
    if !s.debug {
        launch = launch.env("WINEDEBUG", "-all");
    }
    match s.runner {
        Runner::Wine { .. } => {
            if s.esync {
                launch = launch.env("WINEESYNC", "1");
            }
            if s.fsync {
                launch = launch.env("WINEFSYNC", "1");
            }
        }
        _ => {
            if !s.esync {
                launch = launch.env("PROTON_NO_ESYNC", "1");
            }
            if !s.fsync {
                launch = launch.env("PROTON_NO_FSYNC", "1");
            }
        }
    }
    if !dll_overrides.is_empty() {
        launch = launch.env("WINEDLLOVERRIDES", dll_overrides.join(";"));
    }
    for (k, v) in &s.env {
        launch = launch.env(k.clone(), v.clone());
    }

    // Gaming wrappers, outermost first:
    //   gamemoderun gamescope <args> -- mangohud <runner> <program>
    let mut wrappers: Vec<String> = Vec::new();
    let game = pe.is_some_and(advice::is_game);
    let gamemode = match s.gamemode {
        Toggle::Always => true,
        Toggle::Never => false,
        Toggle::Auto => game,
    };
    if gamemode {
        if host.gamemode {
            wrappers.push("gamemoderun".into());
        } else if s.gamemode == Toggle::Always {
            notes.push("GameMode is not installed: sudo pacman -S gamemode lib32-gamemode".into());
        }
    }
    if s.gamescope {
        if host.gamescope {
            wrappers.push("gamescope".into());
            wrappers.extend(s.gamescope_args.iter().cloned());
            wrappers.push("--".into());
        } else {
            notes.push("gamescope is not installed: sudo pacman -S gamescope".into());
        }
    }
    if s.hud {
        if host.mangohud {
            wrappers.push("mangohud".into());
        } else {
            launch = launch.env("DXVK_HUD", "1");
        }
    }
    let launch = wrap(launch, wrappers);

    Ok(LaunchPlan {
        prefix_dir: prefix_dir.to_path_buf(),
        setup,
        launch,
        notes,
        state,
    })
}

/// Prefix a command with wrapper programs, keeping its environment and
/// working directory.
fn wrap(mut cmd: Cmd, wrappers: Vec<String>) -> Cmd {
    let mut parts = wrappers.into_iter();
    let Some(first) = parts.next() else {
        return cmd;
    };
    let mut args: Vec<String> = parts.collect();
    args.push(std::mem::replace(&mut cmd.program, first));
    args.append(&mut cmd.args);
    cmd.args = args;
    cmd
}

pub fn winetricks(runner: &Runner, prefix_dir: &Path, verbs: &[&str]) -> Cmd {
    let mut cmd = Cmd::new("winetricks")
        .arg("-q")
        .args(verbs.iter().copied())
        .env("WINEPREFIX", path_str(&runner.wineprefix(prefix_dir)))
        .env("WINEDEBUG", "-all");
    if let Some(bin) = runner.wine_binary().filter(|b| *b != "wine") {
        cmd = cmd.env("WINE", bin);
    }
    cmd
}

/// The Windows-side command line and working directory.
fn inner_command(
    file: &Path,
    kind: &FileKind,
    args: &[String],
    s: &Settings,
    notes: &mut Vec<String>,
) -> Result<(Vec<String>, Option<PathBuf>)> {
    let abs = path_str(file);
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| abs.clone());
    // Windows programs expect to start in their own folder.
    let cwd = file.parent().map(Path::to_path_buf);

    let mut inner: Vec<String> = match kind {
        FileKind::Pe(info) if info.is_launchable() => vec![abs],
        FileKind::Msi => vec!["msiexec".into(), "/i".into(), name],
        FileKind::Batch => vec!["cmd".into(), "/c".into(), name],
        FileKind::Shortcut => vec!["start".into(), "/unix".into(), abs],
        other => {
            let reason = advice::advise(other)
                .into_iter()
                .find(|a| a.level == Level::Error || a.level == Level::Warn)
                .map(|a| a.text)
                .unwrap_or_else(|| other.describe());
            bail!("cannot run {}: {reason}", file.display());
        }
    };

    if s.silent {
        match kind {
            FileKind::Pe(info) => match info.installer {
                Some(k) => inner.extend(k.silent_args().iter().map(|a| a.to_string())),
                None => notes.push("--silent ignored: not a recognised installer.".into()),
            },
            FileKind::Msi => inner.push("/qn".into()),
            _ => notes.push("--silent only applies to installers.".into()),
        }
    }
    inner.extend(s.extra_args.iter().cloned());
    inner.extend(args.iter().cloned());

    if let Some(size) = &s.virtual_desktop {
        validate_resolution(size)?;
        inner.splice(
            0..0,
            ["explorer".to_string(), format!("/desktop=segrun,{size}")],
        );
    }
    Ok((inner, cwd))
}

fn validate_resolution(size: &str) -> Result<()> {
    let ok = size.split_once('x').is_some_and(|(w, h)| {
        w.parse::<u16>().is_ok_and(|w| w > 0) && h.parse::<u16>().is_ok_and(|h| h > 0)
    });
    if !ok {
        bail!("invalid virtual desktop size `{size}`, expected WIDTHxHEIGHT such as 1920x1080");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use seglinux_pe::{InstallerKind, Machine, PeFormat, PeInfo, Subsystem};

    fn exe(imports: &[&str], installer: Option<InstallerKind>) -> FileKind {
        FileKind::Pe(Box::new(PeInfo {
            machine: Machine::Amd64,
            format: PeFormat::Pe32Plus,
            subsystem: Subsystem::WindowsGui,
            is_dll: false,
            large_address_aware: true,
            timestamp: 0,
            dotnet: false,
            imports: imports.iter().map(|s| s.to_string()).collect(),
            sections: vec![],
            overlay_size: 0,
            installer,
        }))
    }

    const HOST: Host = Host {
        vulkan: true,
        vulkan_api: Some((1, 3)),
        winetricks: true,
        gamemode: true,
        mangohud: true,
        gamescope: true,
    };

    fn settings(config: &Config, file: &str, kind: &FileKind, opts: RunOptions) -> Settings {
        Settings::resolve(config, Path::new(file), kind, &opts).unwrap()
    }

    fn plan_for(
        kind: &FileKind,
        file: &str,
        opts: RunOptions,
        state: Option<PrefixState>,
        host: Host,
    ) -> LaunchPlan {
        let s = settings(&Config::default(), file, kind, opts);
        plan(
            Path::new(file),
            kind,
            &[],
            &s,
            Path::new("/pfx/default"),
            state,
            host,
        )
        .unwrap()
    }

    #[test]
    fn new_prefix_is_initialised_and_dxvk_installed_for_d3d11() {
        let kind = exe(&["d3d11.dll"], None);
        let p = plan_for(
            &kind,
            "/games/Game/game.exe",
            RunOptions::default(),
            None,
            HOST,
        );
        let steps: Vec<String> = p.setup.iter().map(|s| s.cmd.to_string()).collect();
        assert_eq!(
            steps,
            vec![
                "WINEPREFIX=/pfx/default WINEDEBUG=-all wine wineboot --init",
                "WINEPREFIX=/pfx/default WINEDEBUG=-all winetricks -q dxvk",
            ]
        );
        assert!(p.state.dxvk);
        // A Direct3D program is a game: it runs under GameMode.
        assert_eq!(p.launch.program, "gamemoderun");
        assert_eq!(p.launch.args, vec!["wine", "/games/Game/game.exe"]);
        assert_eq!(p.launch.cwd.as_deref(), Some(Path::new("/games/Game")));
        assert_eq!(p.launch.get_env("WINEDEBUG"), Some("-all"));
        assert_eq!(p.launch.get_env("WINEESYNC"), Some("1"));
    }

    #[test]
    fn dxvk_not_reinstalled_and_skipped_without_vulkan() {
        let kind = exe(&["d3d9.dll"], None);
        let installed = PrefixState {
            dxvk: true,
            ..Default::default()
        };
        let p = plan_for(
            &kind,
            "/g/a.exe",
            RunOptions::default(),
            Some(installed),
            HOST,
        );
        assert!(p.setup.iter().all(|s| !s.what.contains("DXVK")));

        let no_vulkan = Host {
            vulkan: false,
            ..HOST
        };
        let p = plan_for(&kind, "/g/a.exe", RunOptions::default(), None, no_vulkan);
        assert!(p.setup.iter().all(|s| !s.what.contains("DXVK")));
        assert!(p.notes.iter().any(|n| n.contains("No Vulkan")));
        assert!(!p.state.dxvk);
    }

    #[test]
    fn dxvk_never_falls_back_to_builtin_dlls() {
        let kind = exe(&["d3d11.dll"], None);
        let installed = PrefixState {
            dxvk: true,
            ..Default::default()
        };
        let opts = RunOptions {
            dxvk: Some(Toggle::Never),
            ..Default::default()
        };
        let p = plan_for(&kind, "/g/a.exe", opts, Some(installed), HOST);
        assert_eq!(
            p.launch.get_env("WINEDLLOVERRIDES"),
            Some("d3d9,d3d10core,d3d11,dxgi=b")
        );
    }

    #[test]
    fn vkd3d_for_d3d12() {
        let kind = exe(&["d3d12.dll", "dxgi.dll"], None);
        let p = plan_for(&kind, "/g/a.exe", RunOptions::default(), None, HOST);
        let whats: Vec<&str> = p.setup.iter().map(|s| s.what.as_str()).collect();
        assert!(whats.iter().any(|w| w.contains("DXVK")));
        assert!(whats.iter().any(|w| w.contains("VKD3D")));
    }

    #[test]
    fn msi_runs_through_msiexec_in_its_folder() {
        let p = plan_for(
            &FileKind::Msi,
            "/dl/tool.msi",
            RunOptions {
                silent: true,
                ..Default::default()
            },
            None,
            HOST,
        );
        assert_eq!(p.launch.args, vec!["msiexec", "/i", "tool.msi", "/qn"]);
        assert_eq!(p.launch.cwd.as_deref(), Some(Path::new("/dl")));
    }

    #[test]
    fn batch_and_shortcut() {
        let p = plan_for(
            &FileKind::Batch,
            "/x/run.bat",
            RunOptions::default(),
            None,
            HOST,
        );
        assert_eq!(p.launch.args, vec!["cmd", "/c", "run.bat"]);
        let p = plan_for(
            &FileKind::Shortcut,
            "/x/App.lnk",
            RunOptions::default(),
            None,
            HOST,
        );
        assert_eq!(p.launch.args, vec!["start", "/unix", "/x/App.lnk"]);
    }

    #[test]
    fn silent_installer_flags() {
        let kind = exe(&[], Some(InstallerKind::InnoSetup));
        let p = plan_for(
            &kind,
            "/dl/setup.exe",
            RunOptions {
                silent: true,
                ..Default::default()
            },
            None,
            HOST,
        );
        assert_eq!(
            p.launch.args,
            vec![
                "/dl/setup.exe",
                "/VERYSILENT",
                "/SUPPRESSMSGBOXES",
                "/NORESTART"
            ]
        );
    }

    #[test]
    fn virtual_desktop_wraps_in_explorer() {
        let kind = exe(&[], None);
        let opts = RunOptions {
            virtual_desktop: Some("1280x720".into()),
            ..Default::default()
        };
        let p = plan_for(&kind, "/g/a.exe", opts, None, HOST);
        assert_eq!(
            p.launch.args,
            vec!["explorer", "/desktop=segrun,1280x720", "/g/a.exe"]
        );

        let kind = exe(&[], None);
        let s = settings(
            &Config::default(),
            "/g/a.exe",
            &kind,
            RunOptions {
                virtual_desktop: Some("big".into()),
                ..Default::default()
            },
        );
        assert!(
            plan(
                Path::new("/g/a.exe"),
                &kind,
                &[],
                &s,
                Path::new("/p"),
                None,
                HOST
            )
            .is_err()
        );
    }

    #[test]
    fn libraries_and_unknown_files_are_refused() {
        let mut dll = exe(&[], None);
        if let FileKind::Pe(info) = &mut dll {
            info.is_dll = true;
        }
        let s = settings(&Config::default(), "/x/a.dll", &dll, RunOptions::default());
        let err = plan(
            Path::new("/x/a.dll"),
            &dll,
            &[],
            &s,
            Path::new("/p"),
            None,
            HOST,
        )
        .unwrap_err();
        assert!(err.to_string().contains("library"));
        let s = settings(
            &Config::default(),
            "/x/a.txt",
            &FileKind::Unknown,
            RunOptions::default(),
        );
        assert!(
            plan(
                Path::new("/x/a.txt"),
                &FileKind::Unknown,
                &[],
                &s,
                Path::new("/p"),
                None,
                HOST
            )
            .is_err()
        );
    }

    #[test]
    fn app_overrides_and_isolation() {
        let config = Config::parse(
            r#"
            [defaults]
            isolate_installers = true
            [env]
            GLOBAL = "1"
            [[app]]
            match = "game*.exe"
            prefix = "games"
            runner = "umu"
            args = ["-dx11"]
            env = { APP = "2" }
            dll_overrides = ["xaudio2_7=n,b"]
            "#,
        )
        .unwrap();
        let kind = exe(&[], None);
        let s = settings(&config, "/g/Game.exe", &kind, RunOptions::default());
        assert_eq!(s.prefix, "games");
        assert_eq!(s.runner.kind(), "umu");
        let p = plan(
            Path::new("/g/Game.exe"),
            &kind,
            &["--x".into()],
            &s,
            Path::new("/p/games"),
            None,
            HOST,
        )
        .unwrap();
        assert_eq!(p.launch.program, "umu-run");
        assert_eq!(p.launch.args, vec!["/g/Game.exe", "-dx11", "--x"]);
        assert_eq!(p.launch.get_env("GLOBAL"), Some("1"));
        assert_eq!(p.launch.get_env("APP"), Some("2"));
        assert_eq!(p.launch.get_env("WINEDLLOVERRIDES"), Some("xaudio2_7=n,b"));
        // umu creates its own prefix and ships DXVK: no setup steps.
        assert!(p.setup.is_empty());

        // Installers get their own prefix when isolate_installers is set...
        let installer = exe(&[], Some(InstallerKind::Nsis));
        let s = settings(
            &config,
            "/dl/Tool Setup.exe",
            &installer,
            RunOptions::default(),
        );
        assert_eq!(s.prefix, "tool-setup");
        // ...but an explicit --prefix always wins.
        let s = settings(
            &config,
            "/dl/Tool Setup.exe",
            &installer,
            RunOptions {
                prefix: Some("default".into()),
                ..Default::default()
            },
        );
        assert_eq!(s.prefix, "default");
    }

    #[test]
    fn older_gpus_get_dxvk_1_10_and_no_vkd3d() {
        let kind = exe(&["d3d11.dll", "d3d12.dll"], None);
        let haswell = Host {
            vulkan_api: Some((1, 2)),
            ..HOST
        };
        let p = plan_for(&kind, "/g/a.exe", RunOptions::default(), None, haswell);
        let steps: Vec<String> = p.setup.iter().map(|s| s.cmd.to_string()).collect();
        assert!(
            steps.iter().any(|s| s.ends_with("winetricks -q dxvk1103")),
            "{steps:?}"
        );
        assert!(!steps.iter().any(|s| s.contains("vkd3d")));
        assert!(p.notes.iter().any(|n| n.contains("Direct3D 12")));
        assert!(p.state.dxvk && !p.state.vkd3d);

        // Only the software rasteriser: no DXVK at all.
        let llvmpipe = Host {
            vulkan_api: Some((0, 0)),
            ..HOST
        };
        let p = plan_for(&kind, "/g/a.exe", RunOptions::default(), None, llvmpipe);
        assert!(p.setup.iter().all(|s| s.kind == StepKind::InitPrefix));
        assert!(p.notes.iter().any(|n| n.contains("WineD3D")));
    }

    #[test]
    fn vulkaninfo_summary_parsing() {
        let modern = "\
Devices:
========
GPU0:
\tapiVersion         = 1.3.289
\tdeviceType         = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU
\tdeviceName         = Intel(R) UHD Graphics 620 (KBL GT2)
GPU1:
\tapiVersion         = 1.4.305
\tdeviceType         = PHYSICAL_DEVICE_TYPE_CPU
\tdeviceName         = llvmpipe (LLVM 19.1.7, 256 bits)
";
        assert_eq!(parse_vulkan_summary(modern), Some((1, 3)));
        let old = "GPU0:\n  apiVersion = 4202671 (1.2.175)\n  deviceType = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n";
        assert_eq!(parse_vulkan_summary(old), Some((1, 2)));
        let software = "GPU0:\n  apiVersion = 1.4.305\n  deviceType = PHYSICAL_DEVICE_TYPE_CPU\n";
        assert_eq!(parse_vulkan_summary(software), Some((0, 0)));
        assert_eq!(parse_vulkan_summary("ERROR: no ICD"), None);
    }

    #[test]
    fn gaming_wrappers() {
        let game = exe(&["d3d11.dll"], None);
        let opts = RunOptions {
            hud: true,
            gamescope: true,
            ..Default::default()
        };
        let p = plan_for(&game, "/g/a.exe", opts.clone(), None, HOST);
        assert_eq!(p.launch.program, "gamemoderun");
        assert_eq!(
            p.launch.args,
            vec!["gamescope", "-f", "--", "mangohud", "wine", "/g/a.exe"]
        );
        // Environment and working directory survive the wrapping.
        assert_eq!(p.launch.get_env("WINEESYNC"), Some("1"));
        assert_eq!(p.launch.cwd.as_deref(), Some(Path::new("/g")));

        // Without the tools: DXVK's own HUD and notes instead of wrappers.
        let bare = Host {
            gamemode: false,
            mangohud: false,
            gamescope: false,
            ..HOST
        };
        let p = plan_for(&game, "/g/a.exe", opts, None, bare);
        assert_eq!(p.launch.program, "wine");
        assert_eq!(p.launch.get_env("DXVK_HUD"), Some("1"));
        assert!(
            p.notes
                .iter()
                .any(|n| n.contains("gamescope is not installed"))
        );

        // GameMode can be turned off, and forced on for non-games.
        let off = RunOptions {
            gamemode: Some(Toggle::Never),
            ..Default::default()
        };
        assert_eq!(
            plan_for(&game, "/g/a.exe", off, None, HOST).launch.program,
            "wine"
        );
        let tool = exe(&["user32.dll"], None);
        assert_eq!(
            plan_for(&tool, "/t/t.exe", RunOptions::default(), None, HOST)
                .launch
                .program,
            "wine"
        );
        let on = RunOptions {
            gamemode: Some(Toggle::Always),
            ..Default::default()
        };
        assert_eq!(
            plan_for(&tool, "/t/t.exe", on, None, HOST).launch.program,
            "gamemoderun"
        );
    }

    #[test]
    fn gamescope_args_from_app_override() {
        let config = Config::parse(
            r#"
            [[app]]
            match = "game.exe"
            gamescope = true
            gamescope_args = ["-W", "2560", "-H", "1440", "-r", "165"]
            "#,
        )
        .unwrap();
        let game = exe(&["d3d12.dll"], None);
        let s = settings(&config, "/g/game.exe", &game, RunOptions::default());
        assert!(s.gamescope);
        let p = plan(
            Path::new("/g/game.exe"),
            &game,
            &[],
            &s,
            Path::new("/p"),
            None,
            HOST,
        )
        .unwrap();
        assert_eq!(
            p.launch.args,
            vec![
                "gamescope",
                "-W",
                "2560",
                "-H",
                "1440",
                "-r",
                "165",
                "--",
                "wine",
                "/g/game.exe"
            ]
        );
    }

    #[test]
    fn debug_keeps_wine_output() {
        let kind = exe(&[], None);
        let p = plan_for(
            &kind,
            "/g/a.exe",
            RunOptions {
                debug: true,
                ..Default::default()
            },
            None,
            HOST,
        );
        assert_eq!(p.launch.get_env("WINEDEBUG"), None);
    }
}
