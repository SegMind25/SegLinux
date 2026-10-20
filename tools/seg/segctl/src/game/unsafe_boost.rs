//! `seg game unsafe-boost`: `mitigations=off` on the kernel command line.
//! Opt-in, with a warning, never done by anything else in SegLinux.

use std::fs;
use std::path::Path;

use crate::error::{Error, Result};
use crate::sys;

pub const HELP: &str = "\
Usage: seg game unsafe-boost on|off|status [--i-understand]

Adds or removes `mitigations=off` in GRUB_CMDLINE_LINUX_DEFAULT of
/etc/default/grub and rebuilds the boot menu. It takes effect at the next
boot. `on` only runs with --i-understand, after you have read this:
";

pub const WARNING: &str = "\
WARNING: mitigations=off switches off the kernel's protections against CPU
security flaws (Spectre, Meltdown, MDS, Retbleed and the others). While it is
on, any program on this computer, including JavaScript in a web page, may be
able to read memory it must not read: passwords, keys, other programs' data.

The speed it gives back is usually small, bigger on older CPUs, and nothing at
all in many games. Measure first (docs/gaming.md has a method) and keep it
only if the numbers are worth it to you. Do not use it on a computer you share,
or use for banking or work.

SegLinux never turns this on by itself. `seg game unsafe-boost off` undoes it.
";

const PARAM: &str = "mitigations=off";
const KEY: &str = "GRUB_CMDLINE_LINUX_DEFAULT";

/// The words inside the quotes of `KEY="..."`, and the quote used.
fn split_value(value: &str) -> Option<(char, Vec<&str>)> {
    let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let inner = value.strip_prefix(quote)?.strip_suffix(quote)?;
    Some((quote, inner.split_whitespace().collect()))
}

/// /etc/default/grub with `param` added (`add`) or removed. The bool says
/// whether anything changed.
pub fn edit_grub(text: &str, param: &str, add: bool) -> (String, bool) {
    let mut changed = false;
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| {
            let Some(value) = line
                .trim_start()
                .strip_prefix(KEY)
                .and_then(|r| r.strip_prefix('='))
            else {
                return line.to_string();
            };
            let Some((quote, mut words)) = split_value(value.trim()) else {
                return line.to_string();
            };
            found = true;
            let present = words.contains(&param);
            if add && !present {
                words.push(param);
                changed = true;
            } else if !add && present {
                words.retain(|w| *w != param);
                changed = true;
            } else {
                return line.to_string();
            }
            format!("{KEY}={quote}{}{quote}", words.join(" "))
        })
        .collect();
    if add && !found {
        lines.push(format!("{KEY}=\"{param}\""));
        changed = true;
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') || (add && !found && !text.is_empty()) {
        out.push('\n');
    }
    (out, changed)
}

/// Whether the configured and the running command lines have the parameter.
pub fn status(grub: &str, cmdline: &str) -> (bool, bool) {
    let configured = grub
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix(KEY)?.strip_prefix('='))
        .filter_map(|v| split_value(v.trim()))
        .any(|(_, words)| words.contains(&PARAM));
    let running = cmdline.split_whitespace().any(|w| w == PARAM);
    (configured, running)
}

pub enum Action {
    On { understood: bool },
    Off,
    Status,
}

pub fn parse_args(parser: &mut lexopt::Parser) -> Result<Action> {
    use lexopt::prelude::*;
    let mut word: Option<String> = None;
    let mut understood = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("i-understand") => understood = true,
            Short('h') | Long("help") => {
                out!("{HELP}\n{WARNING}");
                std::process::exit(0);
            }
            Value(v) if word.is_none() => word = Some(v.string()?),
            other => return Err(other.unexpected().into()),
        }
    }
    match word.as_deref() {
        Some("on") => Ok(Action::On { understood }),
        Some("off") => Ok(Action::Off),
        None | Some("status") => Ok(Action::Status),
        Some(other) => Err(Error::Usage(format!(
            "unknown unsafe-boost command `{other}` (on, off or status)"
        ))),
    }
}

/// Edit the file under `root` and keep the old one next to it.
pub fn apply(root: &Path, add: bool) -> Result<bool> {
    let path = root.join("etc/default/grub");
    let text = sys::read(&path)?;
    let (new, changed) = edit_grub(&text, PARAM, add);
    if changed {
        let mut backup = path.as_os_str().to_owned();
        backup.push(".seg-bak");
        fs::write(&backup, &text)
            .map_err(|e| Error::io("saving a copy of /etc/default/grub", e))?;
        sys::write_atomic(&path, &new)?;
    }
    Ok(changed)
}

pub fn run(action: Action) -> Result<u8> {
    let root = Path::new("/");
    match action {
        Action::Status => {
            let grub = sys::read_trimmed("/etc/default/grub").unwrap_or_default();
            let cmdline = sys::read_trimmed("/proc/cmdline").unwrap_or_default();
            let (configured, running) = status(&grub, &cmdline);
            outln!(
                "mitigations=off in /etc/default/grub: {}",
                if configured { "yes" } else { "no" }
            );
            outln!(
                "mitigations=off on the running kernel: {}",
                if running {
                    "yes (the security mitigations are OFF)"
                } else {
                    "no"
                }
            );
            Ok(0)
        }
        Action::On { understood } => {
            out!("{WARNING}\n");
            if !understood {
                outln!(
                    "Nothing was changed. To turn it on anyway:\n  sudo seg game unsafe-boost on --i-understand"
                );
                return Ok(1);
            }
            sys::require_root("`seg game unsafe-boost on`")?;
            rebuild_menu(root, true)
        }
        Action::Off => {
            sys::require_root("`seg game unsafe-boost off`")?;
            rebuild_menu(root, false)
        }
    }
}

fn rebuild_menu(root: &Path, add: bool) -> Result<u8> {
    if apply(root, add)? {
        if sys::which("grub-mkconfig").is_some() {
            sys::run_inherit("grub-mkconfig", &["-o", "/boot/grub/grub.cfg"])?;
        } else {
            outln!("grub-mkconfig is not installed: rebuild your boot menu yourself.");
        }
        outln!(
            "mitigations=off is now {} in the boot menu. Reboot for it to take effect. (A copy of the old /etc/default/grub is /etc/default/grub.seg-bak.)",
            if add { "ON" } else { "off" }
        );
    } else {
        outln!(
            "nothing to change: mitigations=off is already {}.",
            if add { "on" } else { "off" }
        );
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRUB: &str = "GRUB_DEFAULT=0\n#GRUB_CMDLINE_LINUX_DEFAULT=\"old\"\nGRUB_CMDLINE_LINUX_DEFAULT=\"loglevel=3 quiet nowatchdog\"\nGRUB_CMDLINE_LINUX=\"\"\n";

    #[test]
    fn adding_keeps_the_rest_of_the_line_and_file() {
        let (new, changed) = edit_grub(GRUB, PARAM, true);
        assert!(changed);
        assert!(
            new.contains(
                "GRUB_CMDLINE_LINUX_DEFAULT=\"loglevel=3 quiet nowatchdog mitigations=off\"\n"
            ),
            "{new}"
        );
        assert!(
            new.contains("#GRUB_CMDLINE_LINUX_DEFAULT=\"old\"\n"),
            "comments are untouched"
        );
        assert!(new.contains("GRUB_CMDLINE_LINUX=\"\"\n"));
        assert!(new.ends_with('\n'));
    }

    #[test]
    fn it_is_idempotent_both_ways() {
        let (on, _) = edit_grub(GRUB, PARAM, true);
        assert_eq!(edit_grub(&on, PARAM, true), (on.clone(), false));
        let (off, changed) = edit_grub(&on, PARAM, false);
        assert!(changed);
        assert_eq!(off, GRUB);
        assert_eq!(edit_grub(GRUB, PARAM, false), (GRUB.to_string(), false));
    }

    #[test]
    fn single_quotes_and_a_missing_line() {
        let (new, _) = edit_grub("GRUB_CMDLINE_LINUX_DEFAULT='quiet'\n", PARAM, true);
        assert_eq!(new, "GRUB_CMDLINE_LINUX_DEFAULT='quiet mitigations=off'\n");
        let (new, changed) = edit_grub("GRUB_DEFAULT=0\n", PARAM, true);
        assert!(changed);
        assert_eq!(
            new,
            "GRUB_DEFAULT=0\nGRUB_CMDLINE_LINUX_DEFAULT=\"mitigations=off\"\n"
        );
        // Removing from a file without the line does nothing.
        assert_eq!(
            edit_grub("GRUB_DEFAULT=0\n", PARAM, false),
            ("GRUB_DEFAULT=0\n".to_string(), false)
        );
        // A value that is not quoted is not guessed at.
        assert_eq!(
            edit_grub("GRUB_CMDLINE_LINUX_DEFAULT=quiet\n", PARAM, false),
            ("GRUB_CMDLINE_LINUX_DEFAULT=quiet\n".to_string(), false)
        );
    }

    #[test]
    fn a_similar_word_is_not_the_parameter() {
        let g = "GRUB_CMDLINE_LINUX_DEFAULT=\"quiet nomitigations=off2 xmitigations=off\"\n";
        assert_eq!(edit_grub(g, PARAM, false), (g.to_string(), false));
        assert_eq!(status(g, "quiet"), (false, false));
    }

    #[test]
    fn status_tells_configured_from_running() {
        let (on, _) = edit_grub(GRUB, PARAM, true);
        assert_eq!(status(&on, "root=/dev/sda quiet"), (true, false));
        assert_eq!(status(GRUB, "quiet mitigations=off"), (false, true));
        assert_eq!(status("", ""), (false, false));
    }

    #[test]
    fn the_file_is_backed_up_and_replaced() {
        let t = sys::testutil::TempDir::new();
        let path = t.put("etc/default/grub", GRUB);
        assert!(apply(t.path(), true).unwrap());
        assert!(fs::read_to_string(&path).unwrap().contains(PARAM));
        assert_eq!(
            fs::read_to_string(t.path().join("etc/default/grub.seg-bak")).unwrap(),
            GRUB
        );
        assert!(!apply(t.path(), true).unwrap(), "already on");
        assert!(apply(t.path(), false).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), GRUB);
        assert!(apply(&t.path().join("nowhere"), true).is_err());
    }

    #[test]
    fn the_warning_says_what_it_risks_and_how_to_undo() {
        assert!(WARNING.contains("Spectre"));
        assert!(WARNING.contains("JavaScript in a web page"));
        assert!(WARNING.contains("unsafe-boost off"));
        assert!(!WARNING.contains('%'), "no invented percentages");
    }

    #[test]
    fn arguments() {
        let parse = |a: &[&str]| parse_args(&mut lexopt::Parser::from_args(a.iter().copied()));
        assert!(matches!(
            parse(&["on"]),
            Ok(Action::On { understood: false })
        ));
        assert!(matches!(
            parse(&["on", "--i-understand"]),
            Ok(Action::On { understood: true })
        ));
        assert!(matches!(parse(&["off"]), Ok(Action::Off)));
        assert!(matches!(parse(&[]), Ok(Action::Status)));
        assert!(matches!(parse(&["maybe"]), Err(Error::Usage(_))));
    }
}
