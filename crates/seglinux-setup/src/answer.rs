//! The answer file, `autounattend.toml`.
//!
//! It plays the role of Windows' `autounattend.xml`: every question Setup
//! would ask can be answered in advance, which makes installation fully
//! unattended. Sections map onto the Windows configuration passes:
//!
//! | Section        | Windows equivalent (pass / component)                  |
//! |----------------|--------------------------------------------------------|
//! | `[disk]`       | windowsPE / Microsoft-Windows-Setup `DiskConfiguration`  |
//! | `[image]`      | windowsPE / Microsoft-Windows-Setup `ImageInstall`       |
//! | `[oem]`        | offlineServicing, `$OEM$` folders                      |
//! | `[region]`     | specialize / Microsoft-Windows-International-Core      |
//! | `[computer]`   | specialize / Microsoft-Windows-Shell-Setup `ComputerName`|
//! | `[[users]]`    | oobeSystem / `UserAccounts`                             |
//! | `[[run]]`      | specialize `RunSynchronous`, `SetupComplete.cmd`        |

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Name Setup looks for, like `autounattend.xml` on Windows media.
pub const ANSWER_FILE_NAME: &str = "autounattend.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Answer {
    pub setup: SetupSection,
    pub disk: DiskSection,
    pub image: ImageSection,
    pub oem: OemSection,
    pub region: RegionSection,
    pub computer: ComputerSection,
    pub users: Vec<UserSpec>,
    pub root: RootSection,
    pub bootloader: BootloaderSection,
    pub performance: PerformanceSection,
    pub drivers: DriversSection,
    pub services: ServicesSection,
    #[serde(rename = "run")]
    pub commands: Vec<RunCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SetupSection {
    /// Start installing as soon as the live session boots and finds this
    /// file (like an `autounattend.xml` on the install media).
    pub auto_start: bool,
    /// What to do when installation is complete.
    pub finish: Finish,
}

impl Default for SetupSection {
    fn default() -> Self {
        Self {
            auto_start: false,
            finish: Finish::Reboot,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Finish {
    Reboot,
    Poweroff,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiskSection {
    /// Whole disk to install to (`/dev/sda`, `/dev/nvme0n1`), or `auto` for
    /// the largest suitable internal disk.
    pub device: String,
    /// Must be `true`: the whole disk is erased (`WillWipeDisk`).
    pub wipe: bool,
    pub filesystem: Filesystem,
    /// `zram` (compressed swap in RAM, the default), a swap partition size
    /// (`8GiB`), `auto` (a partition sized from RAM) or `none`.
    pub swap: String,
    /// EFI system partition size.
    pub esp_size: String,
}

impl Default for DiskSection {
    fn default() -> Self {
        Self {
            device: String::new(),
            wipe: false,
            filesystem: Filesystem::Ext4,
            swap: "zram".into(),
            esp_size: "512MiB".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filesystem {
    Ext4,
    Btrfs,
    Xfs,
}

impl fmt::Display for Filesystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Filesystem::Ext4 => "ext4",
            Filesystem::Btrfs => "btrfs",
            Filesystem::Xfs => "xfs",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImageSection {
    /// The system image (the squashfs `rootfs.img` on the install media),
    /// or `auto` to find it. The counterpart of `sources\install.wim`.
    pub source: String,
}

impl Default for ImageSection {
    fn default() -> Self {
        Self {
            source: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OemSection {
    /// Folder copied over the new system (`$OEM$`), or `auto` for
    /// `seglinux/oem` on the install media.
    pub path: String,
    /// Folder of `.pkg.tar.zst` packages installed offline, or `auto` for
    /// `seglinux/packages` on the install media.
    pub packages: String,
}

impl Default for OemSection {
    fn default() -> Self {
        Self {
            path: "auto".into(),
            packages: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RegionSection {
    pub locale: String,
    /// X keyboard layout (`us`, `fr`, `de`, ...), also used for the console.
    pub keyboard: String,
    pub timezone: String,
    /// `local` keeps the hardware clock in local time like Windows does,
    /// which avoids a wrong clock when dual-booting.
    pub hardware_clock: HardwareClock,
}

impl Default for RegionSection {
    fn default() -> Self {
        Self {
            locale: "en_US.UTF-8".into(),
            keyboard: "us".into(),
            timezone: "UTC".into(),
            hardware_clock: HardwareClock::Utc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HardwareClock {
    Utc,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ComputerSection {
    /// Host name, or `auto` for a Windows-style `DESKTOP-XXXXXXX`.
    pub name: String,
}

impl Default for ComputerSection {
    fn default() -> Self {
        Self {
            name: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserSpec {
    pub name: String,
    pub full_name: String,
    /// Plain-text password. Prefer `password_hash` in files you keep.
    pub password: Option<String>,
    /// crypt(3) hash, e.g. from `openssl passwd -6`.
    pub password_hash: Option<String>,
    /// Member of `wheel`, i.e. allowed to use sudo ("Administrator").
    pub admin: bool,
    pub autologin: bool,
    /// Extra groups on top of SegLinux's defaults.
    pub groups: Vec<String>,
}

impl Default for UserSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            full_name: String::new(),
            password: None,
            password_hash: None,
            admin: true,
            autologin: false,
            groups: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RootSection {
    /// Without a password the root account is locked, like the built-in
    /// Administrator account on Windows; administrators use sudo.
    pub password: Option<String>,
    pub password_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BootloaderSection {
    /// Name of the UEFI boot entry (what bcdboot calls the description).
    pub id: String,
    /// Also install to `\EFI\BOOT\BOOTX64.EFI`, the fallback path firmware
    /// boots when it has no boot entry (removable drives, some laptops).
    pub removable: bool,
    /// Add other operating systems (Windows) to the boot menu.
    pub os_prober: bool,
}

impl Default for BootloaderSection {
    fn default() -> Self {
        Self {
            id: "SegLinux".into(),
            removable: true,
            os_prober: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PerformanceSection {
    /// OpenRC: start services in parallel (`rc_parallel`) for a faster boot.
    pub parallel_boot: bool,
    /// Kernel parameters added to SegLinux's defaults
    /// (`loglevel=3 quiet nowatchdog nmi_watchdog=0`).
    pub kernel_parameters: Vec<String>,
}

impl Default for PerformanceSection {
    fn default() -> Self {
        Self {
            parallel_boot: true,
            kernel_parameters: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DriversSection {
    /// `auto` installs NVIDIA's driver (open kernel modules) when a GeForce
    /// GTX 16 / RTX or newer GPU is present and the network is reachable.
    /// Older NVIDIA GPUs keep the open-source nouveau/NVK driver.
    pub nvidia: DriverChoice,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverChoice {
    #[default]
    Auto,
    Never,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServicesSection {
    pub enable: Vec<String>,
    pub disable: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCommand {
    pub pass: CommandPass,
    /// Shell command run inside the new system.
    pub command: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandPass {
    /// After the system is configured, before users are created.
    Specialize,
    /// At the very end, like `SetupComplete.cmd`.
    SetupComplete,
}

/// Services enabled on every installation (missing ones are skipped).
pub const DEFAULT_SERVICES: &[&str] = &[
    "dbus",
    "elogind",
    "binfmt",
    "sysctl",
    "modules",
    "NetworkManager",
    "sddm",
    "bluetoothd",
    "cupsd",
    "cronie",
    "syslog-ng",
    "ntpd",
    "acpid",
    "power-profiles-daemon",
];

/// Groups every user joins (missing ones are skipped).
pub const DEFAULT_GROUPS: &[&str] = &[
    "users", "audio", "video", "input", "storage", "optical", "network", "lp", "scanner", "power",
    "gamemode",
];

const RESERVED_USERS: &[&str] = &[
    "root",
    "bin",
    "daemon",
    "mail",
    "ftp",
    "http",
    "nobody",
    "dbus",
    "polkitd",
    "sddm",
    "avahi",
    "git",
    "admin",
    "administrator",
];

impl Answer {
    pub fn load(path: &Path) -> Result<Answer> {
        let text =
            fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid answer file {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Answer> {
        Ok(toml::from_str(text)?)
    }

    /// Checks that do not depend on the machine. Returns every problem, so a
    /// broken answer file can be fixed in one go.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.disk.device.is_empty() {
            errors.push("[disk] device is required (e.g. \"/dev/sda\" or \"auto\")".into());
        } else if self.disk.device != "auto" && !self.disk.device.starts_with("/dev/") {
            errors.push(format!(
                "[disk] device `{}` must be a /dev path or \"auto\"",
                self.disk.device
            ));
        }
        if !self.disk.wipe {
            errors.push(
                "[disk] wipe = true is required: SegLinux Setup erases the whole disk".into(),
            );
        }
        if let Err(e) = parse_swap(&self.disk.swap) {
            errors.push(format!("[disk] swap: {e}"));
        }
        match parse_size(&self.disk.esp_size) {
            Ok(size) if size < 200 * MIB => {
                errors.push("[disk] esp_size must be at least 200MiB".into())
            }
            Ok(_) => {}
            Err(e) => errors.push(format!("[disk] esp_size: {e}")),
        }
        if !valid_locale_name(&self.region.locale) {
            errors.push(format!(
                "[region] locale `{}` should look like en_US.UTF-8",
                self.region.locale
            ));
        }
        if !valid_simple_name(&self.region.keyboard) {
            errors.push(format!(
                "[region] keyboard `{}` is not a layout name",
                self.region.keyboard
            ));
        }
        if !valid_timezone(&self.region.timezone) {
            errors.push(format!(
                "[region] timezone `{}` should look like Europe/Paris or UTC",
                self.region.timezone
            ));
        }
        if self.computer.name != "auto" && !valid_hostname(&self.computer.name) {
            errors.push(format!(
                "[computer] name `{}`: use 1-63 letters, digits and hyphens",
                self.computer.name
            ));
        }
        if self.users.is_empty() {
            errors.push("at least one [[users]] entry is required".into());
        }
        for (i, u) in self.users.iter().enumerate() {
            let at = format!("[[users]] #{}", i + 1);
            if !valid_username(&u.name) {
                errors.push(format!(
                    "{at}: name `{}` must start with a lowercase letter and use a-z, 0-9, - and _ (max 32)",
                    u.name
                ));
            } else if RESERVED_USERS.contains(&u.name.as_str()) {
                errors.push(format!("{at}: name `{}` is reserved", u.name));
            }
            match (&u.password, &u.password_hash) {
                (Some(_), Some(_)) => {
                    errors.push(format!("{at}: set password or password_hash, not both"))
                }
                (None, None) => {
                    errors.push(format!("{at}: a password (or password_hash) is required"))
                }
                (Some(p), None) if p.is_empty() => {
                    errors.push(format!("{at}: the password must not be empty"))
                }
                (None, Some(h)) if !h.starts_with('$') => errors.push(format!(
                    "{at}: password_hash must be a crypt(3) hash such as $6$..."
                )),
                _ => {}
            }
            if u.full_name.contains([':', '\n']) {
                errors.push(format!("{at}: full_name must not contain ':' or newlines"));
            }
            for g in &u.groups {
                if !valid_simple_name(g) {
                    errors.push(format!("{at}: group `{g}` is not a valid name"));
                }
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for u in &self.users {
            if !seen.insert(&u.name) {
                errors.push(format!("user `{}` is listed twice", u.name));
            }
        }
        if self.users.iter().filter(|u| u.autologin).count() > 1 {
            errors.push("only one user can have autologin = true".into());
        }
        let has_admin = self.users.iter().any(|u| u.admin);
        let root_unlocked = self.root.password.as_deref().is_some_and(|p| !p.is_empty())
            || self.root.password_hash.is_some();
        if !self.users.is_empty() && !has_admin && !root_unlocked {
            errors.push(
                "no user has admin = true and root is locked: nobody could administer the system"
                    .into(),
            );
        }
        for p in &self.performance.kernel_parameters {
            if !valid_kernel_parameter(p) {
                errors.push(format!(
                    "[performance] kernel parameter `{p}` must be one word without quotes"
                ));
            }
        }
        if !valid_simple_name(&self.bootloader.id) {
            errors.push(format!(
                "[bootloader] id `{}` must be a simple name",
                self.bootloader.id
            ));
        }
        for s in self.services.enable.iter().chain(&self.services.disable) {
            if !valid_simple_name(s) {
                errors.push(format!("[services] `{s}` is not a valid service name"));
            }
        }
        for c in &self.commands {
            if c.command.trim().is_empty() {
                errors.push("[[run]] command must not be empty".into());
            }
        }
        errors
    }

    pub fn validate_or_bail(&self) -> Result<()> {
        let errors = self.validate();
        if !errors.is_empty() {
            bail!(
                "the answer file has problems:\n  - {}",
                errors.join("\n  - ")
            );
        }
        Ok(())
    }
}

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;

/// Parse `512MiB`, `4GiB`, `4G`, `2048M`, `1T`. Unit-less values are bytes.
/// Decimal-looking units (`MB`, `GB`) are treated as binary, the way
/// Windows labels sizes.
pub fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: u64 = num
        .parse()
        .with_context(|| format!("`{s}` is not a size"))?;
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => MIB,
        "g" | "gb" | "gib" => GIB,
        "t" | "tb" | "tib" => 1024 * GIB,
        other => bail!("unknown size unit `{other}` in `{s}`"),
    };
    n.checked_mul(mult)
        .with_context(|| format!("`{s}` is too large"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swap {
    None,
    /// Compressed swap in RAM.
    Zram,
    /// A swap partition sized from the installed memory.
    Auto,
    Size(u64),
}

pub fn parse_swap(s: &str) -> Result<Swap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" | "0" | "off" | "" => Ok(Swap::None),
        "zram" => Ok(Swap::Zram),
        "auto" => Ok(Swap::Auto),
        _ => {
            let size = parse_size(s)?;
            if size < 64 * MIB {
                bail!("swap must be at least 64MiB (or \"none\")");
            }
            Ok(Swap::Size(size))
        }
    }
}

/// Swap size for `auto`, the counterpart of a "system managed" page file:
/// as much as the installed memory, between 2 and 8 GiB.
pub fn auto_swap_size(memory: u64) -> u64 {
    let rounded = memory.div_ceil(GIB) * GIB;
    rounded.clamp(2 * GIB, 8 * GIB)
}

pub fn valid_username(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn valid_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn valid_simple_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c))
}

fn valid_kernel_parameter(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.chars()
            .all(|c| c.is_ascii_graphic() && !"\"'\\`$".contains(c))
}

fn valid_locale_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-@".contains(c))
}

fn valid_timezone(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('/')
        && !s
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_-+".contains(c))
}

/// Where an answer file was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerSource {
    CommandLine,
    KernelCmdline,
    InstallMedia,
    RemovableDrive(String),
    LiveImage,
}

impl fmt::Display for AnswerSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnswerSource::CommandLine => f.write_str("command line"),
            AnswerSource::KernelCmdline => f.write_str("kernel command line (seglinux.unattend=)"),
            AnswerSource::InstallMedia => f.write_str("root of the install media"),
            AnswerSource::RemovableDrive(dev) => write!(f, "removable drive {dev}"),
            AnswerSource::LiveImage => f.write_str("/etc/seglinux in the live image"),
        }
    }
}

/// `seglinux.unattend=<path>` from a kernel command line.
pub fn path_from_cmdline(cmdline: &str) -> Option<PathBuf> {
    cmdline
        .split_whitespace()
        .find_map(|arg| arg.strip_prefix("seglinux.unattend="))
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// A commented example answer file, printed by `seglinux-setup example`.
pub const EXAMPLE: &str = include_str!("../autounattend.example.toml");

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> Answer {
        Answer::parse(
            r#"
            [disk]
            device = "/dev/vda"
            wipe = true

            [[users]]
            name = "alice"
            password = "secret"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn minimal_file_is_valid_and_uses_defaults() {
        let a = minimal();
        assert_eq!(a.validate(), Vec::<String>::new());
        assert_eq!(a.region.locale, "en_US.UTF-8");
        assert_eq!(a.disk.filesystem, Filesystem::Ext4);
        assert_eq!(a.setup.finish, Finish::Reboot);
        assert!(a.users[0].admin);
        assert_eq!(a.computer.name, "auto");
        assert_eq!(a.disk.swap, "zram");
        assert!(a.performance.parallel_boot);
        assert_eq!(a.drivers.nvidia, DriverChoice::Auto);
    }

    #[test]
    fn example_file_parses_and_validates() {
        let a = Answer::parse(EXAMPLE).unwrap();
        assert_eq!(a.validate(), Vec::<String>::new());
        assert_eq!(a.commands.len(), 2);
    }

    #[test]
    fn empty_file_lists_every_missing_piece() {
        let errors = Answer::parse("").unwrap().validate();
        assert!(errors.iter().any(|e| e.contains("device is required")));
        assert!(errors.iter().any(|e| e.contains("wipe = true")));
        assert!(errors.iter().any(|e| e.contains("[[users]]")));
    }

    #[test]
    fn bad_values_are_reported() {
        let mut a = minimal();
        a.users[0].name = "Bob".into();
        a.users.push(UserSpec {
            name: "root".into(),
            password_hash: Some("plaintext".into()),
            ..Default::default()
        });
        a.computer.name = "-bad-".into();
        a.region.timezone = "../etc/passwd".into();
        a.disk.swap = "12 parsecs".into();
        a.performance.kernel_parameters = vec!["quiet\" init=/bin/sh".into()];
        let errors = a.validate();
        assert!(errors.iter().any(|e| e.contains("`Bob`")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("reserved")));
        assert!(errors.iter().any(|e| e.contains("crypt(3)")));
        assert!(errors.iter().any(|e| e.contains("[computer]")));
        assert!(errors.iter().any(|e| e.contains("timezone")));
        assert!(errors.iter().any(|e| e.contains("swap")));
        assert!(errors.iter().any(|e| e.contains("kernel parameter")));
    }

    #[test]
    fn someone_must_be_able_to_administer() {
        let mut a = minimal();
        a.users[0].admin = false;
        assert!(a.validate().iter().any(|e| e.contains("administer")));
        a.root.password = Some("toor".into());
        assert!(a.validate().is_empty());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(Answer::parse("[disk]\ndevise = \"/dev/sda\"").is_err());
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("512MiB").unwrap(), 512 * MIB);
        assert_eq!(parse_size("4G").unwrap(), 4 * GIB);
        assert_eq!(parse_size("4 GB").unwrap(), 4 * GIB);
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert!(parse_size("lots").is_err());
        assert!(parse_size("5 furlongs").is_err());
        assert_eq!(parse_swap("none").unwrap(), Swap::None);
        assert_eq!(parse_swap("zram").unwrap(), Swap::Zram);
        assert_eq!(parse_swap("auto").unwrap(), Swap::Auto);
        assert_eq!(parse_swap("2GiB").unwrap(), Swap::Size(2 * GIB));
        assert!(parse_swap("1MiB").is_err());
        assert_eq!(auto_swap_size(GIB), 2 * GIB);
        assert_eq!(auto_swap_size(4 * GIB - 1), 4 * GIB);
        assert_eq!(auto_swap_size(64 * GIB), 8 * GIB);
    }

    #[test]
    fn names() {
        assert!(valid_username("alice"));
        assert!(valid_username("_svc-1"));
        assert!(!valid_username("1alice"));
        assert!(!valid_username("Alice"));
        assert!(valid_hostname("DESKTOP-ABC1234"));
        assert!(!valid_hostname("my_pc"));
        assert!(valid_timezone("America/Argentina/Buenos_Aires"));
        assert!(valid_timezone("Etc/GMT+5"));
        assert!(!valid_timezone("/etc/passwd"));
        assert!(!valid_timezone("Europe/../../x"));
    }

    #[test]
    fn kernel_cmdline() {
        assert_eq!(
            path_from_cmdline("BOOT_IMAGE=/boot/vmlinuz quiet seglinux.unattend=/run/x.toml"),
            Some(PathBuf::from("/run/x.toml"))
        );
        assert_eq!(path_from_cmdline("quiet seglinux.unattend="), None);
    }
}
