//! The desktop sessions one can sign in to: the Wayland session files of
//! `/usr/share/wayland-sessions`, SegLinux's first.

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// The file name without `.desktop`: `seglinux`, `plasma`.
    pub id: String,
    pub name: String,
    pub command: Vec<String>,
    /// `DesktopNames`, for XDG_CURRENT_DESKTOP.
    pub desktop_names: Vec<String>,
}

/// Session files that are no desktop by themselves: a bare compositor
/// with no panel and no way to start anything.
const BARE: &[&str] = &["labwc", "sway-bare"];

/// The preferred order: SegLinux's desktop, Plasma, then the rest by name.
fn rank(id: &str) -> u8 {
    match id {
        "seglinux" => 0,
        "plasma" | "plasmawayland" => 1,
        "seglinux-gaming" => 2,
        _ => 3,
    }
}

/// Splits a desktop entry's `Exec` value into arguments (the quoting rules
/// of the Desktop Entry Specification), leaving out field codes such as
/// `%U`, which have no meaning for a session.
pub fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            // `%%` is a percent sign; any other field code is dropped, and
            // with it an argument that is nothing else.
            '%' => {
                if chars.next() == Some('%') {
                    current.push('%');
                    in_arg = true;
                }
            }
            c => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if in_arg {
        args.push(current);
    }
    args
}

/// The `[Desktop Entry]` group of a session file, or None when it is
/// hidden or has no command.
pub fn parse(id: &str, text: &str) -> Option<Session> {
    let mut in_entry = false;
    let (mut name, mut exec, mut names) = (None, None, Vec::new());
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (key.trim(), value.trim()) {
            ("Name", v) => name = Some(v.to_string()),
            ("Exec", v) => exec = Some(v.to_string()),
            ("DesktopNames", v) => {
                names = v
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            }
            ("Hidden" | "NoDisplay", "true") => return None,
            _ => {}
        }
    }
    let command = split_exec(&exec?);
    if command.is_empty() {
        return None;
    }
    Some(Session {
        id: id.to_string(),
        name: name.unwrap_or_else(|| id.to_string()),
        command,
        desktop_names: names,
    })
}

/// The sessions in `dir`, in the preferred order.
pub fn list(dir: &Path) -> Vec<Session> {
    let mut sessions: Vec<Session> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_name()?.to_str()?.strip_suffix(".desktop")?;
            if BARE.contains(&id) {
                return None;
            }
            parse(id, &std::fs::read_to_string(&path).ok()?)
        })
        .collect();
    sessions.sort_by(|a, b| {
        (rank(&a.id), a.name.to_lowercase()).cmp(&(rank(&b.id), b.name.to_lowercase()))
    });
    sessions
}

impl Session {
    /// The environment greetd gives the session; elogind also records the
    /// session type from it.
    pub fn env(&self) -> Vec<String> {
        let mut env = vec![
            "XDG_SESSION_TYPE=wayland".to_string(),
            format!("XDG_SESSION_DESKTOP={}", self.id),
            format!("DESKTOP_SESSION={}", self.id),
        ];
        if !self.desktop_names.is_empty() {
            env.push(format!(
                "XDG_CURRENT_DESKTOP={}",
                self.desktop_names.join(":")
            ));
        }
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_lines() {
        assert_eq!(split_exec("seglinux-session"), ["seglinux-session"]);
        assert_eq!(
            split_exec("/usr/lib/plasma-dbus-run-session-if-needed /usr/bin/startplasma-wayland"),
            [
                "/usr/lib/plasma-dbus-run-session-if-needed",
                "/usr/bin/startplasma-wayland"
            ]
        );
        assert_eq!(
            split_exec(r#"env "A=two words" run %U --x"#),
            ["env", "A=two words", "run", "--x"]
        );
        assert_eq!(split_exec(r#""quoted \"inner\"""#), [r#"quoted "inner""#]);
        assert_eq!(split_exec("50%% off"), ["50%", "off"]);
        assert!(split_exec("   ").is_empty());
    }

    #[test]
    fn session_files() {
        let plasma = "[Desktop Entry]\nExec=/usr/lib/plasma-dbus-run-session-if-needed /usr/bin/startplasma-wayland\nName=Plasma (Wayland)\nDesktopNames=KDE\n\n[Desktop Action Other]\nName=Other\nExec=other\n";
        let s = parse("plasma", plasma).unwrap();
        assert_eq!(s.name, "Plasma (Wayland)");
        assert_eq!(s.command.len(), 2);
        assert_eq!(s.desktop_names, ["KDE"]);
        assert!(s.env().contains(&"XDG_CURRENT_DESKTOP=KDE".to_string()));
        assert!(s.env().contains(&"XDG_SESSION_DESKTOP=plasma".to_string()));

        assert_eq!(
            parse("x", "[Desktop Entry]\nName=X\nExec=x\nHidden=true\n"),
            None
        );
        assert_eq!(parse("x", "[Desktop Entry]\nName=X\n"), None);
        let unnamed = parse("mine", "[Desktop Entry]\nExec=mine\n").unwrap();
        assert_eq!(unnamed.name, "mine");
        assert!(
            !unnamed
                .env()
                .iter()
                .any(|e| e.starts_with("XDG_CURRENT_DESKTOP"))
        );
    }

    #[test]
    fn listed_in_order_without_bare_compositors() {
        let dir = tempfile::tempdir().unwrap();
        let write = |id: &str, name: &str| {
            std::fs::write(
                dir.path().join(format!("{id}.desktop")),
                format!("[Desktop Entry]\nName={name}\nExec={id}\n"),
            )
            .unwrap()
        };
        write("labwc", "labwc");
        write("zzz", "Another Desktop");
        write("seglinux-gaming", "SegLinux Gaming Mode");
        write("plasma", "Plasma (Wayland)");
        write("seglinux", "SegLinux");
        std::fs::write(dir.path().join("notes.txt"), "not a session").unwrap();
        let names: Vec<String> = list(dir.path()).into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "SegLinux",
                "Plasma (Wayland)",
                "SegLinux Gaming Mode",
                "Another Desktop"
            ]
        );
        assert!(list(&dir.path().join("missing")).is_empty());
    }
}
