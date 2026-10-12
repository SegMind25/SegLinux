# From Windows to Wincraft: how each piece maps

Wincraft rebuilds the Windows installation experience on Artix Linux. This
table maps every part of a Windows ISO and of Windows Setup (see
[windows-iso-anatomy.md](windows-iso-anatomy.md)) onto what Wincraft uses
instead, and where it lives in this repository.

## The installation media

| Windows | Wincraft | Where |
|---------|----------|-------|
| UDF image built with `oscdimg` | ISO 9660 hybrid image (CD and USB) built by Artix's `buildiso` with `xorriso` | `iso/build.sh` |
| El Torito BIOS entry: `etfsboot.com` → `bootmgr` → `\boot\bcd` | El Torito BIOS entry: GRUB (`i386-pc`) → live boot menu | `artix-grub-live` (livefs) |
| El Torito UEFI entry: `efisys.bin` → `\efi\boot\bootx64.efi` → BCD | EFI FAT image with GRUB's `BOOTX64.EFI` → the same menu | `artix-grub-live` (livefs) |
| `sources\boot.wim` index 2 (Windows PE + Setup) loaded into a RAM disk | `vmlinuz` + live initramfs (`iso-initcpio` hooks) mounting `LiveOS/rootfs.img` + `LiveOS/livefs.img` as an overlay | `iso/iso-profiles/wincraft/profile.yaml` |
| `sources\install.wim` / `install.esd` | `LiveOS/rootfs.img`, a squashfs of the system to install | `rootfs:` in `profile.yaml` |
| Editions (WIM indexes: Home, Pro, ...) | ISO profiles (one `profile.yaml` each) | `iso/iso-profiles/` |
| `sources\setup.exe` | `wincraft-setup` (text-mode wizard or unattended) | `crates/wincraft-setup` |
| `autounattend.xml` at the media root | `autounattend.toml` at the media root | `crates/wincraft-setup/autounattend.example.toml` |
| `sources\$OEM$\` | `wincraft/oem/` on the media, copied over the new system | `[oem] path` |
| Drivers/updates for offlineServicing | `wincraft/packages/*.pkg.tar.zst` on the media, installed with `pacman -U` in the new system | `[oem] packages` |
| Windows RE | The live ISO itself (boot it to repair) and the fallback initramfs | |
| Secure Boot-signed boot chain | Not signed: disable Secure Boot (see [building.md](building.md#secure-boot)) | |

## Setup

`wincraft-setup` computes a complete plan before touching the disk and runs
it in the same passes as Windows Setup (`wincraft-setup plan --unattend`
prints it):

| Windows Setup | Wincraft Setup |
|---------------|----------------|
| **windowsPE**: `DiskConfiguration` with diskpart | GPT layout written with `sfdisk`: ESP (512 MiB), BIOS boot (1 MiB), swap (optional), root. One disk boots on both UEFI and legacy BIOS. |
| MSR partition | BIOS boot partition (GRUB's core image on GPT disks) |
| Pagefile | Swap partition, `auto` sizes it from RAM (2–8 GiB) like a "system managed" page file |
| **windowsPE**: `ImageInstall`, `DISM /Apply-Image` | `unsquashfs -f -d /mnt/wincraft LiveOS/rootfs.img` |
| `bcdboot C:\Windows` | `grub-install` (UEFI entry named like the answer file's `[bootloader] id`, plus the removable `\EFI\BOOT\BOOTX64.EFI` fallback) and `grub-mkconfig`; `os-prober` adds Windows to the menu |
| **offlineServicing** | OEM files and offline packages |
| **specialize**: computer name, `International-Core`, `RunSynchronous` | `/etc/fstab` (UUIDs chosen in advance), host name, locale, keyboard (console, X11, Plasma), time zone and hardware clock, machine ID, initramfs, boot manager, services, `[[run]] pass = "specialize"` |
| `ComputerName` `*` → `DESKTOP-XXXXXXX` | `[computer] name = "auto"` → `DESKTOP-XXXXXXX` |
| **oobeSystem**: `UserAccounts` | Users created with `useradd`; administrators join `wheel` and may use `sudo` |
| Built-in Administrator account disabled | `root` locked unless the answer file sets a password |
| `AutoLogon` | `[[users]] autologin = true` → SDDM autologin |
| `SetupComplete.cmd` | Executables in `/etc/wincraft/setupcomplete.d/` and `[[run]] pass = "setup-complete"`; failures are logged, not fatal |
| `Panther\setupact.log`, `setuperr.log` | `/var/log/wincraft/setupact.log`, `setuperr.log` (copied into the new system) |
| Implicit answer file search | Command line → `wincraft.unattend=` kernel parameter → media root → root of other removable drives → `/etc/wincraft/autounattend.toml` |
| Setup starts by itself when `autounattend.xml` is found | `[setup] auto_start = true` starts the unattended installation when the live desktop comes up |
| Hardware requirement checks | x86-64, a disk of at least 20 GiB, not the install media; the plan refuses anything else |

## The installed system

| Windows | Wincraft |
|---------|----------|
| Desktop, taskbar, Start menu | KDE Plasma with the `org.wincraft.desktop` layout: bottom taskbar, Start menu on the left, pinned programs, notification area, clock, Show Desktop corner |
| Window buttons (minimize, maximize, close on the right), Alt+Tab thumbnails | KWin defaults in `/etc/xdg/kwinrc` |
| File Explorer, Notepad, Calculator, Snipping Tool, Photos, Task Manager, Disk Management | Dolphin, Kate, KCalc, Spectacle, Gwenview, System Monitor, Partition Manager |
| Microsoft Store | Discover with Flathub (Flatpak) |
| Windows Update | `pacman -Syu` / Discover |
| Edge, Office, Media Player | Firefox, LibreOffice, VLC |
| Network places and sharing (SMB) | Dolphin `smb://`, folder sharing via kdenetwork-filesharing |
| Arial, Times New Roman, Courier New, Calibri, Cambria | Metric-compatible Liberation, Carlito and Caladea fonts |
| Services (`services.msc`) | OpenRC or dinit services (chosen when the ISO is built) |
| Double-clicking an `.exe`, `.msi`, `.lnk` or `.bat` | `wcrun` via the desktop's file associations; `./program.exe` in a terminal via binfmt_misc |
| `C:\`, `Program Files`, the registry | One Wine prefix per "Windows environment", `wcrun prefix open`, `wcrun tool regedit` |
| Control Panel, Add or Remove Programs | `wcrun tool control`, "Add or Remove Windows Programs" in the menu |
| DirectX | DXVK (Direct3D 9/10/11) and VKD3D-Proton (Direct3D 12) over Vulkan, picked automatically from the program's imports |

## What does not carry over

* **Kernel drivers** (anti-cheat, copy protection, hardware drivers shipped
  as `.sys` files) cannot run under Wine. `wcrun info` warns about them.
* **Secure Boot**: Artix's boot chain is not signed by Microsoft.
* **Editions and activation**: there is nothing to activate; editions are
  just different ISO profiles.
