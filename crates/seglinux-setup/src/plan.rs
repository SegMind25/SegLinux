//! The installation plan: answer file + machine facts -> ordered actions.
//!
//! The plan is computed completely before anything is touched, grouped in
//! the same configuration passes as Windows Setup, so it can be shown in a
//! dry run, logged, and tested.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use seglinux_core::cmd::{Cmd, shell_quote};
use seglinux_core::init::InitSystem;

use crate::answer::{
    Answer, CommandPass, DEFAULT_GROUPS, DEFAULT_SERVICES, DriverChoice, Filesystem, HardwareClock,
    MIB, Swap, auto_swap_size, parse_size, parse_swap,
};
use crate::disk::{self, Disk, Layout, Role, human_size};
use crate::facts::{Facts, Firmware, list_packages};
use crate::ids::IdSource;

/// Where the new system is mounted during installation.
pub const DEFAULT_TARGET: &str = "/mnt/seglinux";

/// Scripts run at the end of setup, like `%WINDIR%\Setup\Scripts\SetupComplete.cmd`.
pub const SETUPCOMPLETE_DIR: &str = "/etc/seglinux/setupcomplete.d";

/// What the sign-in screen (seglinux-greeter) preselects: the account and
/// desktop session used last.
pub const GREETER_STATE: &str = "/var/lib/seglinux-greeter/state.toml";

/// greetd, the sign-in service, and SegLinux's configuration of it (the
/// image has it already; Setup adds automatic sign-in).
pub const GREETD_CONFIG: &str = "/etc/greetd/config.toml";
pub const GREETD_DEFAULTS: &str = include_str!("../../../data/greeter/greetd.toml");

/// Reinstalls the packages whose files the Lite image leaves out
/// (data/system/restore-trimmed-files).
pub const RESTORE_TRIMMED_FILES: &str = "/usr/lib/seglinux/restore-trimmed-files";

/// The boot menu theme, installed by the seglinux package.
pub const GRUB_THEME: &str = "/usr/share/grub/themes/seglinux/theme.txt";

/// GRUB's name for the system: it shows "<distributor> Linux" in its
/// menu, so "SegLinux" is given as "Seg" to read "Seg Linux" rather than
/// "SegLinux Linux".
pub fn grub_distributor(id: &str) -> &str {
    match id.trim_end().strip_suffix("Linux") {
        Some(name) if !name.trim().is_empty() => name.trim_end(),
        _ => id,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    WindowsPe,
    OfflineServicing,
    Specialize,
    OobeSystem,
    SetupComplete,
}

impl Pass {
    /// The Windows configuration pass this phase corresponds to.
    pub fn windows_name(&self) -> &'static str {
        match self {
            Pass::WindowsPe => "windowsPE",
            Pass::OfflineServicing => "offlineServicing",
            Pass::Specialize => "specialize",
            Pass::OobeSystem => "oobeSystem",
            Pass::SetupComplete => "SetupComplete",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Pass::WindowsPe => "Preparing the disk and copying SegLinux files",
            Pass::OfflineServicing => "Adding OEM files and packages",
            Pass::Specialize => "Configuring SegLinux for this computer",
            Pass::OobeSystem => "Setting up user accounts",
            Pass::SetupComplete => "Finishing up",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Secret {
    Plain(String),
    Hash(String),
    Lock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A command on the live system.
    Host {
        cmd: Cmd,
        may_fail: bool,
    },
    /// A command inside the new system.
    Chroot {
        cmd: Cmd,
        may_fail: bool,
    },
    /// Paths of the remaining actions are inside the new system.
    WriteFile {
        path: String,
        contents: String,
        mode: u32,
    },
    Symlink {
        target: String,
        link: String,
    },
    /// Set `KEY=value` in a shell-style config file, keeping the rest.
    SetKey {
        path: String,
        key: String,
        value: String,
    },
    EnableService(String),
    DisableService(String),
    AddUser {
        name: String,
        full_name: String,
        groups: Vec<String>,
    },
    SetPassword {
        user: String,
        secret: Secret,
    },
    /// Run every executable file in a directory, in name order.
    RunScripts {
        dir: String,
    },
    SaveLogs {
        dir: String,
    },
}

impl Action {
    fn host(cmd: Cmd) -> Action {
        Action::Host {
            cmd,
            may_fail: false,
        }
    }
    fn host_optional(cmd: Cmd) -> Action {
        Action::Host {
            cmd,
            may_fail: true,
        }
    }
    fn chroot(cmd: Cmd) -> Action {
        Action::Chroot {
            cmd,
            may_fail: false,
        }
    }
    fn chroot_optional(cmd: Cmd) -> Action {
        Action::Chroot {
            cmd,
            may_fail: true,
        }
    }
    fn write(path: &str, contents: impl Into<String>) -> Action {
        Action::WriteFile {
            path: path.into(),
            contents: contents.into(),
            mode: 0o644,
        }
    }
    fn set_key(path: &str, key: &str, value: impl Into<String>) -> Action {
        Action::SetKey {
            path: path.into(),
            key: key.into(),
            value: value.into(),
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let optional = |may_fail: &bool| if *may_fail { "  # may fail" } else { "" };
        match self {
            Action::Host { cmd, may_fail } => write!(f, "$ {cmd}{}", optional(may_fail)),
            Action::Chroot { cmd, may_fail } => {
                write!(f, "(new system) $ {cmd}{}", optional(may_fail))
            }
            Action::WriteFile {
                path,
                contents,
                mode,
            } => {
                write!(f, "write {path} (mode {mode:o})")?;
                for line in contents.lines() {
                    write!(f, "\n    | {line}")?;
                }
                Ok(())
            }
            Action::Symlink { target, link } => write!(f, "link {link} -> {target}"),
            Action::SetKey { path, key, value } => write!(f, "set {key}={value} in {path}"),
            Action::EnableService(s) => write!(f, "enable service {s}"),
            Action::DisableService(s) => write!(f, "disable service {s}"),
            Action::AddUser {
                name,
                full_name,
                groups,
            } => write!(
                f,
                "create user {name} ({}) in groups {}",
                if full_name.is_empty() {
                    name
                } else {
                    full_name
                },
                groups.join(",")
            ),
            Action::SetPassword { user, secret } => match secret {
                Secret::Lock => write!(f, "lock the password of {user}"),
                _ => write!(f, "set the password of {user}"),
            },
            Action::RunScripts { dir } => write!(f, "run the scripts in {dir}"),
            Action::SaveLogs { dir } => write!(f, "save setup logs to {dir}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Step {
    pub description: String,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone)]
pub struct Phase {
    pub pass: Pass,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub disk: Disk,
    pub layout: Layout,
    pub hostname: String,
    pub image: PathBuf,
    pub firmware: Firmware,
    pub init: InitSystem,
    pub users: Vec<String>,
    /// Packages downloaded during Setup (Lite edition).
    pub online_packages: usize,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub target: PathBuf,
    pub summary: Summary,
    pub phases: Vec<Phase>,
}

impl Plan {
    pub fn step_count(&self) -> usize {
        self.phases.iter().map(|p| p.steps.len()).sum()
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = &self.summary;
        writeln!(f, "SegLinux Setup plan")?;
        writeln!(
            f,
            "  Disk:      {} (all data will be erased)",
            s.disk.label()
        )?;
        for p in &s.layout.partitions {
            let size = p
                .size
                .map(human_size)
                .unwrap_or_else(|| "rest of disk".into());
            let what = match p.role {
                Role::Esp => "EFI system partition (FAT32)".to_string(),
                Role::BiosBoot => "BIOS boot partition".to_string(),
                Role::Swap => "swap".to_string(),
                Role::Root => format!("SegLinux ({})", s.layout.filesystem),
            };
            writeln!(f, "    {:<16} {:<14} {what}", p.path, size)?;
        }
        if let Some(size) = s.layout.zram {
            writeln!(
                f,
                "  Swap:      {} compressed in RAM (zram)",
                human_size(size)
            )?;
        }
        writeln!(f, "  Image:     {}", s.image.display())?;
        writeln!(f, "  Firmware:  {}, init system: {}", s.firmware, s.init)?;
        writeln!(f, "  Computer:  {}", s.hostname)?;
        writeln!(f, "  Users:     {}", s.users.join(", "))?;
        if s.online_packages > 0 {
            writeln!(
                f,
                "  Download:  {} packages (apps, drivers, games); needs a network connection",
                s.online_packages
            )?;
        }
        for phase in &self.phases {
            writeln!(
                f,
                "\n== {} ({})",
                phase.pass.title(),
                phase.pass.windows_name()
            )?;
            for step in &phase.steps {
                writeln!(f, " * {}", step.description)?;
                for action in &step.actions {
                    for line in action.to_string().lines() {
                        writeln!(f, "     {line}")?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn p(target: &Path, rel: &str) -> String {
    seglinux_core::in_root(target, rel)
        .to_string_lossy()
        .into_owned()
}

pub fn build(
    answer: &Answer,
    facts: &Facts,
    ids: &mut dyn IdSource,
    target: &Path,
) -> Result<Plan> {
    answer.validate_or_bail()?;

    // --- Resolve the disk --------------------------------------------------
    let disk = if answer.disk.device == "auto" {
        match facts.auto_disk() {
            Some(d) => d.clone(),
            None => bail!(
                "no internal disk of at least {} was found for device = \"auto\"",
                human_size(disk::MIN_DISK_SIZE)
            ),
        }
    } else {
        let Some(d) = facts.disks.iter().find(|d| d.path == answer.disk.device) else {
            let available: Vec<String> = facts.disks.iter().map(Disk::label).collect();
            bail!(
                "disk {} was not found. Available disks:\n  {}",
                answer.disk.device,
                available.join("\n  ")
            );
        };
        d.clone()
    };
    if facts.boot_disk.as_deref() == Some(disk.path.as_str()) {
        bail!(
            "{} holds the running install media and cannot be installed to",
            disk.path
        );
    }
    if disk.read_only {
        bail!("{} is read-only", disk.path);
    }
    if disk.size < disk::MIN_DISK_SIZE {
        bail!(
            "{} is {}; SegLinux needs at least {}",
            disk.path,
            human_size(disk.size),
            human_size(disk::MIN_DISK_SIZE)
        );
    }

    let swap_choice = parse_swap(&answer.disk.swap)?;
    let swap = match swap_choice {
        Swap::None | Swap::Zram => None,
        Swap::Auto => Some(auto_swap_size(facts.memory)),
        Swap::Size(n) => Some(n),
    };
    let esp_size = parse_size(&answer.disk.esp_size)?;
    let mut layout = Layout::new(&disk.path, answer.disk.filesystem, esp_size, swap, ids);
    layout.zram = (swap_choice == Swap::Zram).then(|| auto_swap_size(facts.memory));
    let root_size = disk.size.saturating_sub(layout.fixed_size());
    if root_size < disk::MIN_ROOT_SIZE {
        bail!(
            "only {} would be left for SegLinux on {} (needs {}); reduce swap or use a bigger disk",
            human_size(root_size),
            disk.path,
            human_size(disk::MIN_ROOT_SIZE)
        );
    }

    // --- Resolve region settings --------------------------------------------
    let region = &answer.region;
    if let Some(known) = &facts.timezones
        && !known.contains(&region.timezone)
    {
        bail!("unknown time zone `{}`", region.timezone);
    }
    if let Some(known) = &facts.locales
        && !known.contains(&region.locale)
    {
        bail!(
            "unknown locale `{}` (see /usr/share/i18n/SUPPORTED)",
            region.locale
        );
    }
    if let Some(known) = &facts.keyboards
        && !known.contains(&region.keyboard)
    {
        bail!("unknown keyboard layout `{}`", region.keyboard);
    }

    let image = match answer.image.source.as_str() {
        "auto" => match &facts.image {
            Some(i) => i.clone(),
            None => bail!(
                "the system image (LiveOS/rootfs.img) was not found: is this the SegLinux live media?"
            ),
        },
        other => PathBuf::from(other),
    };
    let oem_dir = match answer.oem.path.as_str() {
        "auto" => facts.oem_dir.clone(),
        "" | "none" => None,
        other => Some(PathBuf::from(other)),
    };
    let packages = match answer.oem.packages.as_str() {
        "auto" => facts.offline_packages.clone(),
        "" | "none" => Vec::new(),
        other => list_packages(Path::new(other)),
    };
    let hostname = if answer.computer.name == "auto" {
        ids.computer_name()
    } else {
        answer.computer.name.clone()
    };

    let init = facts.init;
    let openrc = init == InitSystem::OpenRc;
    let t = |rel: &str| p(target, rel);
    let mut phases = Vec::new();

    // --- windowsPE: disk and image ----------------------------------------------
    let mut pe = Vec::new();
    pe.push(Step {
        description: "Checking the system image".into(),
        actions: vec![Action::host(
            Cmd::new("test").args(["-r", &image.to_string_lossy()]),
        )],
    });
    pe.push(Step {
        description: format!("Releasing {}", disk.path),
        actions: vec![
            Action::host_optional(Cmd::new("umount").args(["-R", &target.to_string_lossy()])),
            Action::host_optional(Cmd::new("swapoff").arg("-a")),
        ],
    });
    pe.push(Step {
        description: format!("Creating partitions on {}", disk.path),
        actions: vec![
            Action::host(Cmd::new("wipefs").args(["--all", "--force", &disk.path])),
            Action::host(
                Cmd::new("sfdisk")
                    .args([
                        "--wipe",
                        "always",
                        "--wipe-partitions",
                        "always",
                        &disk.path,
                    ])
                    .stdin(layout.sfdisk_script()),
            ),
            Action::host_optional(Cmd::new("udevadm").arg("settle")),
        ],
    });

    let root = layout.root();
    let root_uuid = root.uuid.clone().unwrap_or_default();
    let esp = layout.esp();
    let mut format = vec![Action::host(Cmd::new("mkfs.fat").args([
        "-F",
        "32",
        "-n",
        "SYSTEM",
        "-i",
        esp.uuid.as_deref().unwrap_or_default(),
        &esp.path,
    ]))];
    if let Some(sw) = layout.get(Role::Swap) {
        format.push(Action::host(Cmd::new("mkswap").args([
            "-L",
            "swap",
            "-U",
            sw.uuid.as_deref().unwrap_or_default(),
            &sw.path,
        ])));
    }
    format.push(Action::host(match answer.disk.filesystem {
        Filesystem::Ext4 => {
            Cmd::new("mkfs.ext4").args(["-F", "-L", "SegLinux", "-U", &root_uuid, &root.path])
        }
        Filesystem::Btrfs => {
            Cmd::new("mkfs.btrfs").args(["-f", "-L", "SegLinux", "-U", &root_uuid, &root.path])
        }
        Filesystem::Xfs => Cmd::new("mkfs.xfs").args([
            "-f",
            "-L",
            "SegLinux",
            "-m",
            &format!("uuid={root_uuid}"),
            &root.path,
        ]),
    }));
    pe.push(Step {
        description: "Formatting partitions".into(),
        actions: format,
    });

    let target_s = target.to_string_lossy().into_owned();
    let mut mount = vec![Action::host(Cmd::new("mkdir").args(["-p", &target_s]))];
    if answer.disk.filesystem == Filesystem::Btrfs {
        mount.extend([
            Action::host(Cmd::new("mount").args([&root.path, &target_s])),
            Action::host(Cmd::new("btrfs").args(["subvolume", "create", &t("/@")])),
            Action::host(Cmd::new("btrfs").args(["subvolume", "create", &t("/@home")])),
            Action::host(Cmd::new("umount").arg(&target_s)),
            Action::host(Cmd::new("mount").args([
                "-o",
                layout.root_options(),
                &root.path,
                &target_s,
            ])),
            Action::host(Cmd::new("mkdir").args(["-p", &t("/home")])),
            Action::host(Cmd::new("mount").args([
                "-o",
                "rw,noatime,compress=zstd:1,subvol=/@home",
                &root.path,
                &t("/home"),
            ])),
        ]);
    } else {
        mount.push(Action::host(
            Cmd::new("mount").args([&root.path, &target_s]),
        ));
    }
    pe.push(Step {
        description: "Mounting the new system".into(),
        actions: mount,
    });
    pe.push(Step {
        description: "Copying SegLinux files (this takes a few minutes)".into(),
        actions: vec![Action::host(Cmd::new("unsquashfs").args([
            "-f",
            "-no-progress",
            "-d",
            &target_s,
            &image.to_string_lossy(),
        ]))],
    });
    let mut chroot_mounts = vec![
        Action::host(Cmd::new("mkdir").args(["-p", &t("/boot/efi")])),
        Action::host(Cmd::new("mount").args([&esp.path, &t("/boot/efi")])),
        Action::host(Cmd::new("mount").args(["-t", "proc", "proc", &t("/proc")])),
        Action::host(Cmd::new("mount").args(["-t", "sysfs", "sys", &t("/sys")])),
        Action::host(Cmd::new("mount").args(["--bind", "/dev", &t("/dev")])),
        Action::host(Cmd::new("mount").args(["--bind", "/dev/pts", &t("/dev/pts")])),
        Action::host(Cmd::new("mount").args(["-t", "tmpfs", "tmpfs", &t("/run")])),
    ];
    if facts.firmware == Firmware::Uefi {
        chroot_mounts.push(Action::host_optional(Cmd::new("mount").args([
            "-t",
            "efivarfs",
            "efivarfs",
            &t("/sys/firmware/efi/efivars"),
        ])));
    }
    chroot_mounts.push(Action::host_optional(Cmd::new("cp").args([
        "-L",
        "--remove-destination",
        "/etc/resolv.conf",
        &t("/etc/resolv.conf"),
    ])));
    pe.push(Step {
        description: "Preparing the new system for configuration".into(),
        actions: chroot_mounts,
    });
    phases.push(Phase {
        pass: Pass::WindowsPe,
        steps: pe,
    });

    // --- offlineServicing: OEM files and packages ------------------------------------
    let mut servicing = Vec::new();
    if let Some(oem) = &oem_dir {
        servicing.push(Step {
            description: "Copying OEM files".into(),
            actions: vec![Action::host(Cmd::new("cp").args([
                "-rT",
                "--preserve=mode,timestamps",
                &oem.to_string_lossy(),
                &target_s,
            ]))],
        });
    }
    if !packages.is_empty() {
        let cache = "/var/cache/seglinux/packages";
        let mut copy = Cmd::new("cp").arg("-t").arg(t(cache));
        let mut install = Cmd::new("pacman").args(["-U", "--noconfirm", "--needed"]);
        for pkg in &packages {
            copy = copy.arg(pkg.to_string_lossy());
            let name = pkg
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            install = install.arg(format!("{cache}/{name}"));
        }
        servicing.push(Step {
            description: format!("Installing {} offline package(s)", packages.len()),
            actions: vec![
                Action::host(Cmd::new("mkdir").args(["-p", &t(cache)])),
                Action::host(copy),
                Action::chroot(install),
            ],
        });
    }
    if !servicing.is_empty() {
        phases.push(Phase {
            pass: Pass::OfflineServicing,
            steps: servicing,
        });
    }

    // --- specialize: per-machine configuration --------------------------------------
    let mut sp = Vec::new();
    sp.push(Step {
        description: "Writing the file system table".into(),
        actions: vec![Action::write("/etc/fstab", layout.fstab())],
    });

    let mut host_actions = vec![
        Action::write("/etc/hostname", format!("{hostname}\n")),
        Action::write(
            "/etc/hosts",
            format!(
                "127.0.0.1  localhost\n::1        localhost\n127.0.1.1  {hostname}.localdomain {hostname}\n"
            ),
        ),
    ];
    if openrc {
        host_actions.push(Action::set_key(
            "/etc/conf.d/hostname",
            "hostname",
            format!("\"{hostname}\""),
        ));
    }
    sp.push(Step {
        description: format!("Naming this computer {hostname}"),
        actions: host_actions,
    });

    let mut locale_gen = String::from("# Locales generated by SegLinux Setup.\n");
    let charset = region
        .locale
        .rsplit_once('.')
        .map(|(_, c)| c)
        .unwrap_or("UTF-8");
    locale_gen.push_str(&format!("{} {}\n", region.locale, charset_name(charset)));
    if region.locale != "en_US.UTF-8" {
        locale_gen.push_str("en_US.UTF-8 UTF-8\n");
    }
    sp.push(Step {
        description: format!("Setting the display language to {}", region.locale),
        actions: vec![
            Action::write("/etc/locale.gen", locale_gen),
            Action::chroot(Cmd::new("locale-gen")),
            Action::write("/etc/locale.conf", format!("LANG={}\n", region.locale)),
        ],
    });

    let console_map = console_keymap(&region.keyboard);
    let mut kb = vec![
        Action::write("/etc/vconsole.conf", format!("KEYMAP={console_map}\n")),
        Action::write(
            "/etc/X11/xorg.conf.d/00-keyboard.conf",
            format!(
                "Section \"InputClass\"\n    Identifier \"system-keyboard\"\n    MatchIsKeyboard \"on\"\n    Option \"XkbLayout\" \"{}\"\nEndSection\n",
                region.keyboard
            ),
        ),
    ];
    if openrc {
        kb.push(Action::set_key(
            "/etc/conf.d/keymaps",
            "keymap",
            format!("\"{console_map}\""),
        ));
    }
    sp.push(Step {
        description: format!("Setting the keyboard layout to {}", region.keyboard),
        actions: kb,
    });

    let (clock_flag, clock_name) = match region.hardware_clock {
        HardwareClock::Utc => ("--utc", "UTC"),
        HardwareClock::Local => ("--localtime", "local"),
    };
    let mut tz = vec![
        Action::Symlink {
            target: format!("/usr/share/zoneinfo/{}", region.timezone),
            link: "/etc/localtime".into(),
        },
        Action::chroot_optional(Cmd::new("hwclock").args(["--systohc", clock_flag])),
    ];
    if openrc {
        tz.push(Action::set_key(
            "/etc/conf.d/hwclock",
            "clock",
            format!("\"{clock_name}\""),
        ));
        // The kernel sets the system clock from a UTC hardware clock at boot
        // (rtc_hctosys), so OpenRC's hwclock need not read it again. Where
        // the clock's update interrupt never arrives (a UEFI virtual machine
        // did this), that read held up every boot service for about a
        // minute. A local-time clock (dual boot with Windows) still needs
        // it; either way the time is saved to the hardware clock at shutdown.
        let read_at_boot = match region.hardware_clock {
            HardwareClock::Utc => "\"NO\"",
            HardwareClock::Local => "\"YES\"",
        };
        tz.push(Action::set_key(
            "/etc/conf.d/hwclock",
            "clock_hctosys",
            read_at_boot,
        ));
    }
    sp.push(Step {
        description: format!("Setting the time zone to {}", region.timezone),
        actions: tz,
    });

    if let Some(size) = layout.zram {
        sp.push(Step {
            description: format!("Setting up {} of compressed swap in RAM (zram)", human_size(size)),
            actions: vec![
                Action::write("/etc/modules-load.d/zram.conf", "zram\n"),
                Action::write(
                    "/etc/udev/rules.d/99-seglinux-zram.rules",
                    format!(
                        "# Compressed swap in RAM, written by SegLinux Setup.\n\
                         ACTION==\"add\", KERNEL==\"zram0\", ATTR{{initstate}}==\"0\", ATTR{{comp_algorithm}}=\"zstd\", ATTR{{disksize}}=\"{}M\", RUN=\"/usr/bin/mkswap -U clear %N\"\n",
                        size / MIB
                    ),
                ),
                // Values recommended for swap on zram (ArchWiki, Fedora, Pop!_OS):
                // swapping to compressed RAM is cheap, so swap early.
                Action::write(
                    "/etc/sysctl.d/90-seglinux-zram.conf",
                    "vm.swappiness = 180\nvm.watermark_boost_factor = 0\nvm.watermark_scale_factor = 125\nvm.page-cluster = 0\n",
                ),
            ],
        });
    }
    if openrc && answer.performance.parallel_boot {
        sp.push(Step {
            description: "Starting services in parallel for a faster boot".into(),
            actions: vec![Action::set_key("/etc/rc.conf", "rc_parallel", "\"YES\"")],
        });
    }

    sp.push(Step {
        description: "Generating a machine ID".into(),
        actions: vec![Action::chroot_optional(
            Cmd::new("dbus-uuidgen").arg("--ensure=/etc/machine-id"),
        )],
    });
    sp.push(Step {
        description: "Preparing the package manager".into(),
        actions: vec![
            Action::chroot_optional(Cmd::new("pacman-key").arg("--init")),
            // Every keyring installed: Artix's, Chaotic-AUR's (Brave) and
            // SegLinux's own (its updates).
            Action::chroot_optional(Cmd::new("pacman-key").arg("--populate")),
            // SegLinux's update repository, when this image's seglinux
            // package carries the key its updates are signed with.
            Action::chroot_optional(Cmd::new("seglinux-update").arg("--setup")),
        ],
    });
    if !facts.online_packages.is_empty() {
        sp.push(Step {
            description: format!(
                "Downloading SegLinux's apps, drivers and games ({} packages, needs a network connection)",
                facts.online_packages.len()
            ),
            actions: vec![Action::chroot_optional(
                Cmd::new("pacman")
                    .args(["-Syu", "--noconfirm", "--needed"])
                    .args(facts.online_packages.iter().cloned()),
            )],
        });
        sp.push(Step {
            description:
                "Restoring the documentation, translations and drivers left out of the Lite image"
                    .into(),
            actions: vec![Action::chroot_optional(Cmd::new(RESTORE_TRIMMED_FILES))],
        });
    }
    if answer.drivers.nvidia == DriverChoice::Auto
        && let Some(gpu) = facts.gpus.iter().find(|g| g.nvidia_open_capable())
    {
        sp.push(Step {
            description: format!(
                "Installing the NVIDIA driver for {gpu} (needs a network connection)"
            ),
            actions: vec![Action::chroot_optional(Cmd::new("pacman").args([
                "-Syu",
                "--noconfirm",
                "--needed",
                "nvidia-open-dkms",
                "nvidia-utils",
                "lib32-nvidia-utils",
                "nvidia-settings",
            ]))],
        });
    }
    sp.push(Step {
        description: "Building the boot image (initramfs)".into(),
        actions: vec![Action::chroot(Cmd::new("mkinitcpio").arg("-P"))],
    });

    let boot = &answer.bootloader;
    let mut grub = vec![
        Action::set_key(
            "/etc/default/grub",
            "GRUB_DISTRIBUTOR",
            format!("\"{}\"", grub_distributor(&boot.id)),
        ),
        Action::set_key(
            "/etc/default/grub",
            "GRUB_THEME",
            format!("\"{GRUB_THEME}\""),
        ),
    ];
    if boot.os_prober {
        grub.push(Action::set_key(
            "/etc/default/grub",
            "GRUB_DISABLE_OS_PROBER",
            "false",
        ));
    }
    grub.push(Action::set_key(
        "/etc/default/grub",
        "GRUB_CMDLINE_LINUX_DEFAULT",
        format!("\"{}\"", kernel_parameters(answer, &layout).join(" ")),
    ));
    let efi_install = |extra: &[&str]| {
        Cmd::new("grub-install")
            .args(["--target=x86_64-efi", "--efi-directory=/boot/efi"])
            .args(extra.iter().copied())
    };
    match facts.firmware {
        Firmware::Uefi => {
            grub.push(Action::chroot(efi_install(&[&format!(
                "--bootloader-id={}",
                boot.id
            )])));
            if boot.removable {
                grub.push(Action::chroot(efi_install(&["--removable"])));
            }
        }
        Firmware::Bios => {
            grub.push(Action::chroot(
                Cmd::new("grub-install").args(["--target=i386-pc", &disk.path]),
            ));
            // Keep the disk bootable if it is later moved to a UEFI machine.
            grub.push(Action::chroot_optional(efi_install(&[
                "--removable",
                "--no-nvram",
            ])));
        }
    }
    grub.push(Action::chroot(
        Cmd::new("grub-mkconfig").args(["-o", "/boot/grub/grub.cfg"]),
    ));
    sp.push(Step {
        description: "Installing the boot manager".into(),
        actions: grub,
    });

    let mut services: Vec<String> = DEFAULT_SERVICES.iter().map(|s| s.to_string()).collect();
    for s in &answer.services.enable {
        if !services.contains(s) {
            services.push(s.clone());
        }
    }
    services.retain(|s| !answer.services.disable.contains(s));
    let mut svc: Vec<Action> = services.into_iter().map(Action::EnableService).collect();
    svc.extend(
        answer
            .services
            .disable
            .iter()
            .cloned()
            .map(Action::DisableService),
    );
    sp.push(Step {
        description: format!("Enabling services ({init})"),
        actions: svc,
    });

    for c in answer
        .commands
        .iter()
        .filter(|c| c.pass == CommandPass::Specialize)
    {
        sp.push(run_command_step(c));
    }
    phases.push(Phase {
        pass: Pass::Specialize,
        steps: sp,
    });

    // --- oobeSystem: accounts ---------------------------------------------------------
    let mut oobe = vec![Step {
        description: "Allowing administrators to use sudo".into(),
        actions: vec![Action::WriteFile {
            path: "/etc/sudoers.d/10-seglinux".into(),
            contents: "# Members of wheel are SegLinux administrators.\n%wheel ALL=(ALL:ALL) ALL\n"
                .into(),
            mode: 0o440,
        }],
    }];
    // The sign-in screen preselects the SegLinux desktop (not Plasma) and
    // the first account, as it would after they had been used once.
    if let Some(first) = answer.users.first() {
        let folder = GREETER_STATE
            .rsplit_once('/')
            .map_or(GREETER_STATE, |(dir, _)| dir);
        oobe.push(Step {
            description: "Making the SegLinux desktop the default session".into(),
            actions: vec![
                Action::write(
                    GREETER_STATE,
                    format!("user = \"{}\"\nsession = \"seglinux\"\n", first.name),
                ),
                // The sign-in screen runs as greetd's user and updates it.
                Action::chroot_optional(Cmd::new("chown").args(["-R", "greeter:greeter", folder])),
            ],
        });
    }
    for u in &answer.users {
        let mut groups: Vec<String> = DEFAULT_GROUPS.iter().map(|g| g.to_string()).collect();
        if u.admin {
            groups.push("wheel".into());
        }
        for g in &u.groups {
            if !groups.contains(g) {
                groups.push(g.clone());
            }
        }
        let secret = match (&u.password, &u.password_hash) {
            (_, Some(h)) => Secret::Hash(h.clone()),
            (Some(p), None) => Secret::Plain(p.clone()),
            (None, None) => Secret::Lock,
        };
        oobe.push(Step {
            description: format!(
                "Creating the account {}{}",
                u.name,
                if u.admin { " (administrator)" } else { "" }
            ),
            actions: vec![
                Action::AddUser {
                    name: u.name.clone(),
                    full_name: u.full_name.clone(),
                    groups,
                },
                Action::SetPassword {
                    user: u.name.clone(),
                    secret,
                },
            ],
        });
    }
    let root_secret = match (&answer.root.password, &answer.root.password_hash) {
        (_, Some(h)) => Secret::Hash(h.clone()),
        (Some(p), None) if !p.is_empty() => Secret::Plain(p.clone()),
        _ => Secret::Lock,
    };
    oobe.push(Step {
        description: "Securing the root account".into(),
        actions: vec![Action::SetPassword {
            user: "root".into(),
            secret: root_secret,
        }],
    });
    phases.push(Phase {
        pass: Pass::OobeSystem,
        steps: oobe,
    });

    // --- SetupComplete ------------------------------------------------------------------
    let mut done = vec![Step {
        description: "Running SetupComplete scripts".into(),
        actions: vec![Action::RunScripts {
            dir: SETUPCOMPLETE_DIR.into(),
        }],
    }];
    for c in answer
        .commands
        .iter()
        .filter(|c| c.pass == CommandPass::SetupComplete)
    {
        done.push(run_command_step(c));
    }
    done.push(Step {
        description: "Saving setup logs".into(),
        actions: vec![Action::SaveLogs {
            dir: "/var/log/seglinux".into(),
        }],
    });
    done.push(Step {
        description: "Unmounting the new system".into(),
        actions: vec![
            Action::host(Cmd::new("sync")),
            Action::host(Cmd::new("umount").args(["-R", &target_s])),
        ],
    });
    phases.push(Phase {
        pass: Pass::SetupComplete,
        steps: done,
    });

    Ok(Plan {
        target: target.to_path_buf(),
        summary: Summary {
            disk,
            layout,
            hostname,
            image,
            firmware: facts.firmware,
            init,
            users: answer.users.iter().map(|u| u.name.clone()).collect(),
            online_packages: facts.online_packages.len(),
        },
        phases,
    })
}

/// Kernel command line for the installed system: a quiet boot without the
/// soft-lockup watchdogs (less background work and fewer interrupts while
/// gaming), zswap off when zram provides the swap, then the answer file's
/// own parameters.
pub fn kernel_parameters(answer: &Answer, layout: &Layout) -> Vec<String> {
    let mut params: Vec<String> = ["loglevel=3", "quiet", "nowatchdog", "nmi_watchdog=0"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if layout.zram.is_some() {
        params.push("zswap.enabled=0".into());
    }
    for p in &answer.performance.kernel_parameters {
        if !params.contains(p) {
            params.push(p.clone());
        }
    }
    params
}

fn run_command_step(c: &crate::answer::RunCommand) -> Step {
    Step {
        description: c
            .description
            .clone()
            .unwrap_or_else(|| format!("Running {}", shell_quote(&c.command))),
        actions: vec![Action::chroot(Cmd::new("/bin/sh").args(["-c", &c.command]))],
    }
}

/// `UTF-8`/`utf8` -> `UTF-8` for locale.gen.
fn charset_name(charset: &str) -> String {
    match charset.to_ascii_lowercase().replace('-', "").as_str() {
        "utf8" => "UTF-8".into(),
        _ => charset.to_string(),
    }
}

/// Console keymap for an X layout; they share names except for a few.
pub fn console_keymap(layout: &str) -> String {
    match layout {
        "gb" => "uk".into(),
        "latam" => "la-latin1".into(),
        "br" => "br-abnt2".into(),
        "ch" => "fr_CH".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer::{GIB, MIB};
    use crate::ids::Sequential;

    fn facts() -> Facts {
        Facts {
            firmware: Firmware::Uefi,
            init: InitSystem::OpenRc,
            disks: vec![
                Disk {
                    path: "/dev/vda".into(),
                    size: 64 * GIB,
                    model: "Virtio".into(),
                    removable: false,
                    read_only: false,
                    transport: String::new(),
                },
                Disk {
                    path: "/dev/sdb".into(),
                    size: 16 * GIB,
                    model: "USB stick".into(),
                    removable: true,
                    read_only: false,
                    transport: "usb".into(),
                },
            ],
            boot_disk: Some("/dev/sdb".into()),
            image: Some("/run/artix/bootmnt/LiveOS/rootfs.img".into()),
            oem_dir: None,
            offline_packages: vec![],
            online_packages: vec![],
            network: true,
            memory: 4 * GIB,
            gpus: vec![],
            timezones: None,
            locales: None,
            keyboards: None,
        }
    }

    fn answer(extra: &str) -> Answer {
        Answer::parse(&format!(
            "[disk]\ndevice = \"auto\"\nwipe = true\n[[users]]\nname = \"alice\"\npassword = \"s3cr3t-Pa55\"\n{extra}"
        ))
        .unwrap()
    }

    fn all_actions(plan: &Plan) -> Vec<&Action> {
        plan.phases
            .iter()
            .flat_map(|p| p.steps.iter())
            .flat_map(|s| s.actions.iter())
            .collect()
    }

    fn build_plan(a: &Answer, f: &Facts) -> Result<Plan> {
        build(a, f, &mut Sequential::default(), Path::new(DEFAULT_TARGET))
    }

    #[test]
    fn uefi_openrc_plan() {
        let plan = build_plan(&answer(""), &facts()).unwrap();
        assert_eq!(plan.summary.disk.path, "/dev/vda");
        assert_eq!(plan.summary.hostname, "DESKTOP-0000003");
        let passes: Vec<Pass> = plan.phases.iter().map(|p| p.pass).collect();
        assert_eq!(
            passes,
            vec![
                Pass::WindowsPe,
                Pass::Specialize,
                Pass::OobeSystem,
                Pass::SetupComplete
            ]
        );

        let text = plan.to_string();
        // Default swap is zram: no swap partition, root is partition 3.
        assert!(!text.contains("mkswap -L"), "{text}");
        assert!(text.contains(
            "$ mkfs.ext4 -F -L SegLinux -U 00000000-0000-4000-8000-000000000002 /dev/vda3"
        ));
        assert!(text.contains("  Swap:      4.0 GiB compressed in RAM (zram)"));
        assert!(text.contains("ATTR{comp_algorithm}=\"zstd\", ATTR{disksize}=\"4096M\""));
        assert!(text.contains("| /dev/zram0  none  swap  defaults,discard,pri=100  0 0"));
        assert!(text.contains("| vm.swappiness = 180"));
        assert!(text.contains(
            "set GRUB_CMDLINE_LINUX_DEFAULT=\"loglevel=3 quiet nowatchdog nmi_watchdog=0 zswap.enabled=0\" in /etc/default/grub"
        ));
        assert!(text.contains("set rc_parallel=\"YES\" in /etc/rc.conf"));
        // The sign-in screen preselects the SegLinux desktop and the user.
        assert!(
            text.contains("/var/lib/seglinux-greeter/state.toml"),
            "{text}"
        );
        assert!(text.contains("| user = \"alice\""));
        assert!(text.contains("| session = \"seglinux\""));
        assert!(text.contains(
            "(new system) $ chown -R greeter:greeter /var/lib/seglinux-greeter  # may fail"
        ));
        assert!(text.contains("(new system) $ pacman-key --populate  # may fail"));
        assert!(text.contains("(new system) $ seglinux-update --setup  # may fail"));
        assert!(text.contains("$ mkfs.fat -F 32 -n SYSTEM -i 00000001 /dev/vda1"));
        assert!(text.contains(
            "$ unsquashfs -f -no-progress -d /mnt/seglinux /run/artix/bootmnt/LiveOS/rootfs.img"
        ));
        assert!(text.contains("(new system) $ grub-install --target=x86_64-efi --efi-directory=/boot/efi --bootloader-id=SegLinux"));
        assert!(text.contains("set GRUB_DISTRIBUTOR=\"Seg\" in /etc/default/grub"));
        assert!(text.contains(
            "set GRUB_THEME=\"/usr/share/grub/themes/seglinux/theme.txt\" in /etc/default/grub"
        ));
        assert!(text.contains("--removable"));
        assert!(text.contains("efivarfs"));
        assert!(text.contains("set hostname=\"DESKTOP-0000003\" in /etc/conf.d/hostname"));
        assert!(text.contains("| UUID=0000-0001  /boot/efi  vfat"));
        assert!(text.contains("write /etc/sudoers.d/10-seglinux (mode 440)"));
        // No automatic sign-in: the image's greetd configuration (the
        // sign-in screen) stays.
        assert!(!text.contains("write /etc/greetd/config.toml"), "{text}");
        assert!(!text.contains("[initial_session]"));
        assert!(text.contains("== Setting up user accounts (oobeSystem)"));
        // No NVIDIA GPU, no driver step.
        assert!(!text.contains("nvidia"));

        let actions = all_actions(&plan);
        assert!(actions.contains(&&Action::EnableService("greetd".into())));
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::AddUser { name, groups, .. }
            if name == "alice"
                && groups.contains(&"wheel".to_string())
                && groups.contains(&"gamemode".to_string())))
        );
        assert!(actions.contains(&&Action::SetPassword {
            user: "root".into(),
            secret: Secret::Lock
        }));
        // The password never appears in the printed plan.
        assert!(!text.contains("s3cr3t-Pa55"));
    }

    #[test]
    fn hardware_clock_read_at_boot_only_in_local_time() {
        let text = build_plan(&answer(""), &facts()).unwrap().to_string();
        assert!(
            text.contains("set clock=\"UTC\" in /etc/conf.d/hwclock"),
            "{text}"
        );
        assert!(text.contains("set clock_hctosys=\"NO\" in /etc/conf.d/hwclock"));
        let local = answer("[region]\nhardware_clock = \"local\"\n");
        let text = build_plan(&local, &facts()).unwrap().to_string();
        assert!(text.contains("set clock=\"local\" in /etc/conf.d/hwclock"));
        assert!(text.contains("set clock_hctosys=\"YES\" in /etc/conf.d/hwclock"));
    }

    #[test]
    fn swap_partition_on_request() {
        let mut a = answer("");
        a.disk.swap = "auto".into();
        let plan = build_plan(&a, &facts()).unwrap();
        let text = plan.to_string();
        // 4 GiB of memory -> 4 GiB swap partition 3, root 4, no zram.
        assert!(
            text.contains("$ mkswap -L swap -U 00000000-0000-4000-8000-000000000002 /dev/vda3"),
            "{text}"
        );
        assert!(text.contains("/dev/vda4"));
        assert!(!text.contains("zram"));
        assert!(
            text.contains(
                "GRUB_CMDLINE_LINUX_DEFAULT=\"loglevel=3 quiet nowatchdog nmi_watchdog=0\""
            )
        );
    }

    #[test]
    fn nvidia_driver_for_turing_and_newer_only() {
        use crate::facts::Gpu;
        let mut f = facts();
        f.gpus = vec![Gpu {
            vendor: Gpu::NVIDIA,
            device: 0x2684,
        }];
        let text = build_plan(&answer(""), &f).unwrap().to_string();
        assert!(text.contains("Installing the NVIDIA driver for NVIDIA 10de:2684"));
        assert!(text.contains("pacman -Syu --noconfirm --needed nvidia-open-dkms nvidia-utils lib32-nvidia-utils nvidia-settings  # may fail"));
        // The driver is installed before the initramfs is built.
        assert!(text.find("nvidia-open-dkms").unwrap() < text.find("mkinitcpio -P").unwrap());

        let a = answer("[drivers]\nnvidia = \"never\"\n");
        assert!(
            !build_plan(&a, &f)
                .unwrap()
                .to_string()
                .contains("nvidia-open")
        );

        f.gpus[0].device = 0x1b80; // GTX 1080
        assert!(
            !build_plan(&answer(""), &f)
                .unwrap()
                .to_string()
                .contains("nvidia-open")
        );
    }

    #[test]
    fn lite_edition_downloads_the_rest() {
        let a = answer("");
        let mut f = facts();
        f.online_packages = vec!["wine".into(), "steam".into(), "sddm-openrc".into()];
        let plan = build_plan(&a, &f).unwrap();
        let text = plan.to_string();
        assert!(text.contains("Download:  3 packages"));
        assert!(text.contains(
            "(new system) $ pacman -Syu --noconfirm --needed wine steam sddm-openrc  # may fail"
        ));
        assert!(
            text.contains("(new system) $ /usr/lib/seglinux/restore-trimmed-files  # may fail")
        );
        // After the keyring, before the initramfs and the services it installs.
        let download = text.find("--needed wine").unwrap();
        let restore = text.find("restore-trimmed-files").unwrap();
        assert!(download < restore && restore < text.find("mkinitcpio -P").unwrap());
        assert!(text.find("pacman-key --populate").unwrap() < download);
        assert!(download < text.find("mkinitcpio -P").unwrap());
        assert!(download < text.find("Enabling services").unwrap());

        let full = build_plan(&a, &facts()).unwrap().to_string();
        assert!(!full.contains("Download:"));
        assert!(!full.contains("--needed wine"));
        assert!(!full.contains("restore-trimmed-files"));
    }

    #[test]
    fn extra_kernel_parameters() {
        let a = answer(
            "[performance]\nparallel_boot = false\nkernel_parameters = [\"amd_pstate=active\", \"quiet\"]\n",
        );
        let text = build_plan(&a, &facts()).unwrap().to_string();
        assert!(text.contains("nmi_watchdog=0 zswap.enabled=0 amd_pstate=active\""));
        assert!(!text.contains("rc_parallel"));
    }

    #[test]
    fn bios_dinit_btrfs_plan() {
        let mut f = facts();
        f.firmware = Firmware::Bios;
        f.init = InitSystem::Dinit;
        let a = answer("[region]\ntimezone = \"Europe/Paris\"\nkeyboard = \"gb\"\n");
        let mut a = a;
        a.disk.filesystem = Filesystem::Btrfs;
        a.disk.swap = "none".into();
        let plan = build_plan(&a, &f).unwrap();
        let text = plan.to_string();
        assert!(text.contains("grub-install --target=i386-pc /dev/vda"));
        assert!(text.contains("--removable --no-nvram  # may fail"));
        assert!(!text.contains("efivarfs"));
        assert!(text.contains("btrfs subvolume create /mnt/seglinux/@home"));
        assert!(text.contains("subvol=/@home"));
        assert!(text.contains("link /etc/localtime -> /usr/share/zoneinfo/Europe/Paris"));
        assert!(text.contains("KEYMAP=uk"));
        assert!(text.contains("XkbLayout\" \"gb\""));
        // OpenRC-only files are not written for dinit.
        assert!(!text.contains("/etc/conf.d/"));
        assert!(!text.contains("rc_parallel"));
        assert_eq!(plan.summary.layout.root().path, "/dev/vda3");
    }

    #[test]
    fn refuses_install_media_and_small_disks() {
        let mut a = answer("");
        a.disk.device = "/dev/sdb".into();
        let err = build_plan(&a, &facts()).unwrap_err().to_string();
        assert!(err.contains("install media"), "{err}");

        a.disk.device = "/dev/nope".into();
        let err = build_plan(&a, &facts()).unwrap_err().to_string();
        assert!(
            err.contains("not found") && err.contains("/dev/vda"),
            "{err}"
        );

        let mut f = facts();
        f.disks[0].size = 10 * GIB;
        let err = build_plan(&answer(""), &f).unwrap_err().to_string();
        assert!(err.contains("no internal disk"), "{err}");

        let mut a = answer("");
        a.disk.device = "/dev/vda".into();
        a.disk.swap = "60GiB".into();
        let err = build_plan(&a, &facts()).unwrap_err().to_string();
        assert!(err.contains("reduce swap"), "{err}");
    }

    #[test]
    fn unknown_region_values_are_rejected_when_known_lists_exist() {
        let mut f = facts();
        f.timezones = Some(["UTC".to_string()].into());
        let a = answer("[region]\ntimezone = \"Mars/Olympus_Mons\"\n");
        assert!(
            build_plan(&a, &f)
                .unwrap_err()
                .to_string()
                .contains("time zone")
        );
    }

    #[test]
    fn oem_packages_and_commands() {
        let mut f = facts();
        f.oem_dir = Some("/run/artix/bootmnt/seglinux/oem".into());
        f.offline_packages =
            vec!["/run/artix/bootmnt/seglinux/packages/foo-1-1-x86_64.pkg.tar.zst".into()];
        let a = answer(
            "[services]\nenable = [\"sshd\"]\ndisable = [\"cupsd\"]\n\
             [[run]]\npass = \"specialize\"\ncommand = \"echo hi\"\n\
             [[run]]\npass = \"setup-complete\"\ncommand = \"echo bye\"\ndescription = \"Saying bye\"\n",
        );
        let plan = build_plan(&a, &f).unwrap();
        assert_eq!(plan.phases[1].pass, Pass::OfflineServicing);
        let text = plan.to_string();
        assert!(text.contains(
            "cp -rT --preserve=mode,timestamps /run/artix/bootmnt/seglinux/oem /mnt/seglinux"
        ));
        assert!(text.contains(
            "pacman -U --noconfirm --needed /var/cache/seglinux/packages/foo-1-1-x86_64.pkg.tar.zst"
        ));
        assert!(text.contains("(new system) $ /bin/sh -c 'echo hi'"));
        assert!(text.contains(" * Saying bye"));
        let actions = all_actions(&plan);
        assert!(actions.contains(&&Action::EnableService("sshd".into())));
        assert!(!actions.contains(&&Action::EnableService("cupsd".into())));
        assert!(actions.contains(&&Action::DisableService("cupsd".into())));
    }

    #[test]
    fn step_count_and_sizes() {
        let plan = build_plan(&answer(""), &facts()).unwrap();
        assert!(plan.step_count() > 15);
        assert_eq!(plan.summary.layout.esp().size, Some(512 * MIB));
    }

    /// The text Setup writes to `path`.
    fn written<'a>(plan: &'a Plan, path: &str) -> Option<&'a str> {
        all_actions(plan).into_iter().find_map(|a| match a {
            Action::WriteFile {
                path: p, contents, ..
            } if p == path => Some(contents.as_str()),
            _ => None,
        })
    }

    #[test]
    fn sign_in_screen() {
        // Every installed system shows the sign-in screen (the image's
        // greetd configuration, unchanged), the account preselected.
        let plan = build_plan(&answer(""), &facts()).unwrap();
        assert_eq!(written(&plan, GREETD_CONFIG), None);
        let state: toml::Table = written(&plan, GREETER_STATE).unwrap().parse().unwrap();
        assert_eq!(state["user"].as_str(), Some("alice"));
        assert_eq!(state["session"].as_str(), Some("seglinux"));
        let defaults: toml::Table = GREETD_DEFAULTS.parse().unwrap();
        assert!(defaults.get("initial_session").is_none());
        assert_eq!(
            defaults["default_session"]["command"].as_str(),
            Some("seglinux-greeter-session")
        );
        assert_eq!(
            defaults["default_session"]["user"].as_str(),
            Some("greeter")
        );
        assert_eq!(defaults["terminal"]["vt"].as_integer(), Some(7));
        // Older answer files may say autologin = false; true is refused.
        let refused = Answer::parse(
            "[disk]\ndevice = \"auto\"\nwipe = true\n[[users]]\nname = \"bob\"\npassword = \"s3cr3t-Pa55\"\nautologin = true\n",
        );
        let message = match refused {
            Err(e) => format!("{e:#}"),
            Ok(a) => a.validate().join("\n"),
        };
        assert!(
            message.contains("automatic sign-in is no longer offered"),
            "{message}"
        );
    }

    #[test]
    fn images_carry_the_greetd_defaults() {
        for overlay in [
            include_str!("../../../iso/iso-profiles/seglinux/root-overlay/etc/greetd/config.toml"),
            include_str!(
                "../../../iso/iso-profiles/seglinux-lite/root-overlay/etc/greetd/config.toml"
            ),
        ] {
            assert_eq!(overlay, GREETD_DEFAULTS);
        }
        // The live session signs its user in at once, to the same screen.
        let live: toml::Table =
            include_str!("../../../iso/iso-profiles/seglinux/live-overlay/etc/greetd/config.toml")
                .parse()
                .unwrap();
        let defaults: toml::Table = GREETD_DEFAULTS.parse().unwrap();
        assert_eq!(live["default_session"], defaults["default_session"]);
        assert_eq!(live["terminal"], defaults["terminal"]);
        assert_eq!(live["initial_session"]["user"].as_str(), Some("seglinux"));
    }

    #[test]
    fn keymaps() {
        assert_eq!(console_keymap("us"), "us");
        assert_eq!(console_keymap("gb"), "uk");
    }

    #[test]
    fn grub_menu_names() {
        assert_eq!(grub_distributor("SegLinux"), "Seg");
        assert_eq!(grub_distributor("My Linux"), "My");
        assert_eq!(grub_distributor("Artix"), "Artix");
        assert_eq!(grub_distributor("Linux"), "Linux");
    }
}
