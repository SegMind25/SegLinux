use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use seglinux_core::cmd::Cmd;

use seglinux_setup::answer::{Answer, EXAMPLE, Finish};
use seglinux_setup::exec::{Event, Executor, LOG_DIR, Log};
use seglinux_setup::facts::{Facts, find_answer_file};
use seglinux_setup::ids::{IdSource, Random, Sequential};
use seglinux_setup::{is_root, plan};

#[derive(Parser)]
#[command(
    name = "seglinux-setup",
    version,
    about = "Install SegLinux on this computer",
    long_about = "Install SegLinux on this computer.\n\n\
        Without arguments the interactive installer starts. With an answer file \
        (autounattend.toml, the counterpart of Windows' autounattend.xml) installation \
        runs unattended."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Install SegLinux (interactive unless --unattend is given)
    Install(InstallArgs),
    /// Show everything an installation would do, without doing it
    Plan(InstallArgs),
    /// Check an answer file
    Validate { file: PathBuf },
    /// Print a commented example answer file
    Example,
    /// List the disks SegLinux can be installed to
    Disks,
    /// Print what Setup detected about this computer (JSON)
    Facts,
    /// Print the path of the answer file Setup would use
    FindAnswerFile {
        /// Only succeed if the answer file asks for auto_start
        #[arg(long)]
        require_autostart: bool,
    },
}

#[derive(clap::Args, Clone)]
struct InstallArgs {
    /// Use an answer file. Without a path (or with `auto`), look for
    /// autounattend.toml on the kernel command line, the install media and
    /// removable drives.
    #[arg(long, value_name = "FILE", num_args = 0..=1, default_missing_value = "auto")]
    unattend: Option<PathBuf>,
    /// Mount point for the new system
    #[arg(long, default_value = plan::DEFAULT_TARGET)]
    target: PathBuf,
    /// Use predictable UUIDs and names (for comparing plans)
    #[arg(long, hide = true)]
    reproducible: bool,
    /// How an unattended installation reports progress
    #[arg(long, value_enum, default_value_t = ProgressFormat::Text)]
    progress: ProgressFormat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
enum ProgressFormat {
    /// Lines for people
    #[default]
    Text,
    /// One JSON object per line, for the graphical installer
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        None => interactive(Path::new(plan::DEFAULT_TARGET)),
        Some(Command::Install(args)) if args.unattend.is_none() => interactive(&args.target),
        Some(Command::Install(args)) => unattended(&args),
        Some(Command::Plan(args)) => print_plan(&args),
        Some(Command::Validate { file }) => validate(&file),
        Some(Command::Example) => {
            print!("{EXAMPLE}");
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Disks) => disks(),
        Some(Command::Facts) => Facts::probe().and_then(|f| {
            println!("{}", serde_json::to_string_pretty(&f)?);
            Ok(ExitCode::SUCCESS)
        }),
        Some(Command::FindAnswerFile { require_autostart }) => find(require_autostart),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("seglinux-setup: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn load_answer(arg: &Path) -> Result<(Answer, PathBuf)> {
    let explicit = (arg != Path::new("auto")).then_some(arg);
    let Some((path, source)) = find_answer_file(explicit)? else {
        bail!(
            "no answer file found (looked for autounattend.toml on the kernel command line, the install media, removable drives and /etc/seglinux)"
        );
    };
    eprintln!("Using answer file {} ({source})", path.display());
    Ok((Answer::load(&path)?, path))
}

fn make_plan(args: &InstallArgs) -> Result<plan::Plan> {
    let unattend = args.unattend.clone().unwrap_or_else(|| "auto".into());
    let (answer, _) = load_answer(&unattend)?;
    let facts = Facts::probe()?;
    let mut ids: Box<dyn IdSource> = if args.reproducible {
        Box::new(Sequential::default())
    } else {
        Box::new(Random)
    };
    plan::build(&answer, &facts, ids.as_mut(), &args.target)
}

fn print_plan(args: &InstallArgs) -> Result<ExitCode> {
    print!("{}", make_plan(args)?);
    Ok(ExitCode::SUCCESS)
}

fn unattended(args: &InstallArgs) -> Result<ExitCode> {
    let json = args.progress == ProgressFormat::Json;
    match install_unattended(args, json) {
        Ok(how) => {
            if json {
                println!("{}", Event::Done.to_json());
            } else {
                println!("\nSegLinux is installed.");
            }
            finish(how)
        }
        Err(e) => {
            if json {
                let message = format!("{e:#}");
                println!("{}", Event::Error { message }.to_json());
            }
            Err(e)
        }
    }
}

fn install_unattended(args: &InstallArgs, json: bool) -> Result<Finish> {
    if !is_root() {
        bail!("Setup must run as root (sudo seglinux-setup ...)");
    }
    let unattend = args.unattend.clone().unwrap_or_else(|| "auto".into());
    let (answer, _) = load_answer(&unattend)?;
    let facts = Facts::probe()?;
    let plan = plan::build(&answer, &facts, &mut Random, &args.target)?;
    if !json {
        println!("{}", summary_of(&plan));
    }
    let mut log = Log::open(Path::new(LOG_DIR))?;
    let result = Executor::new(&args.target, facts.init, &mut log).run(&plan, &mut |p| {
        if json {
            println!("{}", Event::from(&p).to_json());
        } else {
            println!(
                "[{:>3}%] {} - {}",
                p.percent(),
                p.pass.windows_name(),
                p.description
            );
        }
    });
    if let Err(e) = result {
        eprintln!("\nSetup failed. Details are in {LOG_DIR}/setupact.log and setuperr.log.");
        return Err(e);
    }
    Ok(answer.setup.finish)
}

fn summary_of(plan: &plan::Plan) -> String {
    plan.to_string()
        .lines()
        .take_while(|l| !l.starts_with("=="))
        .collect::<Vec<_>>()
        .join("\n")
}

fn finish(finish: Finish) -> Result<ExitCode> {
    let action = match finish {
        Finish::Reboot => "reboot",
        Finish::Poweroff => "poweroff",
        Finish::None => return Ok(ExitCode::SUCCESS),
    };
    // The init system's command first, elogind's as a fallback.
    if Cmd::new(action).run().is_err() {
        Cmd::new("loginctl").arg(action).run()?;
    }
    Ok(ExitCode::SUCCESS)
}

fn validate(file: &Path) -> Result<ExitCode> {
    let answer = Answer::load(file)?;
    let errors = answer.validate();
    if errors.is_empty() {
        println!("{}: OK", file.display());
        Ok(ExitCode::SUCCESS)
    } else {
        for e in errors {
            println!("{}: {e}", file.display());
        }
        Ok(ExitCode::FAILURE)
    }
}

fn disks() -> Result<ExitCode> {
    let facts = Facts::probe()?;
    for d in &facts.disks {
        let note = if facts.boot_disk.as_ref() == Some(&d.path) {
            "  <- install media"
        } else if d.read_only {
            "  (read-only)"
        } else {
            ""
        };
        println!("{}{note}", d.label());
    }
    if let Some(d) = facts.auto_disk() {
        println!("\ndevice = \"auto\" would choose {}", d.path);
    }
    Ok(ExitCode::SUCCESS)
}

fn find(require_autostart: bool) -> Result<ExitCode> {
    let Some((path, source)) = find_answer_file(None)? else {
        return Ok(ExitCode::FAILURE);
    };
    if require_autostart {
        let answer = Answer::load(&path)?;
        if !answer.setup.auto_start {
            return Ok(ExitCode::FAILURE);
        }
    }
    println!("{}", path.display());
    eprintln!("({source})");
    Ok(ExitCode::SUCCESS)
}

#[cfg(feature = "tui")]
fn interactive(target: &Path) -> Result<ExitCode> {
    if !is_root() {
        bail!("Setup must run as root (sudo seglinux-setup)");
    }
    use anyhow::Context;
    let facts = Facts::probe().context("cannot inspect this computer")?;
    match seglinux_setup::tui::run(facts, target)? {
        Some(how) => finish(how),
        None => Ok(ExitCode::SUCCESS),
    }
}

#[cfg(not(feature = "tui"))]
fn interactive(_target: &Path) -> Result<ExitCode> {
    bail!("this build has no interactive installer; use --unattend <answer file>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn unattend_flag_with_and_without_path() {
        let cli = Cli::try_parse_from(["seglinux-setup", "install", "--unattend"]).unwrap();
        let Some(Command::Install(args)) = cli.command else {
            panic!()
        };
        assert_eq!(args.unattend, Some(PathBuf::from("auto")));

        let cli = Cli::try_parse_from(["seglinux-setup", "plan", "--unattend", "a.toml"]).unwrap();
        let Some(Command::Plan(args)) = cli.command else {
            panic!()
        };
        assert_eq!(args.unattend, Some(PathBuf::from("a.toml")));
        assert_eq!(args.target, PathBuf::from("/mnt/seglinux"));
        assert_eq!(args.progress, ProgressFormat::Text);

        let cli = Cli::try_parse_from([
            "seglinux-setup",
            "install",
            "--unattend",
            "a.toml",
            "--progress",
            "json",
        ])
        .unwrap();
        let Some(Command::Install(args)) = cli.command else {
            panic!()
        };
        assert_eq!(args.progress, ProgressFormat::Json);
    }
}
