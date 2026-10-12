use std::ffi::OsString;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use wincraft_core::cmd::{Cmd, shell_quote};
use wincraft_core::which;

use wcrun::config::{Config, Toggle};
use wcrun::launch::{self, Host, RunOptions, Settings, StepKind};
use wcrun::runner::{Runner, path_str};
use wcrun::{advice, binfmt, desktop, doctor, prefix};

#[derive(Parser)]
#[command(
    name = "wcrun",
    version,
    about = "Run Windows programs on Wincraft",
    long_about = "Run Windows programs on Wincraft.\n\n\
        `wcrun setup.exe` (or double-clicking an .exe) picks a Wine prefix, prepares it once \
        (Wine environment, DXVK, VKD3D-Proton) and starts the program from its own folder."
)]
struct Cli {
    /// Use this configuration file instead of ~/.config/wincraft/wcrun.toml
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a Windows program, installer (.exe/.msi), shortcut (.lnk) or batch file
    #[command(visible_alias = "install", visible_alias = "open")]
    Run(RunArgs),
    /// Explain what a file is and how Wincraft will run it
    Info { file: PathBuf },
    /// Manage Windows environments (Wine prefixes, the "C: drives")
    Prefix {
        #[command(subcommand)]
        command: PrefixCommand,
    },
    /// Open a Windows system tool inside a prefix
    Tool {
        #[arg(value_enum)]
        tool: Tool,
        #[arg(long, short)]
        prefix: Option<String>,
    },
    /// Add a start menu entry for a Windows program
    Shortcut {
        file: PathBuf,
        /// Name shown in the menu (defaults to the file name)
        #[arg(long)]
        name: Option<String>,
        #[arg(long, short)]
        prefix: Option<String>,
        /// Icon name or path
        #[arg(long)]
        icon: Option<String>,
    },
    /// Check that this machine is ready to run Windows programs
    Doctor,
    /// Let the kernel start .exe files through wcrun (binfmt_misc)
    Binfmt {
        #[command(subcommand)]
        command: BinfmtCommand,
    },
    /// Show the effective configuration
    Config {
        /// Only print which file is used
        #[arg(long)]
        path: bool,
    },
}

#[derive(clap::Args)]
struct RunArgs {
    /// Prefix to run in (default: the shared one, see `wcrun config`)
    #[arg(long, short)]
    prefix: Option<String>,
    /// Use a separate prefix named after the program
    #[arg(long, conflicts_with = "prefix")]
    isolate: bool,
    /// wine | wine:<binary> | proton:<dir> | umu | umu:<proton dir>
    #[arg(long)]
    runner: Option<String>,
    /// Direct3D 9/10/11 over Vulkan
    #[arg(long, value_enum)]
    dxvk: Option<Toggle>,
    /// Direct3D 12 over Vulkan
    #[arg(long, value_enum)]
    vkd3d: Option<Toggle>,
    /// Install without questions (Inno Setup, NSIS, MSI, ...)
    #[arg(long)]
    silent: bool,
    /// Run inside a window of this size, e.g. 1920x1080
    #[arg(long, value_name = "WxH")]
    virtual_desktop: Option<String>,
    /// Show the DXVK performance overlay
    #[arg(long)]
    hud: bool,
    /// Keep Wine's debug output
    #[arg(long)]
    debug: bool,
    /// Print what would be done without doing it
    #[arg(long, short = 'n')]
    dry_run: bool,
    /// The program to run
    file: PathBuf,
    /// Arguments passed to the program
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Subcommand)]
enum PrefixCommand {
    /// List prefixes
    List,
    /// Create a prefix
    Create {
        name: String,
        #[arg(long)]
        runner: Option<String>,
    },
    /// Delete a prefix and every program installed in it
    Remove {
        name: String,
        /// Confirm the deletion
        #[arg(long)]
        yes: bool,
    },
    /// Print the directory of a prefix
    Path { name: String },
    /// Open the prefix's C: drive in the file manager
    Open { name: Option<String> },
    /// Install winetricks verbs (e.g. vcrun2022 corefonts dotnet48)
    Tricks {
        name: String,
        #[arg(required = true)]
        verbs: Vec<String>,
    },
}

#[derive(Subcommand)]
enum BinfmtCommand {
    /// Show which handler starts Windows programs
    Status,
    /// Register wcrun as the handler (root)
    Register,
    /// Remove wcrun's handler (root)
    Unregister,
}

#[derive(Clone, Copy, ValueEnum)]
enum Tool {
    /// Wine configuration (Windows version, drives, graphics)
    Winecfg,
    /// Registry editor
    Regedit,
    /// Task manager
    Taskmgr,
    /// Control panel
    Control,
    /// Windows Explorer
    Explorer,
    /// Command prompt
    Cmd,
    /// Add/remove programs
    Uninstaller,
    /// Notepad
    Notepad,
}

impl Tool {
    fn command_line(self) -> Vec<String> {
        let parts: &[&str] = match self {
            Tool::Winecfg => &["winecfg"],
            Tool::Regedit => &["regedit"],
            Tool::Taskmgr => &["taskmgr"],
            Tool::Control => &["control"],
            Tool::Explorer => &["explorer"],
            Tool::Cmd => &["wineconsole", "cmd"],
            Tool::Uninstaller => &["uninstaller"],
            Tool::Notepad => &["notepad"],
        };
        parts.iter().map(|s| s.to_string()).collect()
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse_from(shorthand(std::env::args_os().collect()));
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("wcrun: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// `wcrun app.exe args...` is short for `wcrun run -- app.exe args...`.
/// This is also how binfmt_misc and file managers invoke wcrun.
fn shorthand(mut argv: Vec<OsString>) -> Vec<OsString> {
    if let Some(first) = argv.get(1) {
        let first_str = first.to_string_lossy();
        let cmd = Cli::command();
        let is_subcommand = cmd
            .get_subcommands()
            .any(|s| s.get_name() == first_str || s.get_all_aliases().any(|a| a == first_str));
        if !first_str.starts_with('-') && !is_subcommand && Path::new(first).is_file() {
            argv.splice(1..1, [OsString::from("run"), OsString::from("--")]);
        }
    }
    argv
}

fn dispatch(cli: Cli) -> Result<ExitCode> {
    let (config, config_path) = Config::load(cli.config.as_deref())?;
    match cli.command {
        Command::Run(args) => run(&config, args),
        Command::Info { file } => info(&config, &file),
        Command::Prefix { command } => prefix_command(&config, command),
        Command::Tool { tool, prefix } => {
            let runner = default_runner(&config)?;
            let name = prefix.unwrap_or_else(|| config.defaults.prefix.clone());
            let dir = prefix::dir(&name)?;
            ensure_prefix(&runner, &name, &dir)?;
            exec(runner.command(&dir, &tool.command_line()))
        }
        Command::Shortcut {
            file,
            name,
            prefix,
            icon,
        } => shortcut(&config, &file, name, prefix, icon),
        Command::Doctor => {
            let wine = default_runner(&config)
                .ok()
                .and_then(|r| r.wine_binary().map(str::to_string))
                .unwrap_or_else(|| "wine".into());
            let checks = doctor::run_checks(&wine);
            for c in &checks {
                println!("{c}");
            }
            let failed = checks.iter().any(|c| c.status == doctor::Status::Fail);
            Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Binfmt { command } => {
            match command {
                BinfmtCommand::Status => {
                    let handlers = binfmt::mz_handlers(Path::new(binfmt::BINFMT_DIR))?;
                    if handlers.is_empty() {
                        println!("No handler is registered for Windows programs.");
                    }
                    for h in handlers {
                        let state = if h.enabled { "enabled" } else { "disabled" };
                        println!("{}: {} ({state})", h.name, h.interpreter);
                    }
                }
                BinfmtCommand::Register => {
                    let exe = std::env::current_exe().context("cannot find wcrun's own path")?;
                    binfmt::register(&exe)?;
                    println!("Registered {}", binfmt::rule(&exe.to_string_lossy()));
                }
                BinfmtCommand::Unregister => binfmt::unregister()?,
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Config { path } => {
            if path {
                match config_path {
                    Some(p) => println!("{}", p.display()),
                    None => println!(
                        "(built-in defaults; create {})",
                        Config::user_path().display()
                    ),
                }
            } else {
                print!("{}", toml::to_string_pretty(&config)?);
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn default_runner(config: &Config) -> Result<Runner> {
    config.defaults.runner.parse().map_err(anyhow::Error::msg)
}

fn require_runner(runner: &Runner) -> Result<()> {
    let program = runner.required_program();
    if which(&program).is_none() {
        bail!(
            "`{program}` was not found. Install the runner first (for Wine: sudo pacman -S wine winetricks)"
        );
    }
    Ok(())
}

/// Create a prefix with plain Wine if it does not exist yet.
fn ensure_prefix(runner: &Runner, name: &str, dir: &Path) -> Result<()> {
    require_runner(runner)?;
    if matches!(runner, Runner::Wine { .. }) && !prefix::is_initialized(&runner.wineprefix(dir)) {
        eprintln!("wcrun: Creating the Windows environment `{name}`...");
        fs::create_dir_all(dir)?;
        runner
            .command(dir, &["wineboot".into(), "--init".into()])
            .env("WINEDEBUG", "-all")
            .run()?;
    }
    Ok(())
}

fn exec(cmd: Cmd) -> Result<ExitCode> {
    let err = cmd.to_command().exec();
    Err(err).with_context(|| format!("cannot start `{}`", cmd.program))
}

fn run(config: &Config, args: RunArgs) -> Result<ExitCode> {
    let file = fs::canonicalize(&args.file)
        .with_context(|| format!("cannot open {}", args.file.display()))?;
    let kind = wincraft_pe::analyze_path(&file)?;
    let opts = RunOptions {
        prefix: args.prefix,
        isolate: args.isolate,
        runner: args.runner,
        dxvk: args.dxvk,
        vkd3d: args.vkd3d,
        silent: args.silent,
        virtual_desktop: args.virtual_desktop,
        hud: args.hud,
        debug: args.debug,
    };
    let settings = Settings::resolve(config, &file, &kind, &opts)?;
    let prefix_dir = prefix::dir(&settings.prefix)?;
    let state = prefix::load_state(&prefix_dir);
    let plan = launch::plan(
        &file,
        &kind,
        &args.args,
        &settings,
        &prefix_dir,
        state,
        Host::detect(),
    )?;

    for note in &plan.notes {
        eprintln!("wcrun: {note}");
    }
    if args.dry_run {
        println!("# {}: {}", file.display(), kind.describe());
        println!("# prefix `{}`: {}", settings.prefix, prefix_dir.display());
        for step in &plan.setup {
            println!("# {}\n{}", step.what, step.cmd);
        }
        if let Some(cwd) = &plan.launch.cwd {
            println!("cd {}", shell_quote(&path_str(cwd)));
        }
        println!("{}", plan.launch);
        return Ok(ExitCode::SUCCESS);
    }

    require_runner(&settings.runner)?;
    fs::create_dir_all(&prefix_dir)?;
    let mut state = plan.state.clone();
    for step in &plan.setup {
        eprintln!("wcrun: {}...", step.what);
        if let Err(e) = step.cmd.run() {
            if step.kind == StepKind::InitPrefix {
                return Err(e);
            }
            eprintln!("wcrun: warning: {e:#}; continuing without it");
            state.mark(step.kind, false);
        }
    }
    prefix::save_state(&prefix_dir, &state)?;
    exec(plan.launch)
}

fn info(config: &Config, file: &Path) -> Result<ExitCode> {
    let file = fs::canonicalize(file).with_context(|| format!("cannot open {}", file.display()))?;
    let kind = wincraft_pe::analyze_path(&file)?;
    println!("File:      {}", file.display());
    println!("Type:      {}", kind.describe());
    if let wincraft_pe::FileKind::Pe(pe) = &kind {
        println!("Machine:   {}", pe.machine);
        println!("Subsystem: {}", pe.subsystem);
        if let Some(installer) = pe.installer {
            println!("Installer: {installer}");
        }
        if pe.overlay_size > 0 {
            println!("Payload:   {} bytes appended", pe.overlay_size);
        }
        if !pe.imports.is_empty() {
            println!("Imports:   {}", pe.imports.join(", "));
        }
    }
    let notes = advice::advise(&kind);
    if !notes.is_empty() {
        println!();
        for n in notes {
            println!("{n}");
        }
    }
    if kind.is_launchable() {
        let settings = Settings::resolve(config, &file, &kind, &RunOptions::default())?;
        let prefix_dir = prefix::dir(&settings.prefix)?;
        if let Ok(plan) = launch::plan(
            &file,
            &kind,
            &[],
            &settings,
            &prefix_dir,
            prefix::load_state(&prefix_dir),
            Host::detect(),
        ) {
            println!(
                "\nWincraft will run it with {} in prefix `{}`:",
                settings.runner, settings.prefix
            );
            for step in &plan.setup {
                println!("  (once) {}", step.what);
            }
            println!("  {}", plan.launch);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn prefix_command(config: &Config, command: PrefixCommand) -> Result<ExitCode> {
    match command {
        PrefixCommand::List => {
            let names = prefix::list()?;
            if names.is_empty() {
                println!(
                    "No prefixes yet. They are created in {}",
                    prefix::root().display()
                );
            }
            for name in names {
                let dir = prefix::dir(&name)?;
                let state = prefix::load_state(&dir).unwrap_or_default();
                let mut extras = Vec::new();
                if !state.runner.is_empty() {
                    extras.push(state.runner.clone());
                }
                if state.dxvk {
                    extras.push("dxvk".into());
                }
                if state.vkd3d {
                    extras.push("vkd3d".into());
                }
                extras.extend(state.verbs.iter().cloned());
                println!("{name:<24} {}", extras.join(", "));
            }
        }
        PrefixCommand::Create { name, runner } => {
            let runner: Runner = runner
                .unwrap_or_else(|| config.defaults.runner.clone())
                .parse()
                .map_err(anyhow::Error::msg)?;
            let dir = prefix::dir(&name)?;
            ensure_prefix(&runner, &name, &dir)?;
            let mut state = prefix::load_state(&dir).unwrap_or_default();
            state.runner = runner.to_string();
            state.created_by = format!("wcrun {}", env!("CARGO_PKG_VERSION"));
            prefix::save_state(&dir, &state)?;
            println!("{}", dir.display());
        }
        PrefixCommand::Remove { name, yes } => {
            let dir = prefix::dir(&name)?;
            if !dir.exists() {
                bail!("prefix `{name}` does not exist");
            }
            if !yes {
                bail!(
                    "this deletes {} and every program installed in it; repeat with --yes to confirm",
                    dir.display()
                );
            }
            fs::remove_dir_all(&dir)?;
            println!("Removed {}", dir.display());
        }
        PrefixCommand::Path { name } => println!("{}", prefix::dir(&name)?.display()),
        PrefixCommand::Open { name } => {
            let runner = default_runner(config)?;
            let name = name.unwrap_or_else(|| config.defaults.prefix.clone());
            let dir = prefix::dir(&name)?;
            let drive_c = runner.wineprefix(&dir).join("drive_c");
            if !drive_c.exists() {
                bail!("prefix `{name}` has not been created yet");
            }
            return exec(Cmd::new("xdg-open").arg(path_str(&drive_c)));
        }
        PrefixCommand::Tricks { name, verbs } => {
            let runner = default_runner(config)?;
            let dir = prefix::dir(&name)?;
            ensure_prefix(&runner, &name, &dir)?;
            if which("winetricks").is_none() {
                bail!("winetricks is not installed: sudo pacman -S winetricks");
            }
            let refs: Vec<&str> = verbs.iter().map(String::as_str).collect();
            launch::winetricks(&runner, &dir, &refs).run()?;
            let mut state = prefix::load_state(&dir).unwrap_or_default();
            for v in verbs {
                if !state.verbs.contains(&v) {
                    state.verbs.push(v);
                }
            }
            prefix::save_state(&dir, &state)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn shortcut(
    config: &Config,
    file: &Path,
    name: Option<String>,
    prefix_name: Option<String>,
    icon: Option<String>,
) -> Result<ExitCode> {
    let file = fs::canonicalize(file).with_context(|| format!("cannot open {}", file.display()))?;
    let kind = wincraft_pe::analyze_path(&file)?;
    if !kind.is_launchable() {
        bail!(
            "{} is not something Wincraft can run ({})",
            file.display(),
            kind.describe()
        );
    }
    let name = name.unwrap_or_else(|| {
        file.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Windows program".into())
    });
    let prefix_name = prefix_name.unwrap_or_else(|| config.defaults.prefix.clone());
    prefix::validate_name(&prefix_name)?;
    let wcrun = std::env::current_exe()
        .map(|p| path_str(&p))
        .unwrap_or_else(|_| "wcrun".into());
    let entry = desktop::Shortcut {
        name: &name,
        target: &file,
        prefix: &prefix_name,
        icon: icon.as_deref(),
        wcrun: &wcrun,
    };
    let dir = wincraft_core::xdg::applications_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(desktop::file_name(&name));
    fs::write(&path, entry.render())?;
    println!("Created {}", path.display());
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn shorthand_inserts_run_for_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("app.exe");
        fs::write(&exe, b"MZ").unwrap();
        let exe = exe.to_str().unwrap();

        assert_eq!(
            shorthand(os(&["wcrun", exe, "-n", "x"])),
            os(&["wcrun", "run", "--", exe, "-n", "x"])
        );
        assert_eq!(
            shorthand(os(&["wcrun", "doctor"])),
            os(&["wcrun", "doctor"])
        );
        assert_eq!(
            shorthand(os(&["wcrun", "--help"])),
            os(&["wcrun", "--help"])
        );
        assert_eq!(shorthand(os(&["wcrun"])), os(&["wcrun"]));
    }

    #[test]
    fn program_arguments_may_look_like_flags() {
        let cli = Cli::try_parse_from([
            "wcrun",
            "run",
            "--prefix",
            "games",
            "game.exe",
            "-dx11",
            "--windowed",
        ])
        .unwrap();
        let Command::Run(args) = cli.command else {
            panic!("expected run");
        };
        assert_eq!(args.prefix.as_deref(), Some("games"));
        assert_eq!(args.file, PathBuf::from("game.exe"));
        assert_eq!(args.args, vec!["-dx11", "--windowed"]);
    }
}
