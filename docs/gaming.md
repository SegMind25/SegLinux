# Gaming on SegLinux

SegLinux is set up for games out of the box: Windows games run through Wine,
Proton and DXVK, and the system is tuned for low latency. This page lists
what is included and why.

## Settings → Gaming

Open **Settings** (Super+I) → **Gaming**. Everything applies to the next game
started; the display settings apply at once.

- **Windows games and programs** (segrun): the runner, chosen from the
  installed ones (system Wine, umu-launcher, and every Proton build Steam
  installed or that is in `compatibilitytools.d`, such as GE-Proton) or typed;
  the shared Windows environment; a separate environment per installer;
  GameMode, DXVK and VKD3D-Proton (automatic, always, never); esync, fsync
  and Wine's debug output.
- **Performance overlay**: MangoHud in Windows games, its position and detail
  (frame rate only, one line, standard, detailed), and a frame rate limit
  for every game. Right Shift+F12 shows or hides it while playing.
- **gamescope**: run Windows games in gamescope, fullscreen or borderless; the
  game's and the output's resolution (upscaling when the output is larger),
  a frame rate limit, the upscaling filter (FSR, NIS, integer scaling,
  nearest) and its sharpness, variable refresh rate and HDR, plus any other
  gamescope arguments (one per line). Zero leaves a value to gamescope.
- **Display**: variable refresh rate (FreeSync, G-Sync) for fullscreen games
  or always, and tearing when a game asks for it, for the lowest latency.
- **Gaming Mode**: the resolution, refresh rate, variable refresh rate, HDR
  and performance overlay of the *SegLinux Gaming Mode* session.
- **Environment variables** for every Windows program, such as
  `PROTON_ENABLE_NVAPI=1` (DLSS on NVIDIA cards).
- **Game profiles**: settings for one program, matched by its file name
  (`Game.exe`, `*Launcher*.exe`): runner, environment, GameMode, DXVK,
  VKD3D-Proton, gamescope, a Wine virtual desktop, DLL overrides and
  variables. Fields a profile does not set keep the general settings.
- **Installed gaming tools**: which of Wine, umu-launcher, GameMode,
  MangoHud, gamescope, Steam and Lutris are installed.

segrun's settings are saved in `~/.config/seglinux/segrun.toml`; settings
Settings has no control for (such as a profile's `args`) are kept. **Restore
page defaults** resets the settings but keeps game profiles and variables.
Steam and Lutris keep their own settings for the games they start.

## Choices

| Area | SegLinux default | Why |
|------|------------------|-----|
| Init system | **OpenRC** with parallel start (`rc_parallel="YES"`); runit and dinit builds are available | OpenRC is Artix's most used init. It applies `binfmt.d`, `sysctl.d` and `modules-load.d` natively, which the tuning below relies on, and starting services in parallel keeps boot fast. runit boots about as fast but leaves some of those files to its own scripts. |
| Kernel | **linux-zen** | Arch's kernel variant tuned for desktop responsiveness and low latency; `-k linux` builds with the stock kernel |
| Desktop | SegLinux Rust shell on **labwc** | A light Wayland compositor: little RAM and CPU left for games. Plasma stays available as a fallback session. |
| Swap | **zram** (compressed swap in RAM, up to 8 GiB) | Avoids stutter from swapping to disk when a game fills memory |

## Included software

| Package | Use |
|---------|-----|
| Steam (with Proton) | Steam games; turn on Proton for all titles in Steam → Settings → Compatibility |
| Lutris | GOG, Epic, Battle.net, emulators and other launchers |
| Wine, winetricks | Windows programs and games outside Steam (`segrun`) |
| GameMode | Performance governor, I/O priority and GPU performance level while a game runs |
| MangoHud | FPS, frame time, CPU/GPU load overlay |
| gamescope | Valve's gaming compositor: frame limiting, FSR upscaling, Gaming Mode |
| Mesa, Vulkan (AMD, Intel, NVK) + 32-bit | Graphics drivers, including for 32-bit games |
| NVIDIA driver | Installed by Setup on GeForce GTX 16 / RTX and newer when online (see below) |

## Gaming Mode

The sign-in screen offers **SegLinux Gaming Mode**: Steam's Big Picture
inside gamescope, like SteamOS. It suits controllers and TVs. Leaving Big
Picture (Steam menu → Exit) returns to the sign-in screen.

## Running Windows games with segrun

`segrun` recognises games from what they import (Direct3D, OpenGL, Vulkan,
XInput) and then:

* installs DXVK (Direct3D 9/10/11) or VKD3D-Proton (Direct3D 12) into the
  prefix once;
* starts the game under GameMode (`gamemode = "auto"`).

```sh
segrun Game.exe                       # GameMode automatically
segrun run --hud Game.exe             # MangoHud overlay
segrun run --gamescope Game.exe       # inside gamescope (fullscreen)
segrun run --gamemode never Game.exe  # without GameMode
```

Per-game settings go in `~/.config/seglinux/segrun.toml`:

```toml
[[app]]
match = "Game*.exe"
gamescope = true
gamescope_args = ["-W", "2560", "-H", "1440", "-r", "165", "--adaptive-sync"]
```

## System tuning

| Setting | File | Effect |
|---------|------|--------|
| `vm.max_map_count = 2147483642` | `/etc/sysctl.d/80-seglinux-gaming.conf` | Games that map many memory areas no longer crash (SteamOS value) |
| `kernel.split_lock_mitigate = 0` | same | No deliberate slowdown of games that perform split locks |
| `nmi_watchdog=0`, `nowatchdog`, watchdog modules blacklisted | kernel command line (Setup), `/usr/lib/modprobe.d/seglinux-nowatchdog.conf` | Fewer periodic interrupts |
| `fs.inotify.max_user_watches` | sysctl | Launchers watching many files |
| I/O schedulers: NVMe none, SSD mq-deadline, HDD BFQ | `/usr/lib/udev/rules.d/60-seglinux-ioschedulers.rules` | Low latency while games stream assets |
| NTSYNC | `/etc/modules-load.d/10-ntsync.conf`, udev rule | Windows synchronisation in the kernel, used by Wine 11 and Proton |
| esync file limit | `/etc/security/limits.d/90-seglinux-esync.conf` | For Proton/wine-staging's esync |
| zram + `vm.swappiness = 180` | written by Setup | Compressed swap in RAM; `zswap.enabled=0` avoids double compression |
| Quiet boot, `loglevel=3` | kernel command line (Setup) | Faster, quieter boot |
| Variable refresh rate and tearing | `/usr/share/seglinux/labwc/rc.xml` (`adaptiveSync`, `allowTearing`) | FreeSync/G-Sync for fullscreen games, lowest input latency |
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
  which is not a conformant Vulkan 1.3 driver. `segrun` asks `vulkaninfo`
  which version the GPU offers; below 1.3 it installs **DXVK 1.10.3** (the
  last release for Vulkan 1.1/1.2) instead of DXVK 2, and tells you that
  Direct3D 12 games cannot run. Expect old games only.
* **Weak CPUs:** DXVK compiles shaders while you play, which causes stutter
  the first time effects appear. Steam's shader pre-caching (Settings →
  Downloads) removes most of it for Steam games.

What SegLinux does for low-end machines:

* A light desktop (labwc + the Rust shell) leaves more RAM and CPU to the
  game than Plasma or Windows would.
* zram doubles as cheap extra memory: on a 4 GB PC, swapping to compressed
  RAM is much faster than to disk.
* GameMode switches the CPU to its performance governor while a game runs,
  and on integrated GPUs, when the GPU is the busy part, lowers the CPU's
  share of the power budget so the GPU can clock higher (`/etc/gamemode.ini`).
* Mesa's shader cache may grow to 4 GB (`MESA_SHADER_CACHE_MAX_SIZE`, set
  in `/etc/profile.d/seglinux-gaming.sh`): DXVK 2 and VKD3D-Proton keep
  compiled shaders there, and the 1 GB default makes games recompile them
  (stutter) once several are installed.
* Hardware video decoding (`intel-media-driver`, `libva-intel-driver`) takes
  video playback off the CPU.
* The **Lite** ISO (well under 1 GB, see [building.md](building.md#the-lite-edition))
  is quicker to download and fits small USB sticks. It installs the same
  system as the full ISO (Setup downloads the games stack), so games run
  exactly as fast with either.

Getting more frames on weak GPUs:

```toml
# ~/.config/seglinux/segrun.toml
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

More on Intel GPUs:

* **Two memory modules.** An integrated GPU uses system RAM as video
  memory, so dual-channel memory (two modules instead of one) often gives
  noticeably more frames.
* **Plugged in, performance profile:** `powerprofilesctl set performance`.
* **Ivy Bridge and Haswell (HD 2500-5000):** the kernel's i915 driver
  clears GPU state between programs on these generations to fix an
  information leak (CVE-2019-14615), which costs them a large part of their
  speed. `i915.mitigations=off` on the kernel command line turns that off:
  much faster, but programs on the GPU can then read each other's leftovers.
  Your choice; in an answer file:
  `[performance] kernel_parameters = ["i915.mitigations=off"]`.
* **Check that the GPU renders:** `segrun doctor` shows the Vulkan driver.
  A virtual machine without 3D acceleration only has Mesa's software
  renderer, which runs the desktop but not games.

## Checking the setup

`segrun doctor` reports Wine, winetricks, Vulkan, NTSYNC, GameMode, MangoHud,
gamescope, split lock mitigation, zram, the open file limit,
`vm.max_map_count` and binfmt support.

## Limits

Games whose anti-cheat requires a Windows kernel driver do not run unless
the developer enables Proton support. Check [ProtonDB](https://www.protondb.com/)
for per-game reports.
