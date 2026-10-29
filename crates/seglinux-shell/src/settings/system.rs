//! System: about this computer, and the installed tools that manage the
//! network, sound, displays, Bluetooth and printers.
use super::widgets;
use super::{Page, State};
use crate::common;
use gtk::prelude::*;

/// `PRETTY_NAME` from os-release.
pub fn os_name(os_release: &str) -> Option<String> {
    os_release.lines().find_map(|line| {
        let value = line.strip_prefix("PRETTY_NAME=")?;
        Some(value.trim_matches('"').to_string())
    })
}

/// The processor's model name from /proc/cpuinfo, and its number of threads.
pub fn cpu(cpuinfo: &str) -> Option<String> {
    let model = cpuinfo.lines().find_map(|l| {
        l.strip_prefix("model name")?
            .split_once(':')
            .map(|(_, v)| v.trim().to_string())
    })?;
    let threads = cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count();
    Some(if threads > 1 {
        format!("{model} ({threads} threads)")
    } else {
        model
    })
}

/// Total memory from /proc/meminfo, in GiB.
pub fn memory(meminfo: &str) -> Option<String> {
    let kib: u64 = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(format!("{:.1} GiB", kib as f64 / 1024.0 / 1024.0))
}

/// Graphics vendors, from the PCI ids of the DRM devices.
fn graphics() -> Option<String> {
    let mut vendors: Vec<&str> = std::fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("card") && !name.contains('-')
        })
        .filter_map(|e| std::fs::read_to_string(e.path().join("device/vendor")).ok())
        .map(|id| match id.trim() {
            "0x10de" => "NVIDIA",
            "0x1002" => "AMD",
            "0x8086" => "Intel",
            "0x1af4" | "0x15ad" | "0x1234" => "Virtual machine",
            _ => "Other",
        })
        .collect();
    vendors.sort_unstable();
    vendors.dedup();
    (!vendors.is_empty()).then(|| vendors.join(", "))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

pub struct System {
    root: gtk::ScrolledWindow,
}

impl System {
    pub fn new() -> Self {
        let (root, page) = widgets::page();

        let about = widgets::section(&page, "About this computer", None);
        let facts = [
            ("System", os_name(&read("/etc/os-release"))),
            (
                "Kernel",
                Some(read("/proc/sys/kernel/osrelease").trim().to_string())
                    .filter(|s| !s.is_empty()),
            ),
            ("Processor", cpu(&read("/proc/cpuinfo"))),
            ("Memory", memory(&read("/proc/meminfo"))),
            ("Graphics", graphics()),
            ("Desktop session", std::env::var("XDG_CURRENT_DESKTOP").ok()),
        ];
        for (name, value) in facts {
            let text = value.as_deref().unwrap_or("Unknown");
            let label = gtk::Label::new(Some(text));
            label.set_selectable(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_max_width_chars(48);
            label.set_xalign(1.0);
            label.set_tooltip_text(Some(text));
            widgets::row(&about, name, None, &label);
        }

        let tools = widgets::section(
            &page,
            "System tools",
            Some("These tools manage their own settings and ask for your password when needed."),
        );
        for (name, icon, command, args) in [
            (
                "Network connections",
                "network-wireless-symbolic",
                "nm-connection-editor",
                vec![],
            ),
            (
                "Sound and microphone",
                "audio-volume-high-symbolic",
                "pavucontrol",
                vec![],
            ),
            (
                "Displays and resolution",
                "video-display-symbolic",
                "wdisplays",
                vec![],
            ),
            (
                "Bluetooth devices",
                "bluetooth-symbolic",
                "blueman-manager",
                vec![],
            ),
            (
                "Printers",
                "printer-symbolic",
                "system-config-printer",
                vec![],
            ),
            (
                "Steam settings and library",
                "applications-games-symbolic",
                "steam",
                vec![],
            ),
            (
                "Lutris game library",
                "applications-games-symbolic",
                "lutris",
                vec![],
            ),
        ] {
            let button = common::command_button(name, icon, command, &args);
            if !common::installed(command) {
                button.set_sensitive(false);
                button
                    .set_tooltip_text(Some(&format!("{command} is not installed on this system.")));
            }
            tools.append(&button);
        }
        if !common::installed("nm-connection-editor") && common::installed("nmtui") {
            tools.append(&common::command_button(
                "Network setup in a terminal",
                "network-wireless-symbolic",
                common::TERMINAL,
                &["nmtui"],
            ));
        }

        let files = widgets::section(
            &page,
            "Settings files",
            Some("Everything on these pages is saved in this folder; copy it to keep your setup."),
        );
        let folder = gtk::Button::with_label("Open the settings folder");
        folder.connect_clicked(|_| {
            let dir = seglinux_core::xdg::seglinux_config();
            let _ = std::fs::create_dir_all(&dir);
            common::open_path(&dir);
        });
        widgets::row(
            &files,
            &seglinux_core::xdg::seglinux_config().display().to_string(),
            None,
            &folder,
        );

        Self { root }
    }
}

impl Page for System {
    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    fn load(&self, _: &State) {}
    fn store(&self, _: &mut State) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_system_facts() {
        assert_eq!(
            os_name("NAME=SegLinux\nPRETTY_NAME=\"SegLinux (rolling)\"\n").as_deref(),
            Some("SegLinux (rolling)")
        );
        let cpuinfo = "processor\t: 0\nmodel name\t: AMD Ryzen 5 5600X 6-Core Processor\nprocessor\t: 1\nmodel name\t: AMD Ryzen 5 5600X 6-Core Processor\n";
        assert_eq!(
            cpu(cpuinfo).as_deref(),
            Some("AMD Ryzen 5 5600X 6-Core Processor (2 threads)")
        );
        assert_eq!(
            memory("MemTotal:       16314200 kB\n").as_deref(),
            Some("15.6 GiB")
        );
        assert_eq!(memory(""), None);
    }
}
