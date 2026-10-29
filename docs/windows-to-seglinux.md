# From Windows to SegLinux: how each piece maps

SegLinux rebuilds the Windows installation experience on Linux (Artix
packages, without systemd). This table maps every part of a Windows ISO and
of Windows Setup (see [windows-iso-anatomy.md](windows-iso-anatomy.md)) onto
what SegLinux uses instead, and where it lives in this repository.

## The installation media

| Windows | SegLinux | Where |
|---------|----------|-------|
| UDF image built with `oscdimg` | ISO 9660 hybrid image (CD and USB) built by Artix's `buildiso` with `xorriso` | `iso/build.sh` |
| El Torito BIOS entry: `etfsboot.com` → `bootmgr` → `\boot\bcd` | El Torito BIOS entry: GRUB (`i386-pc`) → live boot menu | `artix-grub-live` (livefs) |
| El Torito UEFI entry: `efisys.bin` → `\efi\boot\bootx64.efi` → BCD | EFI FAT image with GRUB's `BOOTX64.EFI` → the same menu | `artix-grub-live` (livefs) |
| `sources\boot.wim` index 2 (Windows PE + Setup) loaded into a RAM disk | `vmlinuz` + live initramfs (`iso-initcpio` hooks) mounting `LiveOS/rootfs.img` + `LiveOS/livefs.img` as an overlay | `iso/iso-profiles/seglinux/profile.yaml` |
| `sources\install.wim` / `install.esd` | `LiveOS/rootfs.img`, a squashfs of the system to install | `rootfs:` in `profile.yaml` |
| Editions (WIM indexes: Home, Pro, ...) | ISO profiles (one `profile.yaml` each) | `iso/iso-profiles/` |
| `sources\setup.exe` | `seglinux-setup` (text-mode wizard or unattended) | `crates/seglinux-setup` |
| `autounattend.xml` at the media root | `autounattend.toml` at the media root | `crates/seglinux-setup/autounattend.example.toml` |
| `sources\$OEM$\` | `seglinux/oem/` on the media, copied over the new system | `[oem] path` |
| Drivers/updates for offlineServicing | `seglinux/packages/*.pkg.tar.zst` on the media, installed with `pacman -U` in the new system | `[oem] packages` |
| Windows RE | The live ISO itself (boot it to repair) and the fallback initramfs | |
| Secure Boot-signed boot chain | Not signed: disable Secure Boot (see [building.md](building.md#secure-boot)) | |

## Setup

`seglinux-setup` computes a complete plan before touching the disk and runs
it in the same passes as Windows Setup (`seglinux-setup plan --unattend`
prints it):

| Windows Setup | SegLinux Setup |
|---------------|----------------|
| **windowsPE**: `DiskConfiguration` with diskpart | GPT layout written with `sfdisk`: ESP (512 MiB), BIOS boot (1 MiB), swap (optional), root. One disk boots on both UEFI and legacy BIOS. |
| MSR partition | BIOS boot partition (GRUB's core image on GPT disks) |
| Pagefile | Compressed swap in RAM (zram) sized from memory, like a "system managed" page file; a swap partition on request |
| **windowsPE**: `ImageInstall`, `DISM /Apply-Image` | `unsquashfs -f -d /mnt/seglinux LiveOS/rootfs.img` |
| `bcdboot C:\Windows` | `grub-install` (UEFI entry named like the answer file's `[bootloader] id`, plus the removable `\EFI\BOOT\BOOTX64.EFI` fallback) and `grub-mkconfig`; `os-prober` adds Windows to the menu |
| **offlineServicing** | OEM files and offline packages |
| **specialize**: computer name, `International-Core`, `RunSynchronous` | `/etc/fstab` (UUIDs chosen in advance), host name, locale, keyboard (console, X11, Plasma), time zone and hardware clock, machine ID, initramfs, boot manager, services, `[[run]] pass = "specialize"` |
| `ComputerName` `*` → `DESKTOP-XXXXXXX` | `[computer] name = "auto"` → `DESKTOP-XXXXXXX` |
| **oobeSystem**: `UserAccounts` | Users created with `useradd`; administrators join `wheel` and may use `sudo` |
| Built-in Administrator account disabled | `root` locked unless the answer file sets a password |
| `AutoLogon` | Not offered: SegLinux always shows its sign-in screen |
| `SetupComplete.cmd` | Executables in `/etc/seglinux/setupcomplete.d/` and `[[run]] pass = "setup-complete"`; failures are logged, not fatal |
| `Panther\setupact.log`, `setuperr.log` | `/var/log/seglinux/setupact.log`, `setuperr.log` (copied into the new system) |
| Implicit answer file search | Command line → `seglinux.unattend=` kernel parameter → media root → root of other removable drives → `/etc/seglinux/autounattend.toml` |
| Setup starts by itself when `autounattend.xml` is found | `[setup] auto_start = true` starts the unattended installation when the live desktop comes up |
| Hardware requirement checks | x86-64, a disk of at least 20 GiB, not the install media; the plan refuses anything else |

## The installed system

| Windows | SegLinux |
|---------|----------|
| Action Center, notifications, the notification area (system tray) | Notification pop-ups and history (Do not disturb) and other applications' tray icons, in the dock's tray (see [desktop.md](desktop.md#implemented)) |
| Sign-in screen (LogonUI) | `seglinux-greeter` on greetd: clock, accounts, password, the desktop to start, sleep, restart and shut down (see [desktop.md](desktop.md#sign-in-screen)) |
| Desktop, taskbar, Start menu | The SegLinux Rust shell on labwc: a floating dock with Start, pinned programs (Dolphin, Brave, the foot terminal, Steam), running windows, quick settings and clock. KDE Plasma with the `org.seglinux.desktop` layout is the fallback session |
| Settings (Win+I), Personalization, Mouse, Keyboard | SegLinux Settings (Super+I): theme, accent, wallpaper, dock, clock, window focus and workspaces, keyboard layouts, mouse and touchpad, gaming; applied at once (see [desktop.md](desktop.md#settings)) |
| Virtual desktops (Win+Ctrl+Left/Right) | labwc workspaces, with the same shortcut; their number is set in Settings |
| Xbox app / Game Bar, Game Mode | Steam, Lutris, MangoHud, GameMode and the SegLinux Gaming Mode session (see [gaming.md](gaming.md)) |
| Window buttons (minimize, maximize, close on the right), Alt+Tab thumbnails | KWin defaults in `/etc/xdg/kwinrc` |
| File Explorer, Notepad, Calculator, Snipping Tool, Photos, Task Manager, Disk Management | Dolphin, Kate, KCalc, the desktop's own screenshots (Print, Super+Shift+S), Gwenview, System Monitor, Partition Manager |
| Microsoft Store | Discover with Flathub (Flatpak) |
| Windows Update | `sudo seglinux-upgrade` (see [updating.md](updating.md)) or Discover |
| Edge, Office, Media Player | Brave, LibreOffice, Haruna |
| Network places and sharing (SMB) | Dolphin `smb://`, folder sharing via kdenetwork-filesharing |
| Arial, Times New Roman, Courier New, Calibri | Metric-compatible Liberation and Carlito fonts |
| Services (`services.msc`) | OpenRC or dinit services (chosen when the ISO is built) |
| Double-clicking an `.exe`, `.msi`, `.lnk` or `.bat` | `segrun` via the desktop's file associations; `./program.exe` in a terminal via binfmt_misc |
| `C:\`, `Program Files`, the registry | One Wine prefix per "Windows environment", `segrun prefix open`, `segrun tool regedit` |
| Control Panel, Add or Remove Programs | `segrun tool control`, "Add or Remove Windows Programs" in the menu |
| DirectX | DXVK (Direct3D 9/10/11) and VKD3D-Proton (Direct3D 12) over Vulkan, picked automatically from the program's imports |

## What does not carry over

* **Kernel drivers** (anti-cheat, copy protection, hardware drivers shipped
  as `.sys` files) cannot run under Wine. `segrun info` warns about them.
* **Secure Boot**: Artix's boot chain is not signed by Microsoft.
* **Editions and activation**: there is nothing to activate; editions are
  just different ISO profiles.
