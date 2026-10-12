# Gaming on Wincraft

Wincraft is set up for games out of the box: Windows games run through Wine,
Proton and DXVK, and the system is tuned for low latency. This page lists
what is included and why.

## Choices

| Area | Wincraft default | Why |
|------|------------------|-----|
| Init system | **OpenRC** with parallel start (`rc_parallel="YES"`); runit and dinit builds are available | OpenRC is Artix's most used init. It applies `binfmt.d`, `sysctl.d` and `modules-load.d` natively, which the tuning below relies on, and starting services in parallel keeps boot fast. runit boots about as fast but leaves some of those files to its own scripts. |
| Kernel | **linux-zen** | Arch's kernel variant tuned for desktop responsiveness and low latency; `-k linux` builds with the stock kernel |
| Desktop | Wincraft Rust shell on **labwc** | A light Wayland compositor: little RAM and CPU left for games. Plasma stays available as a fallback session. |
| Swap | **zram** (compressed swap in RAM, up to 8 GiB) | Avoids stutter from swapping to disk when a game fills memory |

## Included software

| Package | Use |
|---------|-----|
| Steam (with Proton) | Steam games; turn on Proton for all titles in Steam → Settings → Compatibility |
| Lutris | GOG, Epic, Battle.net, emulators and other launchers |
| Wine, winetricks | Windows programs and games outside Steam (`wcrun`) |
| GameMode | Performance governor, I/O priority and GPU performance level while a game runs |
| MangoHud | FPS, frame time, CPU/GPU load overlay |
| gamescope | Valve's gaming compositor: frame limiting, FSR upscaling, Gaming Mode |
| Mesa, Vulkan (AMD, Intel, NVK) + 32-bit | Graphics drivers, including for 32-bit games |
| NVIDIA driver | Installed by Setup on GeForce GTX 16 / RTX and newer when online (see below) |

## Gaming Mode

The sign-in screen offers **Wincraft Gaming Mode**: Steam's Big Picture
inside gamescope, like SteamOS. It suits controllers and TVs. Leaving Big
Picture (Steam menu → Exit) returns to the sign-in screen.

## Running Windows games with wcrun

`wcrun` recognises games from what they import (Direct3D, OpenGL, Vulkan,
XInput) and then:

* installs DXVK (Direct3D 9/10/11) or VKD3D-Proton (Direct3D 12) into the
  prefix once;
* starts the game under GameMode (`gamemode = "auto"`).

```sh
wcrun Game.exe                       # GameMode automatically
wcrun run --hud Game.exe             # MangoHud overlay
wcrun run --gamescope Game.exe       # inside gamescope (fullscreen)
wcrun run --gamemode never Game.exe  # without GameMode
```

Per-game settings go in `~/.config/wincraft/wcrun.toml`:

```toml
[[app]]
match = "Game*.exe"
gamescope = true
gamescope_args = ["-W", "2560", "-H", "1440", "-r", "165", "--adaptive-sync"]
```

## System tuning

| Setting | File | Effect |
|---------|------|--------|
| `vm.max_map_count = 2147483642` | `/etc/sysctl.d/80-wincraft-gaming.conf` | Games that map many memory areas no longer crash (SteamOS value) |
| `kernel.split_lock_mitigate = 0` | same | No deliberate slowdown of games that perform split locks |
| `kernel.nmi_watchdog = 0`, `nowatchdog`, watchdog modules blacklisted | sysctl, kernel command line, `/usr/lib/modprobe.d/wincraft-nowatchdog.conf` | Fewer periodic interrupts |
| `fs.inotify.max_user_watches` | sysctl | Launchers watching many files |
| I/O schedulers: NVMe none, SSD mq-deadline, HDD BFQ | `/usr/lib/udev/rules.d/60-wincraft-ioschedulers.rules` | Low latency while games stream assets |
| NTSYNC | `/etc/modules-load.d/ntsync.conf`, udev rule | Windows synchronisation in the kernel, used by Wine 11 and Proton |
| esync file limit | `/etc/security/limits.d/90-wincraft-esync.conf` | For Proton/wine-staging's esync |
| zram + `vm.swappiness = 180` | written by Setup | Compressed swap in RAM; `zswap.enabled=0` avoids double compression |
| Quiet boot, `loglevel=3` | kernel command line (Setup) | Faster, quieter boot |
| Variable refresh rate and tearing | `/usr/share/wincraft/labwc/rc.xml` (`adaptiveSync`, `allowTearing`) | FreeSync/G-Sync for fullscreen games, lowest input latency |
| Parallel service start | `/etc/rc.conf` (Setup, OpenRC) | Faster boot |

Extra kernel parameters can be added in the answer file
(`[performance] kernel_parameters`) or in `/etc/default/grub` followed by
`sudo grub-mkconfig -o /boot/grub/grub.cfg`.

## NVIDIA

Setup reads the PCI IDs of the graphics cards. For an NVIDIA GPU of the
Turing generation or newer (GeForce GTX 16, RTX 20 and later), it installs
`nvidia-open-dkms`, `nvidia-utils`, `lib32-nvidia-utils` and
`nvidia-settings` when the network is reachable (this also updates the
system). Without a network connection, or with `[drivers] nvidia = "never"`,
the open-source nouveau/NVK driver is used; install the packages later with
`sudo pacman -Syu nvidia-open-dkms nvidia-utils lib32-nvidia-utils`. Older
NVIDIA cards are not supported by NVIDIA's current driver and keep nouveau.

## Intel graphics and low-end PCs

What to expect, honestly:

* **Intel Iris Xe, Arc and newer (Mesa's ANV driver):** Vulkan 1.3, so
  DXVK 2 and VKD3D-Proton work. Older and lighter games (Direct3D 9-11,
  esports titles, indie games) run well; recent AAA games are limited by the
  GPU itself, as on Windows. Intel's Linux drivers are mature: in
  Phoronix's 2025 comparison on Lunar Lake, Linux roughly matched Windows 11,
  ahead in some tests and behind in others.
* **Intel HD Graphics 520/620 and other Skylake-era GPUs:** Vulkan 1.3
  through ANV; fine for older and light games at low settings and 720p.
* **Ivy Bridge, Haswell, Broadwell (HD 2500-6000):** Mesa's HASVK driver,
  which is not a conformant Vulkan 1.3 driver. `wcrun` asks `vulkaninfo`
  which version the GPU offers; below 1.3 it installs **DXVK 1.10.3** (the
  last release for Vulkan 1.1/1.2) instead of DXVK 2, and tells you that
  Direct3D 12 games cannot run. Expect old games only.
* **Weak CPUs:** DXVK compiles shaders while you play, which causes stutter
  the first time effects appear. Steam's shader pre-caching (Settings →
  Downloads) removes most of it for Steam games.

What Wincraft does for low-end machines:

* A light desktop (labwc + the Rust shell) leaves more RAM and CPU to the
  game than Plasma or Windows would.
* zram doubles as cheap extra memory: on a 4 GB PC, swapping to compressed
  RAM is much faster than to disk.
* GameMode switches the CPU to its performance governor while a game runs.
* Hardware video decoding (`intel-media-driver`, `libva-intel-driver`) takes
  video playback off the CPU.
* The **Lite** ISO (about 500-600 MB, see [building.md](building.md#the-lite-edition))
  is quicker to download and fits small USB sticks. It installs the same
  system as the full ISO (Setup downloads the games stack), so games run
  exactly as fast with either.

Getting more frames on weak GPUs:

```toml
# ~/.config/wincraft/wcrun.toml
[[app]]
match = "Game.exe"
gamescope = true
# Render at 720p, upscale to 1080p with AMD FSR, cap at 60 FPS.
gamescope_args = ["-w", "1280", "-h", "720", "-W", "1920", "-H", "1080", "-F", "fsr", "-r", "60", "-f"]
```

In Steam, the same works per game with launch options such as
`gamescope -w 1280 -h 720 -W 1920 -H 1080 -F fsr -f -- %command%`.
Lower in-game resolution, shadows and post-processing first; MangoHud's
`fps_limit` (in `~/.config/MangoHud/MangoHud.conf`) keeps frame times steady.

## Checking the setup

`wcrun doctor` reports Wine, winetricks, Vulkan, NTSYNC, GameMode, MangoHud,
gamescope, split lock mitigation, zram, the open file limit,
`vm.max_map_count` and binfmt support.

## Limits

Games whose anti-cheat requires a Windows kernel driver do not run unless
the developer enables Proton support. Check [ProtonDB](https://www.protondb.com/)
for per-game reports.
