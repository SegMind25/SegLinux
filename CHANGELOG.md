# Changelog

Notable changes to SegLinux. The project has no version numbers yet, so this
lists what changed since the last ISO build of 19 October 2026 (`4ca2f45`).
Measurements and the reasons behind each removal are in
[docs/lean-audit.md](docs/lean-audit.md).

## Unreleased

### Changed

- **`seglinux-upgrade` no longer uses Chaotic-AUR**: its package list (several
  MB on every update) only served Brave. The update removes the repository and
  its keyring and updates Brave from its AUR recipe instead, only when a new
  version is out ([docs/updating.md](docs/updating.md)).
- **A dead repository no longer stops updates**: `seglinux-upgrade` turns off
  (comments out) a repository in `/etc/pacman.conf` whose server is gone, such
  as `net-merge`, instead of failing with "failed to synchronize all databases".

### Removed

- **The Plasma session and Discover** (about 500 MiB, and a second set of
  desktop services): the SegLinux desktop is the only one. Blueman and the
  printer settings (system-config-printer) take over Bluetooth and printers;
  Flathub stays available through `flatpak`. `seglinux-upgrade` offers once
  to remove them from installed systems.

### Added

- **SegLinux's own compositor (preview)**: `seglinux-compositor`, written in
  Rust on Smithay, as an extra choice on the sign-in screen beside the labwc
  session, which stays the default. Windows, the dock and its menus, the
  same keys as on labwc, a settings file for keys, start-up and keyboard,
  GPU buffers for games, several screens, the dock's list of open
  windows, title bars with minimize, maximize and close, panels that take
  the keyboard (Start menu search), X11 applications through XWayland, copy and paste
  between X11 and Wayland, screenshots, the lock screen, workspaces, and
  the Settings app's keyboard, mouse and touchpad choices ([docs/compositor.md](docs/compositor.md)).

- **A lighter desktop**: the terminal is foot instead of kitty (about 65 MiB
  less on disk, no Python, and about 200 MB less memory with a terminal open
  in the October measurement), and the media player is mpv instead of VLC,
  which removes the whole Qt 5 library set (about 100 MiB).
  Updated systems are offered foot once; kitty and VLC stay until removed
  (`sudo pacman -Rns kitty vlc`).

- **Laptops with two GPUs play on the dedicated one**: `seg game run` sends
  the game to the NVIDIA (PRIME offload) or AMD/Intel card (`DRI_PRIME`)
  when the screen starts on the integrated GPU; before, games ran on the
  integrated GPU ([docs/gaming.md](docs/gaming.md#what-seg-game-run-does)).
- **NVIDIA**: the driver uses the Page Attribute Table
  (`NVreg_UsePageAttributeTable=1`); `seg game setup` tells GTX 750 to
  GTX 1080 Ti owners that NVK runs them slowly and names NVIDIA's 580 driver
  in the AUR.

- **`seg game`**, a small gaming layer in the `seg` tool, the same on OpenRC,
  runit, dinit and s6 ([docs/gaming.md](docs/gaming.md#seg-game)):
  - `seg game run -- CMD` (Steam: `seg game run -- %command%`) raises the CPU
    governor and energy preference and an AMD GPU's power level while a game
    runs, gives the game a little more CPU and the best I/O priority, holds
    off the screen saver, can pin it to the V-Cache chiplet of a two-chiplet
    X3D CPU, and restores everything when the game ends, on Ctrl+C, SIGTERM,
    SIGHUP or a crash of `seg`.
  - `seg game check`: PASS/WARN/FAIL/SKIP for Vulkan, the GPU driver, 32-bit
    drivers, NTSYNC, `vm.max_map_count`, open files, kernel, fallback kernel,
    CPU governor and driver, boost permission, hugepages, split lock,
    compaction and mitigations, each with the exact fix.
  - `seg game setup [--yes]`: the driver packages for the GPUs found in the
    computer, and only those.
  - `seg game profiles`, per-game profiles in `~/.config/seg/games.toml`
    (governor, CPU pinning, MangoHud, gamescope arguments, nice).
  - `seg game apply` and the **`seg-game` service** (OpenRC, runit, dinit,
    s6): the gaming sysctls and `/dev/ntsync` permissions at boot.
  - `seg game audio on|off`: low-latency PipeWire, off by default.
  - `seg game unsafe-boost on|off`: `mitigations=off`, the only way to get
    it, behind a warning and `--i-understand`.
  - `seg tune gaming`: low swappiness, warm caches, short write-back, CPU
    left dynamic.
  - A sudoers rule for the `gamemode` group (two commands, nothing else), a
    MangoHud preset and PipeWire drop-ins under `/usr/share/seg` (all off
    until asked for).
- Setup installs the **other kernel** (`linux` beside `linux-zen`, or the
  reverse) as a boot menu entry to fall back on when online
  (`[performance] fallback_kernel`); the installed kernel is never removed.
- Transparent hugepages go from `always` to `madvise` at boot (`seg-game`
  service), so the live image and upgraded systems get it too, not only
  fresh installs.
- Kernel command line: `transparent_hugepage=madvise` everywhere and
  `amd_pstate=active` on AMD processors (an answer file's parameter of the
  same name replaces the default).
- `vm.compaction_proactiveness = 0` with the other gaming sysctls.
- NVIDIA: `nvidia_drm modeset=1 fbdev=1` (Wayland compositors, VT switch).
- `seg` (`tools/seg`, package `segctl`): `info`, `svc`, `clean`, `tune`,
  `zram`, `doctor` for OpenRC, runit, dinit and s6, with the `seg-zram` and
  `seg-tune` services and a weekly `fstrim` cron job.
- `ripgrep`, `fd`, `eza` and `zoxide` in the full edition.
- `docs/lean-audit.md`: what the images weigh before and after, the bloat
  table, the bugs found, what was not verified.
- CI: a job for `seg` (fmt, clippy, tests, release size, dependency and
  `unsafe` guards, `visudo -cf` of the sudoers rule), `udevadm verify` for
  the udev rules, a check for repeated packages, more `shellcheck` targets.

### Changed

- `wine` -> `wine-staging` (the Staging patches: esync/fsync/NTSYNC, game
  fixes) in the full edition. `lib32-mangohud` was tried and is not in the
  Artix repositories, so 32-bit games get no MangoHud overlay.
- Open-file hard limit 524288 -> 1048576 (the kernel's maximum); the
  `gamemode` group may use `nice -10`.
- Shader caches: Mesa 4 GB -> 10 GB; NVIDIA's shader cache set to 10 GB and
  no longer deleted when full (`/etc/profile.d`, so on every init).
- `sudo seglinux-upgrade` (the from-source path) now also builds and
  installs `segctl`, so an older install gets `seg`, `seg game`, the
  services and the sudoers rule; CI checks this on an Artix container.
- `makepkg` builds with every thread; the I/O scheduler rule skips partitions
  and covers virtual disks.

### Removed

- From the full image: `linux-headers` (325 MiB; Setup downloads the running
  kernel's headers together with the NVIDIA DKMS driver), `spectacle` and
  `opencv`, the X server and its drivers (`xorg-xwayland` stays), `inxi`,
  and five packages that were listed twice. The full ISO is 4.7 % smaller and
  475 MiB less is installed.
- `cupsd` is no longer started by default (the packages stay; `seg svc enable
  cupsd`).

### Fixed

- The NVIDIA DKMS driver could only build because the headers were on every
  image: Setup now installs them in that step.
- The artools patch that reads the kernel version without headers applies to
  every kernel, not only the non-stock ones.
- The I/O scheduler udev rule matched partitions (errors in the log) and
  missed virtual disks.
- `data/system/zram-setup` and the udev rules were not checked by CI.
- `seg doctor` called OpenRC services of the `shutdown` runlevel (`killprocs`,
  `mount-ro`, `savecache`) failed; found by booting the Lite image.
