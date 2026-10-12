//! Start menu entries (freedesktop `.desktop` files) for Windows programs.

use std::path::Path;

use crate::runner::path_str;

/// Quote one argument for the `Exec` key of a desktop entry.
///
/// The Desktop Entry Specification applies the string-value escape rules
/// first and the `Exec` quoting rules second, so a literal backslash inside
/// a quoted argument must be written as four backslashes, and `%` must be
/// doubled because it introduces field codes.
pub fn exec_quote(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let arg = arg.replace('%', "%%");
    if !arg.is_empty() && !arg.contains(RESERVED) {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escape a value for a plain string key such as `Name`.
fn escape_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

pub struct Shortcut<'a> {
    pub name: &'a str,
    pub target: &'a Path,
    pub prefix: &'a str,
    pub icon: Option<&'a str>,
    pub segrun: &'a str,
}

impl Shortcut<'_> {
    pub fn render(&self) -> String {
        let exec = [
            self.segrun,
            "run",
            "--prefix",
            self.prefix,
            &path_str(self.target),
        ]
        .iter()
        .map(|a| exec_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
        let wm_class = self
            .target
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let dir = self.target.parent().map(path_str).unwrap_or_default();
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Version=1.5\n\
             Name={name}\n\
             Comment=Windows application\n\
             Exec={exec}\n\
             Path={dir}\n\
             Icon={icon}\n\
             Terminal=false\n\
             Categories=Wine;\n\
             StartupWMClass={wm_class}\n\
             X-SegLinux-Prefix={prefix}\n",
            name = escape_value(self.name),
            dir = escape_value(&dir),
            icon = escape_value(self.icon.unwrap_or("wine")),
            wm_class = escape_value(&wm_class),
            prefix = escape_value(self.prefix),
        )
    }
}

/// File name for a shortcut: `seglinux-<slug>.desktop`.
pub fn file_name(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!(
        "seglinux-{}.desktop",
        if slug.is_empty() { "app" } else { &slug }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(exec_quote("/usr/bin/segrun"), "/usr/bin/segrun");
        assert_eq!(
            exec_quote("/games/My Game/game.exe"),
            "\"/games/My Game/game.exe\""
        );
        assert_eq!(exec_quote("100%"), "100%%");
        assert_eq!(exec_quote("a$b"), "\"a\\\\$b\"");
        assert_eq!(exec_quote(r"C:\x"), "\"C:\\\\\\\\x\"");
        assert_eq!(exec_quote(""), "\"\"");
    }

    #[test]
    fn render_entry() {
        let s = Shortcut {
            name: "Notepad++",
            target: Path::new("/home/me/Apps/Notepad++/notepad++.exe"),
            prefix: "default",
            icon: None,
            segrun: "/usr/bin/segrun",
        };
        let text = s.render();
        assert!(text.contains("Name=Notepad++\n"));
        assert!(text.contains(
            "Exec=/usr/bin/segrun run --prefix default /home/me/Apps/Notepad++/notepad++.exe\n"
        ));
        assert!(text.contains("Path=/home/me/Apps/Notepad++\n"));
        assert!(text.contains("StartupWMClass=notepad++.exe\n"));
        assert_eq!(file_name("Notepad++"), "seglinux-notepad.desktop");
        assert_eq!(file_name("My Cool App 2"), "seglinux-my-cool-app-2.desktop");
    }
}
