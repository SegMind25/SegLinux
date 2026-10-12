//! What the graphical installer collects, turned into the answer file that
//! `wincraft-setup install --unattend` runs, plus the window's pure helpers.

use std::collections::BTreeSet;

use wincraft_core::init::InitSystem;
use wincraft_setup::answer::{
    Answer, Filesystem, Finish, RegionSection, UserSpec, valid_hostname, valid_username,
};
use wincraft_setup::disk::Disk;
use wincraft_setup::facts::{Facts, Firmware};
use wincraft_setup::plan::Plan;

/// File systems offered on the disk page, with their labels.
pub const FILESYSTEMS: &[(Filesystem, &str)] = &[
    (Filesystem::Ext4, "ext4 (recommended)"),
    (Filesystem::Btrfs, "btrfs (compression, subvolumes)"),
    (Filesystem::Xfs, "XFS"),
];

/// Swap choices offered on the disk page: answer file value and label.
pub const SWAP: &[(&str, &str)] = &[
    ("zram", "Compressed swap in memory (zram, recommended)"),
    ("auto", "Swap partition sized from memory"),
    ("none", "No swap"),
];

/// Everything the wizard asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    pub locale: String,
    pub keyboard: String,
    pub timezone: String,
    pub disk: Option<String>,
    pub filesystem: Filesystem,
    pub swap: String,
    pub full_name: String,
    pub user: String,
    pub password: String,
    pub confirm: String,
    pub computer: String,
    pub autologin: bool,
}

impl Default for Choices {
    fn default() -> Self {
        let region = RegionSection::default();
        Self {
            locale: region.locale,
            keyboard: region.keyboard,
            timezone: region.timezone,
            disk: None,
            filesystem: Filesystem::Ext4,
            swap: "zram".into(),
            full_name: String::new(),
            user: String::new(),
            password: String::new(),
            confirm: String::new(),
            computer: String::new(),
            autologin: false,
        }
    }
}

impl Choices {
    /// The first problem with the account page, in the order of its fields.
    pub fn account_problem(&self) -> Option<&'static str> {
        let user = self.user.trim();
        if user.is_empty() {
            return Some("Type a user name.");
        }
        if !valid_username(user) {
            return Some(
                "The user name must start with a lowercase letter and may contain lowercase letters, digits, - and _.",
            );
        }
        if self.password.is_empty() {
            return Some("Type a password.");
        }
        if self.password != self.confirm {
            return Some("The passwords do not match.");
        }
        if !valid_hostname(self.computer.trim()) {
            return Some(
                "The computer name may contain letters, digits and - (not at the start or end), up to 63 characters.",
            );
        }
        None
    }

    /// The answer file for these choices. Setup does not restart by itself:
    /// the installer offers to restart when it is done.
    pub fn answer(&self) -> Answer {
        let mut a = Answer::default();
        a.setup.finish = Finish::None;
        a.disk.device = self.disk.clone().unwrap_or_default();
        a.disk.wipe = true;
        a.disk.filesystem = self.filesystem;
        a.disk.swap = self.swap.clone();
        a.region.locale = self.locale.clone();
        a.region.keyboard = self.keyboard.clone();
        a.region.timezone = self.timezone.clone();
        a.computer.name = self.computer.trim().to_string();
        a.users = vec![UserSpec {
            name: self.user.trim().to_string(),
            full_name: self.full_name.trim().to_string(),
            password: Some(self.password.clone()),
            admin: true,
            autologin: self.autologin,
            ..Default::default()
        }];
        a
    }
}

/// The user name Windows-style Setup suggests for a full name: its first
/// word in lowercase, without accents or symbols. Empty if nothing usable.
pub fn suggest_user(full_name: &str) -> String {
    let word: String = full_name
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    if valid_username(&word) {
        word
    } else {
        String::new()
    }
}

/// The plan's summary: everything before the step-by-step listing.
pub fn summary(plan: &Plan) -> String {
    plan.to_string()
        .lines()
        .take_while(|l| !l.starts_with("=="))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Choices for a list Setup checks against (`None`: nothing known, so only
/// the default), always containing the default.
pub fn options(known: Option<&BTreeSet<String>>, default: &str) -> Vec<String> {
    let mut list: Vec<String> = known.into_iter().flatten().cloned().collect();
    if !list.iter().any(|s| s == default) {
        list.push(default.to_string());
        list.sort();
    }
    list
}

/// Languages: UTF-8 locales only, which is all Wincraft uses.
pub fn locale_options(known: Option<&BTreeSet<String>>, default: &str) -> Vec<String> {
    let utf8: BTreeSet<String> = known
        .into_iter()
        .flatten()
        .filter(|l| l.ends_with(".UTF-8"))
        .cloned()
        .collect();
    options(Some(&utf8), default)
}

/// Fraction for the progress bar.
pub fn fraction(done: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0)
    }
}

/// A made-up computer for `--demo`: two internal disks and a USB stick, the
/// Lite edition's download list and short region lists.
pub fn demo_facts() -> Facts {
    const GIB: u64 = 1024 * 1024 * 1024;
    let disk = |path: &str, size: u64, model: &str, removable: bool, transport: &str| Disk {
        path: path.into(),
        size,
        model: model.into(),
        removable,
        read_only: false,
        transport: transport.into(),
    };
    let set =
        |items: &[&str]| -> BTreeSet<String> { items.iter().map(|s| s.to_string()).collect() };
    Facts {
        firmware: Firmware::Uefi,
        init: InitSystem::OpenRc,
        disks: vec![
            disk("/dev/nvme0n1", 512 * GIB, "Samsung SSD 980", false, "nvme"),
            disk("/dev/sda", 1000 * GIB, "WDC WD10EZEX", false, "sata"),
            disk("/dev/sdb", 16 * GIB, "SanDisk Ultra", true, "usb"),
        ],
        boot_disk: Some("/dev/sdb".into()),
        image: Some("/run/artix/bootmnt/LiveOS/rootfs.img".into()),
        oem_dir: None,
        offline_packages: vec![],
        online_packages: ["steam", "wine", "firefox", "lutris", "sddm"]
            .map(String::from)
            .to_vec(),
        network: true,
        memory: 8 * GIB,
        gpus: vec![],
        timezones: Some(set(&[
            "UTC",
            "Africa/Tunis",
            "America/New_York",
            "Asia/Tokyo",
            "Europe/Berlin",
            "Europe/Paris",
        ])),
        locales: Some(set(&[
            "ar_TN.UTF-8",
            "de_DE.UTF-8",
            "en_GB.UTF-8",
            "en_US.UTF-8",
            "fr_FR.UTF-8",
            "fr_FR.ISO-8859-1",
        ])),
        keyboards: Some(set(&["ara", "de", "fr", "gb", "us"])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use wincraft_setup::ids::Sequential;
    use wincraft_setup::plan;

    fn filled() -> Choices {
        Choices {
            disk: Some("/dev/nvme0n1".into()),
            full_name: "Ada Lovelace".into(),
            user: "ada".into(),
            password: "s3cret pass".into(),
            confirm: "s3cret pass".into(),
            computer: "DESKTOP-ADA1234".into(),
            autologin: true,
            ..Default::default()
        }
    }

    #[test]
    fn account_problems_in_field_order() {
        assert_eq!(filled().account_problem(), None);
        let mut c = filled();
        c.user = " ".into();
        assert_eq!(c.account_problem(), Some("Type a user name."));
        c.user = "Ada".into();
        assert!(c.account_problem().unwrap().starts_with("The user name"));
        let mut c = filled();
        c.confirm = "other".into();
        assert_eq!(c.account_problem(), Some("The passwords do not match."));
        let mut c = filled();
        c.password.clear();
        c.confirm.clear();
        assert_eq!(c.account_problem(), Some("Type a password."));
        let mut c = filled();
        c.computer = "-bad".into();
        assert!(
            c.account_problem()
                .unwrap()
                .starts_with("The computer name")
        );
    }

    #[test]
    fn answer_file_round_trips_and_validates() {
        let answer = filled().answer();
        assert!(answer.validate().is_empty(), "{:?}", answer.validate());
        assert_eq!(answer.setup.finish, Finish::None);
        assert!(answer.disk.wipe);
        let text = toml::to_string(&answer).unwrap();
        assert_eq!(Answer::parse(&text).unwrap(), answer);
        assert!(text.contains("password = \"s3cret pass\""));
    }

    #[test]
    fn demo_computer_gives_a_plan() {
        let facts = demo_facts();
        let answer = filled().answer();
        let plan = plan::build(
            &answer,
            &facts,
            &mut Sequential::default(),
            Path::new(plan::DEFAULT_TARGET),
        )
        .unwrap();
        let text = summary(&plan);
        assert!(text.contains("/dev/nvme0n1"));
        assert!(text.contains("Download:  5 packages"));
        assert!(!text.contains("=="));
    }

    #[test]
    fn user_name_suggestions() {
        assert_eq!(suggest_user("Ada Lovelace"), "ada");
        assert_eq!(suggest_user("  Zoë Smith"), "zo");
        assert_eq!(suggest_user("42 Wallaby Way"), "");
        assert_eq!(suggest_user(""), "");
    }

    #[test]
    fn option_lists() {
        let known: BTreeSet<String> = ["fr_FR.UTF-8", "fr_FR.ISO-8859-1", "de_DE.UTF-8"]
            .map(String::from)
            .into();
        assert_eq!(
            locale_options(Some(&known), "en_US.UTF-8"),
            ["de_DE.UTF-8", "en_US.UTF-8", "fr_FR.UTF-8"]
        );
        assert_eq!(options(None, "us"), ["us"]);
        assert!((fraction(1, 4) - 0.25).abs() < f64::EPSILON);
        assert_eq!(fraction(3, 0), 0.0);
    }
}
