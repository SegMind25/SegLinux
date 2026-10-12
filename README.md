# SegLinux

**SegLinux** is a lightweight Linux distribution for gaming, without
systemd: a desktop that looks, installs and behaves like Windows, runs
Windows games and programs, and is tuned for low latency. Its own tools are
written in Rust.

* **Installs like Windows.** The ISO is built the way a Windows ISO is (see
  [how a Windows ISO works](docs/windows-iso-anatomy.md)): a live image
  plays the role of WinPE, a squashfs system image plays `install.wim`, and
  `seglinux-setup` plays `setup.exe`. It runs the same configuration passes
  (windowsPE, offlineServicing, specialize, oobeSystem, SetupComplete) and
  reads `autounattend.toml`, the counterpart of `autounattend.xml`. In the
  live session **Install SegLinux** opens a graphical GTK 4 Setup wizard
  (a text-mode one is in the menu too).
* **Lightweight, Plasma-like desktop in Rust.** Wallpaper and desktop icons,
  a floating dock with Start menu, one icon per application, tray and clock,
  written in Rust (GTK 4) on the labwc Wayland compositor, and the default
  session; KDE Plasma remains available as a fallback.
  Familiar applications (File Explorer → Dolphin, Store → Discover, ...).
* **Built for gaming.** linux-zen kernel, Steam with Proton, Lutris,
  GameMode, MangoHud and gamescope; a Steam "Gaming Mode" session; variable
  refresh rate and tearing for fullscreen games; zram swap, NTSYNC, I/O
  scheduler and sysctl tuning; NVIDIA's driver installed automatically on
  GeForce GTX 16/RTX cards. See [docs/gaming.md](docs/gaming.md).
* **Runs Windows programs.** Double-click an `.exe`, `.msi`, `.lnk` or
  `.bat`, or type `./setup.exe`: `segrun` inspects the program, prepares a
  Wine prefix (with DXVK/VKD3D-Proton when the program uses Direct3D) and
  starts it, under GameMode when it is a game.
* **Fast, systemd-free base.** OpenRC by default (parallel boot), or runit
  or dinit; rolling release.

## Repository layout

```
crates/
  seglinux-shell/     Rust/GTK4 desktop, launcher, taskbar, settings and Wayland integration
  seglinux-pe/        Windows file identification: PE headers, imports, .NET,
                      installer frameworks, MSI, shortcuts (no dependencies)
  seglinux-core/      commands, XDG paths, offline service management (OpenRC/dinit/runit/s6)
  segrun/             the Windows program launcher
  seglinux-setup/     the installer: answer files, planning, execution, text-mode wizard
  seglinux-installer/ the graphical Setup wizard (GTK 4), which runs seglinux-setup
data/                 defaults installed by the seglinux package (Plasma layout,
                      file associations, binfmt, NTSYNC, sysctl, branding)
packaging/seglinux/   PKGBUILD for the seglinux package
iso/
  build.sh            builds the ISO (container or native Artix)
  Containerfile       Artix build environment
  iso-profiles/       artools profiles: common.yaml, seglinux/ (full) and
                      seglinux-lite/ (small image), with their overlays
  pacman.conf.d/      pacman configuration used by buildiso
  scripts/            build and package-check scripts
docs/                 design notes and guides
```

## Build the ISO

```sh
./iso/build.sh                 # OpenRC + linux-zen (recommended)
./iso/build.sh -i runit        # or runit / dinit; -k linux for the stock kernel
./iso/build.sh --lite          # Lite edition: smaller image, Setup downloads the apps
```

Runs in a privileged Artix container (docker or podman) on any Linux host
and writes `iso/out/seglinux-*.iso`. The Lite edition boots the same Rust
desktop from a much smaller image and installs the same system, downloading
Wine, Steam, Firefox and the drivers during Setup (so it needs internet to
install). See [docs/building.md](docs/building.md)
for step-by-step instructions for your PC, native builds, customisation and
testing in QEMU. Ready-made ISOs are built by the **ISO** workflow and
published under Releases.

## Preview the Rust desktop

With GTK4 and gtk4-layer-shell development libraries installed:

```sh
cargo run -p seglinux-shell -- --preview --wallpaper "$PWD/data/artwork/seglinux-dark.jpg"
```

The preview opens a regular window. The packaged **SegLinux (Rust)** session
uses labwc and a real Wayland panel with running-window buttons. See
[the desktop guide](docs/desktop.md) for shortcuts, configuration and limitations.

## Use the tools

```sh
cargo build --release

# What is this file, and how would SegLinux run it?
target/release/segrun info setup.exe
target/release/segrun run --dry-run setup.exe

# Install SegLinux (in the live session)
seglinux-installer                               # graphical wizard
target/release/seglinux-installer --demo         # try the wizard anywhere; installs nothing
sudo seglinux-setup                              # text-mode wizard
seglinux-setup install --unattend                # find autounattend.toml and install
seglinux-setup plan --unattend answer.toml       # dry run: every command and file
seglinux-setup validate answer.toml
```

* [Running Windows programs](docs/running-windows-apps.md)
* [Unattended installation](docs/unattended-install.md)
* [How a Windows ISO works](docs/windows-iso-anatomy.md) and
  [how SegLinux maps it](docs/windows-to-seglinux.md)

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
shellcheck iso/build.sh iso/scripts/*.sh
iso/scripts/check-packages.sh -i openrc     # on Artix / in the build container
```

## Status

The Rust installer and Windows-launcher tools have unit tests. The new
Rust desktop is an early implementation, not a feature-complete KDE replacement. The installer's plan
is tested end to end in dry-run form, and its file and service actions are
tested against temporary directories; the disk-writing steps need a real
or virtual machine. The ISO profile follows artools 0.39+'s format. CI
checks every package name against the Artix repositories, and the ISO
workflow builds the image. Before installing on real hardware, try a build
in a VM (see [docs/building.md](docs/building.md#testing-in-qemu)).

SegLinux does not include any Microsoft software. Windows programs run with
Wine and Proton, so programs that need Windows kernel drivers (for example
most anti-cheat systems) do not work.

## Credits

SegLinux is built from the packages of [Artix Linux](https://artixlinux.org)
and Arch Linux, with Artix's ISO tools (artools). It is an independent
project with its own name and artwork, not affiliated with or endorsed by
either. The boot menu font is made from [Inter](https://rsms.me/inter/)
(SIL Open Font License 1.1).

No license has been chosen for this repository yet.
