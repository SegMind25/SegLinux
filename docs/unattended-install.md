# Unattended installation (autounattend.toml)

`seglinux-setup` can install SegLinux without asking anything, driven by an
answer file, just like Windows Setup with `autounattend.xml`. The format is
TOML; the full commented example is
[`crates/seglinux-setup/autounattend.example.toml`](../crates/seglinux-setup/autounattend.example.toml)
(`seglinux-setup example` prints it, and it is installed as
`/usr/share/seglinux/autounattend.example.toml`).

## Minimal answer file

```toml
[setup]
auto_start = true        # start as soon as the live desktop finds this file

[disk]
device = "auto"          # largest internal disk; or "/dev/nvme0n1"
wipe = true              # required: the whole disk is erased

[[users]]
name = "ada"
full_name = "Ada Lovelace"
password_hash = "$6$..."   # openssl passwd -6
```

Everything else has defaults: ext4, compressed swap in RAM (zram), `en_US.UTF-8`,
US keyboard, UTC, a `DESKTOP-XXXXXXX` computer name, reboot when done.

## Where Setup looks for it

In this order (compare Windows' implicit search order in
[windows-iso-anatomy.md](windows-iso-anatomy.md#4-unattended-installation)):

1. `seglinux-setup install --unattend /path/to/file.toml`
2. Kernel command line: `seglinux.unattend=/path/to/file.toml`
3. `autounattend.toml` at the root of the install media (next to `LiveOS/`)
4. `autounattend.toml` at the root of any other removable drive (mounted
   read-only, copied to `/run/seglinux/`)
5. `/etc/seglinux/autounattend.toml` baked into the live image
   (`iso/iso-profiles/seglinux/live-overlay/etc/seglinux/`)

`seglinux-setup install --unattend` (no path) and `seglinux-setup
find-answer-file` use the same search.

The graphical Setup works the same way: it writes an answer file from your
choices (readable only by you, deleted afterwards) and runs
`sudo -n seglinux-setup install --unattend FILE --progress json`, which
prints one JSON object per line (`{"event":"progress",...}`, then
`{"event":"done"}` or `{"event":"error","message":...}`).

With `auto_start = true`, the live session's autostart entry runs the
installation in a terminal window as soon as the desktop is up. Without it,
start it by hand: `sudo seglinux-setup install --unattend`.

## Reference

| Section / key | Default | Windows counterpart | Meaning |
|---------------|---------|---------------------|---------|
| `[setup] auto_start` | `false` | Setup starting by itself | Start when the live session finds the file |
| `[setup] finish` | `"reboot"` | | `reboot`, `poweroff` or `none` |
| `[disk] device` | required | `DiskID` | Whole disk, or `"auto"` (largest internal disk ≥ 20 GiB) |
| `[disk] wipe` | required | `WillWipeDisk` | Must be `true` |
| `[disk] filesystem` | `"ext4"` | | `ext4`, `btrfs` (subvolumes `@` and `@home`, zstd compression) or `xfs` |
| `[disk] swap` | `"zram"` | Page file | `"zram"` (compressed in RAM, sized from memory, 2–8 GiB), a partition size (`"8GiB"`), `"auto"` (partition sized from RAM) or `"none"` |
| `[disk] esp_size` | `"512MiB"` | ESP size | At least 200 MiB |
| `[image] source` | `"auto"` | `ImageInstall` / `InstallFrom` | Path to the squashfs image (`LiveOS/rootfs.img`) |
| `[oem] path` | `"auto"` | `$OEM$` | Folder copied over `/` (`seglinux/oem` on the media) |
| `[oem] packages` | `"auto"` | offlineServicing packages | Folder of `.pkg.tar.zst` files installed offline (`seglinux/packages` on the media) |
| `[region] locale` | `"en_US.UTF-8"` | `UILanguage`, `UserLocale` | |
| `[region] keyboard` | `"us"` | `InputLocale` | X layout name; also used for the console and the SegLinux desktop (changeable later in Settings) |
| `[region] timezone` | `"UTC"` | `TimeZone` | `Region/City` |
| `[region] hardware_clock` | `"utc"` | | `"local"` when dual-booting Windows |
| `[computer] name` | `"auto"` | `ComputerName` | `"auto"` → `DESKTOP-XXXXXXX` |
| `[[users]] name` | required | `LocalAccount/Name` | Lowercase Linux user name |
| `[[users]] full_name` | `""` | `DisplayName` | |
| `[[users]] password` / `password_hash` | one required | `Password` | Plain text, or a crypt(3) hash |
| `[[users]] admin` | `true` | `Group: Administrators` | Member of `wheel`, can use `sudo` |
| `[[users]] autologin` | `false` | `AutoLogon` | At most one user |
| `[[users]] groups` | `[]` | `Group` | Extra groups; missing ones are skipped |
| `[root] password` / `password_hash` | locked | Built-in Administrator | |
| `[bootloader] id` | `"SegLinux"` | BCD description | UEFI boot entry name |
| `[bootloader] removable` | `true` | | Also install `\EFI\BOOT\BOOTX64.EFI` |
| `[bootloader] os_prober` | `true` | | Add Windows and other systems to the boot menu |
| `[performance] parallel_boot` | `true` | | OpenRC: start services in parallel (`rc_parallel`) |
| `[performance] kernel_parameters` | `[]` | | Added to `loglevel=3 quiet nowatchdog nmi_watchdog=0` (and `zswap.enabled=0` with zram) |
| `[drivers] nvidia` | `"auto"` | Driver installation | `"auto"`: install NVIDIA's driver for GeForce GTX 16/RTX and newer when online; `"never"` |
| `[services] enable` / `disable` | `[]` | | On top of SegLinux's defaults |
| `[[run]] pass`, `command`, `description` | | `RunSynchronous`, `SetupComplete.cmd` | Shell command run in the new system during `specialize` or `setup-complete` |

Unknown keys are errors, so typos are caught. Check a file with:

```sh
seglinux-setup validate autounattend.toml
sudo seglinux-setup plan --unattend autounattend.toml   # full dry run on the target machine
```

## OEM folder and SetupComplete scripts

```
<install media>/
├── autounattend.toml
└── seglinux/
    ├── oem/                          copied over the new system's /
    │   ├── etc/skel/...              defaults for every new user
    │   └── etc/seglinux/setupcomplete.d/
    │       └── 50-company-setup      executable, runs at the end of Setup
    └── packages/
        └── company-tools-1.0-1-x86_64.pkg.tar.zst
```

Scripts in `/etc/seglinux/setupcomplete.d/` run in name order inside the
new system, as root, after users are created, like `SetupComplete.cmd`. A
failing script is logged but does not stop Setup.

## Logs

Setup writes `/var/log/seglinux/setupact.log` (every action and command
output) and `setuperr.log` (errors) in the live system and copies both into
the installed system.
