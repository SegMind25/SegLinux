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
| Kernel | **linux-zen**, with the stock `linux` kernel beside it as a boot menu entry | linux-zen is Arch's kernel variant tuned for desktop responsiveness and low latency (and has NTSYNC); `-k linux` builds with the stock kernel. Setup installs the other kernel too when online, so a bad update never leaves the machine without a kernel to boot (GRUB lists it under *Advanced options*). Neither is ever removed. |
| Desktop | SegLinux Rust shell on **labwc** | A light Wayland compositor: little RAM and CPU left for games. Plasma stays available as a fallback session. |
| Swap | **zram** (compressed swap in RAM, up to 8 GiB) | Avoids stutter from swapping to disk when a game fills memory |

## Included software

| Package | Use |
|---------|-----|
| Steam (with Proton) | Steam games; turn on Proton for all titles in Steam → Settings → Compatibility |
| Lutris | GOG, Epic, Battle.net, emulators and other launchers |
| Wine Staging, winetricks | Windows programs and games outside Steam (`segrun`); Staging adds the esync/fsync/NTSYNC patches and game fixes |
| GameMode | Performance governor, I/O priority and GPU performance level while a game runs |
| `seg game` | The same idea without a daemon, plus a readiness check and the driver packages for the GPUs found ([below](#seg-game)) |
| MangoHud | FPS, frame time, CPU/GPU load overlay for 64-bit games (the Artix repositories have no 32-bit MangoHud; 32-bit games get no overlay unless you build `lib32-mangohud` from the AUR) |
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

## seg game

`seg game` is part of [`seg`](seg.md), the small Rust tool that is already on
the system (`segctl` package). It works the same on OpenRC, runit, dinit and
s6, and it needs no daemon of its own.

```sh
sudo seg game setup --yes        # the graphics driver packages for the GPUs in this PC
seg game check                   # PASS / FAIL for everything games need, with the fix
seg game run -- ./game           # run a game with the computer set up for it
```

### Steam launch options

Right-click the game → Properties → General → Launch options:

| Goal | Launch options |
|---|---|
| Boost while the game runs | `seg game run -- %command%` |
| Boost, and MangoHud (64-bit games) | `seg game run -- mangohud %command%` |
| Boost, inside gamescope (render 1440p, 165 Hz, adaptive sync) | `seg game run -- gamescope -W 2560 -H 1440 -r 165 --adaptive-sync -f -- %command%` |
| A Proton variable as well | `PROTON_LOG=1 seg game run -- %command%` |
| Only the cheap parts (no governor or GPU change) | `seg game run --no-boost -- %command%` |
| Show what would happen and exit | `seg game run --dry-run -- %command%` |

For Lutris, put `seg game run --` in front of the runner's command (Configure →
System options → Command prefix: `seg game run --`). Games outside Steam:
`seg game run -- wine Game.exe` or `seg game run -- segrun Game.exe`.

### What `seg game run` does

While the game runs, and only then:

| Change | How | Why | Undone |
|---|---|---|---|
| CPU governor `performance` and energy preference `performance` | through the one root helper (`sudo -n seg game boost`, the `gamemode` group; or directly as root) | A dynamic governor can be slow to raise clocks on short bursts, which shows as low "1 % lows" | when the game ends |
| AMD GPU: `power_dpm_force_performance_level = high` | same helper | The driver can sit at a low clock while the CPU is the bottleneck, then ramp late | when the game ends |
| Game gets `nice -4` and best-effort I/O priority 0 | the game's own process (`nice -10` is allowed to the `gamemode` group by `limits.d`) | A little more CPU than a browser or a download in the background | dies with the game |
| Screen saver and idle blanking held off | `org.freedesktop.ScreenSaver.Inhibit` on the session bus (a hand-written D-Bus call; no library) | The screen does not go dark in the middle of a cutscene or with a controller | when the game ends |
| X3D CPUs with two chiplets: the game only uses the cores of the V-Cache chiplet | `--pin vcache` or `pin = "vcache"` in a profile; the chiplet is the one whose L3 cache is bigger (read from `/sys/devices/system/cpu/cpu*/cache`) | Games that like cache run on the cache; the other chiplet clocks higher but has less of it | dies with the game |
| Laptops with two GPUs: the game runs on the dedicated one | `__NV_PRIME_RENDER_OFFLOAD=1` (NVIDIA's driver) or `DRI_PRIME=pci-…` (AMD, Intel, nouveau), when the screen starts on a slower GPU; set `DRI_PRIME=0` yourself to keep a game on the integrated one | Without it, games run on the integrated GPU, at a fraction of the frame rate | dies with the game |
| `MESA_SHADER_CACHE_MAX_SIZE=10G`, `__GL_SHADER_DISK_CACHE_SIZE` | environment of the game | See [system tuning](#system-tuning) | dies with the game |

Several games can run at once: the helper keeps a count (`/run/seg/game-boost`,
mode 0600) of the game processes it was asked for, identified by process id
and start time, so a game that crashed does not keep the boost on and the
last one to end restores the settings that were found. The settings come
back when the game exits, on Ctrl+C, SIGTERM and SIGHUP, and when `seg`
itself panics. If the whole computer loses power the settings are runtime
state anyway: a reboot clears them.

What it does **not** do: it never changes anything permanently, never turns
on `mitigations=off`, never touches overclocking or voltages, and does not
set the governor to `performance` for the whole system.

### Per-game profiles

`~/.config/seg/games.toml`. A profile is picked when its `match` (a glob such
as `eldenring.exe` or `*Launcher*`) fits the name of a file in the command, so
a Proton command line matches the `.exe` at its end. The first profile that
matches wins. A mistake in the file is reported with its line number and the
game starts anyway, without profiles.

```toml
[[game]]
match = "eldenring.exe"
governor = "performance"          # or "keep" (no boost for this game), or a governor name
pin = "vcache"                    # "vcache", "off" or a CPU list such as "0-7,16-23"
mangohud = true
mangohud_config = "preset"        # SegLinux's preset (/usr/share/seg/mangohud-gaming.conf)
gamescope = ["-W", "2560", "-H", "1440", "-r", "165", "--adaptive-sync"]
nice = -5

[[game]]
match = "cs2"
pin = "0-7"
```

`seg game profiles` checks the file and lists what it found.

### Reading `seg game check`

Every line is `PASS` (fine), `WARN` (works, could be better), `FAIL` (games
may crash or run badly) or `SKIP` (cannot be checked here, for example no GPU
in a virtual machine). Each WARN and FAIL prints the exact command that fixes
it, and the exit status is 1 when anything fails. This is the real output of
a build container with no GPU and the stock kernel, to show the format:

```
[FAIL] vulkan          vulkaninfo did not run (not installed, or no Vulkan driver)
                       fix: sudo pacman -S vulkan-tools vulkan-icd-loader   (then: sudo seg game setup)
[SKIP] gpu driver      no graphics card found in /sys/class/drm (virtual machine?)
[FAIL] ntsync          /dev/ntsync does not exist: the module is not loaded
                       fix: sudo modprobe ntsync   (always: echo ntsync | sudo tee /etc/modules-load.d/10-ntsync.conf)
[WARN] compaction      vm.compaction_proactiveness = 20: the kernel defragments memory in the background, which can stall a game
                       fix: sudo sysctl -w vm.compaction_proactiveness=0   (always: /etc/sysctl.d/80-seglinux-gaming.conf)
[PASS] mitigations     the kernel's default security mitigations are on

3 passed, 4 to improve, 4 failed
```

| Check | Passes when | Why it matters |
|---|---|---|
| vulkan | `vulkaninfo` lists a real device | Nearly every current game (DXVK, VKD3D-Proton) needs Vulkan; only the software renderer (llvmpipe/lavapipe), or no `vulkaninfo`, is a FAIL |
| gpu driver | each GPU has its kernel driver: `amdgpu`, `i915`/`xe`, `nvidia` (or `nouveau` on a pre-Turing NVIDIA card, which has nothing better) | No driver bound, or `nouveau` on a card NVIDIA's driver supports, is a FAIL with `seg game setup --yes`; an unknown driver is a WARN |
| 32-bit drivers | the `lib32-` packages for the GPU are installed (and `[lib32]` is enabled) | Steam and many games are 32-bit |
| ntsync | the kernel is 6.14 or newer, `/dev/ntsync` exists and every user may open it | The kernel implementation of Windows synchronisation, used by Wine 11 and recent Proton |
| max_map_count | `vm.max_map_count` is at least 1048576 | Several games (and Proton) crash with "out of memory" below it |
| open files | the hard limit of open files is at least 524288 | esync keeps a file descriptor per Windows synchronisation object |
| kernel | `linux-zen`, or any other tuned kernel | A WARN, not a FAIL: the stock kernel works |
| fallback kernel | a second kernel is installed | If an update breaks one, the other boots |
| cpu governor | a dynamic governor at rest; SKIP without cpufreq (a VM) | `performance` all the time is a WARN: hot, loud and no better while idle. `seg game run` raises it only while a game runs |
| cpu driver (AMD CPUs only) | `amd-pstate` in active mode (`amd-pstate-epp`) | The CPU picks its own clocks, guided by the energy hint `seg game run` sets; otherwise a WARN with the kernel parameter |
| game boost | you are root, or `sudo -n -l` shows that the helper is allowed (member of `gamemode`) | Without it `seg game run` still starts the game but cannot change the governor or the GPU; the FAIL says whether `segctl` is missing, you are not in the group, or the sudoers file is broken |
| hugepages | transparent hugepages are `madvise` | `always` can stall on memory compaction |
| split lock | `kernel.split_lock_mitigate = 0` | Otherwise games that do split locks are slowed down on purpose |
| compaction | `vm.compaction_proactiveness = 0` | Background defragmentation can show as stutter |
| mitigations | `mitigations=off` is not on the kernel command line | A WARN if it is, with how to undo it |

### Other `seg game` commands

* `sudo seg game setup [--yes] [--games] [--lutris]` finds the graphics cards
  (`/sys/class/drm`, names from `lspci`) and lists the packages **for those
  cards only**: Mesa and the Vulkan driver plus their `lib32-` versions for
  AMD and Intel; NVIDIA's open kernel driver (`nvidia-open-dkms` with the
  headers of the running kernel, `nvidia-utils`, `lib32-nvidia-utils`,
  `nvidia-settings`) for GeForce GTX 16 / RTX and newer; the open-source NVK
  driver for older NVIDIA cards, because NVIDIA's current driver does not
  support them. `--yes` runs `pacman -S --needed --noconfirm` with the list;
  `--games` adds Steam, gamescope, MangoHud (64-bit only), Wine Staging and
  winetricks; `--lutris` adds Lutris. It also tells you when `[lib32]` is not
  enabled in `/etc/pacman.conf`.
* `sudo seg game apply` applies the gaming sysctls, sets transparent
  hugepages to `madvise` (from `always` only) and makes `/dev/ntsync`
  usable now; the `seg-game` service runs it at boot on every init.
* `seg game audio on|off|status` lowers PipeWire's buffer to 256 samples at
  48 kHz (about 5 ms), for you only (`~/.config/pipewire/...conf.d`). **Off by
  default**: it costs CPU and can make sound crackle on a weak or busy
  machine; `seg game audio off` puts the defaults back.
* `sudo seg game unsafe-boost on|off|status` is the only way to get
  `mitigations=off`. See [safety](#safety-notes).
* `sudo seg tune gaming` (a saved profile, like `light`/`balanced`/`performance`):
  `vm.swappiness` 10 without zram (180 with zram, the value Setup writes),
  `vm.vfs_cache_pressure` 50, shorter write-back, and the governor left
  dynamic, because `seg game run` raises it only while a game runs.

### Safety notes

* **No global unsafe tweaks.** Everything above is either a kernel setting
  that Valve's SteamOS and the gaming kernels also use, or lives only as long
  as a game runs. The governor is never `performance` all day on its own.
* **The root helper is narrow.** `/etc/sudoers.d/seg-game` lets the `gamemode`
  group (Setup puts every user in it; the `gamemode` package uses the same
  group) run exactly `/usr/bin/seg game boost on <pid>` and `boost off <pid>`.
  The program checks that the arguments are a process id and a governor that
  exists, and writes only the cpufreq and `amdgpu` power files. A member of
  the group can therefore change the CPU governor and the AMD GPU power level
  and nothing else; that is the same trust `gamemoded` asks for.
* **`mitigations=off` is opt-in and loud.** `sudo seg game unsafe-boost on`
  prints a warning and does nothing unless you add `--i-understand`; it
  edits `GRUB_CMDLINE_LINUX_DEFAULT`, rebuilds the boot menu and takes
  effect at the next boot. `off` undoes it, `status` says whether it is on,
  and `seg game check` shows a WARN while it is. Measure first (see below):
  on recent CPUs it is often within the run-to-run noise. Do not use it on
  a computer you share or use for banking.
* **The fallback kernel is never removed**, by Setup, `seg clean` or
  anything else here.
* **Drivers only for hardware that is there.** `seg game setup` and Setup read
  the PCI devices; with no NVIDIA card no NVIDIA package is selected.

## Measuring: stock versus `seg game run`

There are no benchmark numbers in this repository, on purpose: they depend on
the CPU, GPU, game, scene and driver. This is how to produce your own and
tell a real difference from noise. Use MangoHud's log, which records frame
times, so you get the average frame rate **and** the "1 % lows" (the
stutter), which is where a governor or a pinning change shows up.

1. **Fix everything else.** Same game, same settings, same resolution, same
   scene (the game's built-in benchmark, or the same 60 seconds walked the
   same way), plugged in, nothing else running, the display's refresh rate
   unchanged. Turn the in-game frame limiter off for both runs, or set it
   the same.
2. **Warm the shader cache** by playing the scene once and discarding that
   run, otherwise the first run measures shader compilation.
3. **Log the frame times.** In Steam's launch options, run A (stock) and run B:

   ```
   A: MANGOHUD_CONFIG="output_folder=$HOME/bench/stock,log_interval=0,autostart_log=15,log_duration=60,no_display" mangohud %command%
   B: MANGOHUD_CONFIG="output_folder=$HOME/bench/seg,log_interval=0,autostart_log=15,log_duration=60,no_display" seg game run -- mangohud %command%
   ```

   `autostart_log=15,log_duration=60` records 60 seconds starting 15 seconds
   after launch, so load screens are left out; `log_interval=0` writes one
   row per frame. (A MangoHud version that ignores it writes a row every
   100 ms: still fine for comparing, less exact for lows; use the same
   version for both.) `mkdir -p ~/bench/stock ~/bench/seg` first.
   `seg game run --no-boost -- ...` is a third variant that shows how much
   of any difference is the governor and GPU level and how much the rest.
4. **Repeat at least five times each, alternating** (A B A B ...), so that
   thermals and background tasks hit both alike. Reboot between the two
   sessions only if you suspect a leak.
5. **Summarise each log.** MangoHud's CSV has a few lines of system
   information, then a header line starting with `fps,` and one row per
   frame; this reads the `frametime` column (milliseconds):

   ```sh
   summarise() {   # usage: summarise ~/bench/stock/*.csv
     for f in "$@"; do
       awk -F, '/^fps,/ { for (i = 1; i <= NF; i++) if ($i == "frametime") c = i; next }
                c && $c + 0 > 0 { print $c }' "$f" | LC_ALL=C sort -n |
       awk -v name="$f" '
         { ft[NR] = $1; sum += $1 }
         END {
           n = NR
           if (!n) { print name ": no frames"; exit }
           k1 = int(n * 0.99);  if (k1 < 1) k1 = 1
           k01 = int(n * 0.999); if (k01 < 1) k01 = 1
           for (i = k1 + 1; i <= n; i++) { s1 += ft[i]; n1++ }       # slowest 1 %
           for (i = k01 + 1; i <= n; i++) { s01 += ft[i]; n01++ }    # slowest 0.1 %
           printf "%s  frames=%d  avg=%.1f fps  1%%low=%.1f fps  0.1%%low=%.1f fps  p99=%.2f ms  max=%.2f ms\n",
             name, n, 1000 * n / sum, n1 ? 1000 * n1 / s1 : 0, n01 ? 1000 * n01 / s01 : 0, ft[k1], ft[n]
         }'
     done
   }
   summarise ~/bench/stock/*.csv; summarise ~/bench/seg/*.csv
   ```

   (Any awk will do.) "1 % low" here is the frame rate
   of the slowest 1 % of frames taken together; other tools define it as
   the 99th-percentile frame time, so only compare numbers made by the same
   script.
6. **Read the result honestly.** Take the median of each side. A difference
   smaller than the spread between runs of the *same* side is noise. The
   average rarely moves; if `seg game run` helps, it is usually the 1 % and
   0.1 % lows and the p99 frame time. On a PC that already runs the
   `performance` governor all the time, expect no difference at all; the
   benefit is the same performance without paying for it when idle.

For `mitigations=off`, repeat the A/B across a reboot with and without
`seg game unsafe-boost on`; if the lows do not move by more than the spread,
leave it off.

## System tuning

| Setting | File | Effect |
|---------|------|--------|
| `vm.max_map_count = 2147483642` | `/etc/sysctl.d/80-seglinux-gaming.conf` | Games that map many memory areas no longer crash (SteamOS value) |
| `kernel.split_lock_mitigate = 0` | same | No deliberate slowdown of games that perform split locks |
| `nmi_watchdog=0`, `nowatchdog`, watchdog modules blacklisted | kernel command line (Setup), `/usr/lib/modprobe.d/seglinux-nowatchdog.conf` | Fewer periodic interrupts |
| `vm.compaction_proactiveness = 0` | same | No background memory defragmentation while a game allocates (the default is 20); a program that needs a contiguous block still gets it by compacting on demand. What SteamOS sets. |
| `fs.inotify.max_user_watches` | sysctl | Launchers watching many files |
| `transparent_hugepage=madvise` | kernel command line (Setup), and `seg game apply` at boot | Huge pages only for programs that ask (games, databases); `always` can stall on compaction. linux-zen starts with `always`, so the live image and a system upgraded with `seglinux-upgrade` get `madvise` from the `seg-game` service (it changes `always` only; `never` is left alone). Found by booting the Lite image: the check said `[always]` before this. |
| `amd_pstate=active` | kernel command line (Setup, AMD processors only) | The CPU picks its frequency itself, guided by the energy hint `seg game run` sets. Kernels or CPUs without it ignore it. |
| I/O schedulers: NVMe none, SSD mq-deadline, HDD BFQ | `/usr/lib/udev/rules.d/60-seglinux-ioschedulers.rules` | Low latency while games stream assets |
| NTSYNC | `/etc/modules-load.d/10-ntsync.conf`, udev rule | Windows synchronisation in the kernel, used by Wine 11 and Proton |
| Open files `nofile` hard limit 1048576 | `/etc/security/limits.d/90-seglinux-esync.conf` | esync/fsync keep a file descriptor per Windows synchronisation object (1048576 is the kernel's maximum); Wine raises its own soft limit up to it |
| `nice -10` for the `gamemode` group | same file | Lets `seg game run` raise a game's priority |
| Shader caches 10 GB: `MESA_SHADER_CACHE_MAX_SIZE`, `__GL_SHADER_DISK_CACHE_SIZE`, `__GL_SHADER_DISK_CACHE_SKIP_CLEANUP=1` | `/etc/profile.d/seglinux-gaming.sh` (every init; a value you set wins) | With several games installed, Mesa's 1 GB default evicts compiled shaders and games stutter while recompiling; NVIDIA's driver deletes its cache when full unless told not to. Disk space only. |
| `nvidia_drm modeset=1 fbdev=1` | `/usr/lib/modprobe.d/seglinux-nvidia.conf` | Needed by Wayland compositors and a proper VT switch; does nothing without NVIDIA's driver |
| THP stays `madvise`, mitigations stay on | kernel defaults | Deliberately not tuned further; see [safety](#safety-notes) |
| Applied at boot by the `seg-game` service | `seg game apply` | The sysctls and `/dev/ntsync` permissions on OpenRC, runit, dinit and s6 alike |
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
`sudo pacman -Syu nvidia-open-dkms nvidia-utils lib32-nvidia-utils`, or
`sudo seg game setup --yes`, which also adds the headers of the running kernel
that DKMS needs. Older NVIDIA cards (before Turing) are not supported by
NVIDIA's current driver and keep the open-source driver (NVK). NVK cannot
raise the clocks of GTX 750 to GTX 1080 Ti cards, so games run slowly there;
`seg game setup` names NVIDIA's last driver for them, in the AUR
(`yay -S nvidia-580xx-dkms nvidia-580xx-utils lib32-nvidia-580xx-utils`). The
driver is loaded with `nvidia_drm modeset=1 fbdev=1` and
`NVreg_UsePageAttributeTable=1` (faster CPU-to-GPU transfers)
(`/usr/lib/modprobe.d/seglinux-nvidia.conf`).

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
* Mesa's shader cache may grow to 10 GB (`MESA_SHADER_CACHE_MAX_SIZE`, set
  in `/etc/profile.d/seglinux-gaming.sh`, which also sizes NVIDIA's cache):
  DXVK 2 and VKD3D-Proton keep compiled shaders there, and the 1 GB default
  makes games recompile them (stutter) once several are installed.
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

`seg game check` is the quick one: Vulkan, the GPU driver and its 32-bit
version, NTSYNC, `vm.max_map_count`, the open files limit, the kernels, the CPU
governor, whether the boost helper may run, hugepages, split lock, compaction
and mitigations, each with its fix (see [above](#reading-seg-game-check)).
`segrun doctor` reports Wine, winetricks, Vulkan, NTSYNC, GameMode, MangoHud,
gamescope, split lock mitigation, zram, the open file limit,
`vm.max_map_count` and binfmt support.

## Limits

Games whose anti-cheat requires a Windows kernel driver do not run unless
the developer enables Proton support. Check [ProtonDB](https://www.protondb.com/)
for per-game reports.
