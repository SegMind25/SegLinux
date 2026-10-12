# Wincraft

**Wincraft** is a Linux distribution based on [Artix Linux](https://artixlinux.org)
(Arch without systemd) for gaming: a lightweight desktop that looks, installs
and behaves like Windows, runs Windows games and programs, and is tuned for
low latency. Its own tools are written in Rust.

* **Installs like Windows.** The ISO is built the way a Windows ISO is (see
  [how a Windows ISO works](docs/windows-iso-anatomy.md)): a live image
  plays the role of WinPE, a squashfs system image plays `install.wim`, and
  `wincraft-setup` plays `setup.exe`. It runs the same configuration passes
  (windowsPE, offlineServicing, specialize, oobeSystem, SetupComplete) and
  reads `autounattend.toml`, the counterpart of `autounattend.xml`.
* **Lightweight, looks like Windows.** A dark Wincraft shell written in Rust,
  with a searchable application launcher, taskbar and desktop preferences, on
  the labwc Wayland compositor; KDE Plasma remains available as a fallback.
  Familiar applications (File Explorer → Dolphin, Store → Discover, ...).
* **Built for gaming.** linux-zen kernel, Steam with Proton, Lutris,
  GameMode, MangoHud and gamescope; a Steam "Gaming Mode" session; variable
  refresh rate and tearing for fullscreen games; zram swap, NTSYNC, I/O
  scheduler and sysctl tuning; NVIDIA's driver installed automatically on
  GeForce GTX 16/RTX cards. See [docs/gaming.md](docs/gaming.md).
* **Runs Windows programs.** Double-click an `.exe`, `.msi`, `.lnk` or
  `.bat`, or type `./setup.exe`: `wcrun` inspects the program, prepares a
  Wine prefix (with DXVK/VKD3D-Proton when the program uses Direct3D) and
  starts it, under GameMode when it is a game.
* **Artix underneath.** OpenRC by default (parallel boot), or runit or dinit,
  instead of systemd; Artix and Arch packages, rolling release.

## Repository layout

```
crates/
  wincraft-shell/     Rust/GTK4 desktop, launcher, taskbar, settings and Wayland integration
  wincraft-pe/        Windows file identification: PE headers, imports, .NET,
                      installer frameworks, MSI, shortcuts (no dependencies)
  wincraft-core/      commands, XDG paths, offline service management (OpenRC/dinit/runit/s6)
  wcrun/              the Windows program launcher
  wincraft-setup/     the installer: answer files, planning, execution, text-mode wizard
data/                 defaults installed by the wincraft package (Plasma layout,
                      file associations, binfmt, NTSYNC, sysctl, branding)
packaging/wincraft/   PKGBUILD for the wincraft package
iso/
  build.sh            builds the ISO (container or native Artix)
  Containerfile       Artix build environment
  iso-profiles/       artools profiles: common.yaml, wincraft/ (full) and
                      wincraft-lite/ (small image), with their overlays
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
and writes `iso/out/wincraft-*.iso`. The Lite edition boots the same Rust
desktop from a much smaller image and installs the same system, downloading
Wine, Steam, Firefox and the drivers during Setup (so it needs internet to
install). See [docs/building.md](docs/building.md)
for step-by-step instructions for your PC, native builds, customisation and
testing in QEMU. Ready-made ISOs are built by the **ISO** workflow and
published under Releases.

## Preview the Rust desktop

With GTK4 and gtk4-layer-shell development libraries installed:

```sh
cargo run -p wincraft-shell -- --preview --wallpaper "$PWD/data/artwork/wincraft-dark.png"
```

The preview opens a regular window. The packaged **Wincraft (Rust)** session
uses labwc and a real Wayland panel with running-window buttons. See
[the desktop guide](docs/desktop.md) for shortcuts, configuration and limitations.

## Use the tools

```sh
cargo build --release

# What is this file, and how would Wincraft run it?
target/release/wcrun info setup.exe
target/release/wcrun run --dry-run setup.exe

# Install Wincraft (in the live session, as root)
wincraft-setup                                   # interactive
wincraft-setup install --unattend                # find autounattend.toml and install
wincraft-setup plan --unattend answer.toml       # dry run: every command and file
wincraft-setup validate answer.toml
```

* [Running Windows programs](docs/running-windows-apps.md)
* [Unattended installation](docs/unattended-install.md)
* [How a Windows ISO works](docs/windows-iso-anatomy.md) and
  [how Wincraft maps it](docs/windows-to-wincraft.md)

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

Wincraft does not include any Microsoft software. Windows programs run with
Wine and Proton, so programs that need Windows kernel drivers (for example
most anti-cheat systems) do not work.

No license has been chosen for this repository yet.
