# Lean audit

What the images weigh, what is bloat and what was done about it. The audit
(this file's first commit) comes before any removal, so each change can be
checked against it.

* Init system: **OpenRC** is the default and the only one the Lite edition
  builds; the full edition also builds runit and dinit. Every new service
  file ships for all three, nothing assumes systemd.
* Repositories: Artix (`system`, `world`, `galaxy`, `lib32`) and
  Chaotic-AUR, which exists only for `brave-bin`. The Arch repositories are
  not enabled.
* Sizes below are *uncompressed installed sizes* from the ISO workflow's
  "What takes the space" report (`build-in-container.sh`), unless a row says
  otherwise.

## Before

Measured by CI run 48 of 19 October 2026 (commit `4ca2f45`, OpenRC,
linux-zen), the last ISO build before this work.

| Metric | Lite | Full |
|---|---|---|
| ISO | 774,014,976 B (738 MiB) | 3,030,378,496 B (2.82 GiB) |
| `LiveOS/rootfs.img` | 637.2 MiB | 2,681.8 MiB |
| initramfs | 49.7 MiB | 156.7 MiB |
| Installed system, uncompressed | 1,790.1 MiB | 7,139.8 MiB |
| Package list entries (OpenRC) | 48 | 164 (159 unique: 5 listed twice) |
| Services started in the live session | 1 | 8 |
| Services enabled by Setup on an installed system | 14 | 14 |
| Idle RAM | not measured | 792 MB used, live session, 4 GB VM, software GL (docs/desktop.md, 15 Oct) |
| Boot time in QEMU | not measured | not measured |
| Rust code | 24,849 lines, 7 crates | |

Boot time and idle RAM could not be measured when the audit was written: the
sandbox it ran in has no route to the Artix mirrors, no QEMU and no loop
devices. The "After" numbers come from a GitHub Actions build instead.

## After

Measured by the temporary "ISO check" workflow (since removed) on 20 October
2026, commit `57a1b1f` of this branch: both editions built with the real
`build-in-container.sh` (OpenRC, linux-zen) and the images opened. The
repositories are rolling, so every package is a day newer than in the
"Before" build; that is part of the differences, mostly the small ones.

| Metric | Lite before | Lite after | Full before | Full after |
|---|---|---|---|---|
| ISO | 774,014,976 B | 774,531,072 B (+0.07 %) | 3,030,378,496 B | **2,887,837,696 B (-4.7 %, -136 MiB)** |
| `LiveOS/rootfs.img` | 637.2 MiB | 637.6 MiB | 2,681.8 MiB | **2,545.8 MiB (-5.1 %)** |
| initramfs | 49.7 MiB | 49.7 MiB | 156.7 MiB | 156.7 MiB |
| Installed system, uncompressed | 1,790.1 MiB | 1,792.0 MiB | 7,139.8 MiB | **about 6,665 MiB (-6.6 %, -475 MiB)** |
| Package list entries (OpenRC) | 48 | 49 (+`segctl`) | 164 (159 unique) | **152 (152 unique)** |
| Packages in the built image | not recorded | 351 | not recorded | 1,083 |
| Services started in the live session | 1 | 1 | 8 | 7 (no `cupsd`) |
| Services Setup enables on an installed system | 14 | 15 (-`cupsd`, +`seg-zram`, +`seg-tune`) | | |
| Kernel header folders on the image | 0 | 0 | 1 (325 MiB) | 0 |
| Rust code | 24,849 lines | 29,610 (24,907 in `crates/`, 4,703 new in `tools/seg/`) | | |
| `seg` binary | | 567,536 B (stripped, `opt-level = "z"`, LTO, `panic = "abort"`) | | |
| Idle RAM | not measured | 430.7 MiB of 2.9 GiB in use, 2 min after boot, QEMU/KVM, logged in on a text console | 792 MB (15 Oct, other setup) | not measured |
| Boot time | not measured | not measured (see below) | not measured | not measured |

The Lite edition never had kernel headers (it has its own `common.yaml`), so
removing them only shrinks the full edition; Lite grew by the `segctl`
package (a 0.6 MB binary and a few service files) and the day's package
updates. The full edition's saving is the headers (325 MiB), `spectacle` with
`opencv` (113 MiB), the X server stack and `inxi`.

What the image check confirmed, per edition: `seg`, the zram and tune
services (OpenRC links, runit and dinit definitions), the weekly `fstrim`
job, the makepkg drop-in, the I/O scheduler rule and, in the full edition,
`rg`, `fd`, `eza`, `zoxide`; `Xorg`, `spectacle` and `inxi` are gone.

What booting Lite in QEMU (KVM, text console, 150 s of waiting) showed:
`linux-zen` 7.2.11 comes up on OpenRC, the `modules` service loads `ntsync`,
`tcp_bbr`, `sch_fq` and `zram`, `seg-tune` and `seg-zram` start, `seg zram
status` reports an active 1.4 GiB zstd device, and `seg info`, `seg tune`,
`seg svc list --enabled` and `seg doctor` work on the real system. It also
found a bug of the new code: `seg doctor` called `killprocs`, `mount-ro` and
`savecache` (OpenRC `shutdown` runlevel) "enabled but stopped". Fixed in
`6e323a4`; the fix is covered by a unit test and was **not** booted again.
The same boot showed `vm.swappiness` 100 under zram; Setup's 180 is kept
since `cfd6ba8`, also not booted again.

Boot time is not reported: the harness looks at the screen every 31 s, and
QEMU on a shared CI runner is not a stable clock. The full edition was built
and opened, not booted.

## Bloat table

| Item | Size / cost | Why it is bloat | Action |
|---|---|---|---|
| `linux-zen-headers` (`linux-headers` in the list) | 325.1 MiB, on the full edition (`common.yaml`; Lite has its own list and never had it) | Only needed to build `nvidia-open-dkms` on GTX 16 / RTX GPUs. Setup already downloads that driver in a step that needs a network. | **remove** from the image; Setup adds the running kernel's headers to the NVIDIA step |
| `spectacle` (and `opencv`, which only it needs) | `opencv` 112.8 MiB, plus the libraries only opencv uses | Second screenshot tool: the desktop has its own (Print, Super+Shift+S: grim + slurp). Spectacle would only serve the Plasma fallback session. | **remove** |
| `xorg-server`, `xf86-input-*`, `xf86-video-*` | tens of MiB | Legacy X server and its drivers. The sessions are Wayland (labwc, Plasma Wayland); applications use XWayland. The Lite image already ships only `xorg-xwayland`. | **replace** with `xorg-xwayland` only, as Lite does |
| `inxi` | ~1 MiB (Perl stays: OpenRC needs it) | Third system-information tool beside fastfetch and KInfoCenter | **remove** |
| `cupsd` started at boot | an idle daemon on every machine; no socket activation without systemd | Few gaming PCs print | **disable by default** (live session and Setup); packages stay, `seg svc enable cupsd` turns it on |
| 5 packages listed twice (`xorg-server`, `xorg-xwayland`, `xdg-user-dirs`, `xdg-utils`, `ttf-liberation`) | none | Noise | **remove** the repeats |
| `kitty` | 65.6 MiB, needs Python (73.9 MiB, shared). 185 MB + 31 MB of RAM idle in the 15 Oct VM measurement, the largest process | Lite uses `foot`, which draws on the CPU and is tiny | **keep, maintainers decide**: the README presents kitty as the terminal |
| Plasma fallback session (`plasma-workspace` 57.8, `plasma-desktop` 42.0, `breeze` 38.8 MiB, KWin, PowerDevil, KScreen, ...) | roughly 250 MiB | Second desktop: second network applet (`plasma-nm` / `network-manager-applet`), volume (`plasma-pa` / `pavucontrol`), polkit agent, portal backends | **keep, maintainers decide**: the README advertises it as the fallback |
| `discover` -> `signon-ui` -> `qt6-webengine` | 282.9 MiB | A browser engine for the Store window | **keep, maintainers decide** (the Store icon is part of the Windows-like pitch) |
| `vlc` -> Qt 5 (`qt5-base` 64.0 MiB, declarative, svg, x11extras) | about 100 MiB | A second Qt generation for one application | **keep, maintainers decide** (a Qt 6 player would remove it) |
| `mangohud` -> `python-numpy` 49.6 MiB, matplotlib | about 100 MiB | Plotting libraries for a log tool | **keep**: the overlay is a headline gaming feature |
| `vim-runtime` 38.8 MiB | 38.8 MiB | Two command-line editors (`nano`, `vi`/vim) | **keep**: `crontab -e` and `visudo` fall back to `vi` |
| `samba` | 64.3 MiB | Needed by `kdenetwork-filesharing` | **keep**: sharing folders with Windows PCs |
| `wine` 584.9, `brave-bin` 471.6, `lutris` -> `webkit2gtk-4.1` 138.3 | | The product | **keep** |
| `linux-firmware-intel` 138.0, `-nvidia` 103.5, `sof-firmware` 55.1 | | The full edition installs offline on any common hardware | **keep** |
| `llvm-libs` 166.6, `lib32-llvm-libs` 187.8 | | Mesa and Steam need them | **keep** |
| `perl` 55.0 | | `openrc` depends on it | **keep** (protected) |
| `syslog-ng` | 28 MB RSS idle (15 Oct measurement) | Heavier than needed, but logging is wanted | **keep**; `metalog` is a candidate, needs a boot test |
| `bluetoothd` | ~2 MB idle | Controllers and headsets | **keep** |
| `ntpd`, `cronie`, `acpid`, `power-profiles-daemon` | small | Time, scheduled jobs (weekly fstrim uses cron), power events and profiles | **keep** |
| `etc/sddm.conf.d` file and SDDM migration code | 2 lines | SDDM is gone from the image; `seglinux-upgrade` still migrates older systems | **keep** until those systems are gone |
| `s6` entries in `common.yaml` | 12 lines | The build scripts reject s6 | **keep**: the same layout as Artix's own profile |

Not bloat, checked: one network stack (NetworkManager with `wpa_supplicant`),
one audio stack (PipeWire), one browser, one file manager, one media player,
one terminal per edition. No `-git` packages, and the only package of
AUR origin (`brave-bin`, with `chaotic-keyring` and `chaotic-mirrorlist`) is
a deliberate product choice. No systemd units or `systemctl` calls anywhere
in the repository, and no package with the wrong init suffix.

Protected, never touched: `base`, the kernel, `openrc` and the `-openrc` /
`-runit` / `-dinit` service packages, `elogind`, `artix-keyring` (through
`base`), `pacman`, `sudo`, NetworkManager, the firmware packages.

## Rust command-line tools

Only packages from the Artix repositories; none from the AUR. Sizes are not
given because they could not be measured here (no repository access); the
build report ("What takes the space") of the next ISO shows them.

| Tool | Added | Replaces | Why |
|---|---|---|---|
| `ripgrep` (`rg`) | full edition | `grep -r` for searching folders | far faster on a tree, skips `.gitignore`d files and binaries |
| `fd` | full edition | `find -name` | sane defaults, readable syntax |
| `eza` | full edition | `ls -l` (as the new names `ll`, `la`; `ls` itself is untouched) | colors, git status, folders first |
| `zoxide` | full edition | `cd` by typing paths (`z folder`) | learns the folders you use |
| `bat`, `dust`, `bottom`, `procs`, `sd` | no | nothing installed | they would only add to the image |
| `helix` | no | `nano` | it needs a runtime of grammars; nano is already there and small |
| `alacritty` | no | `kitty`/`foot` | the terminal is the maintainers' call (see below) |
| `zellij` | no | no multiplexer is wanted | |
| `sudo-rs` | no | `sudo` | `sudo` is protected (the live user, Setup and `artools` use it) and its availability in the Artix repositories could not be checked |
| `greetd` + `tuigreet` | no | | SegLinux already signs in through greetd with its own Rust greeter |

They are installed in the full edition only (the Lite image stays as it
was); Lite's Setup downloads them with the rest. `ll`, `la` and `z` come from
`/etc/bash/bashrc.d/seglinux-rust-tools.bashrc`.

## Bugs found

File and line are those of the commit before this work (`4ca2f45`).

| file:line | problem | fix |
|---|---|---|
| `data/system/udev-ioschedulers.rules:4-6` | the globs `nvme[0-9]*n[0-9]*` and `sd[a-z]*` also match partitions (`nvme0n1p1`, `sda1`), which have no `queue/scheduler`: udev logged an error for each. Virtual disks (`vda`) got no rule. | only whole disks (`DEVTYPE=="disk"`); `vd*`/`xvd*` get `none`; checked with `udevadm verify` |
| `iso/iso-profiles/seglinux/profile.yaml:79-82,123` | `xorg-server`, `xorg-xwayland`, `xdg-user-dirs`, `xdg-utils` and `ttf-liberation` also in `common.yaml:55,77-79,82` | listed once; CI now fails on a repeat |
| `crates/seglinux-setup/src/plan.rs:824` | the NVIDIA DKMS driver could only build because 325 MiB of kernel headers were on every image | Setup installs the running kernel's headers in that step |
| `iso/scripts/build-in-container.sh:157-160` | the artools patch that reads the kernel version without headers applied to non-stock kernels only; the stock kernel relied on `linux-headers` | applies to every kernel |
| `.github/workflows/ci.yml:72-76` | `data/system/zram-setup` was not shellchecked; the udev rules were not checked at all | both are now; new job for `seg` |
| `crates/seglinux-setup/src/plan.rs:1273` | the swap partition test asserted that the plan never contains the word `zram` | asserts there is no `/dev/zram0` fstab line, which is what it means |

Checked and fine: `shellcheck` is clean on every shell script of the
repository (also with the style level on); every file the PKGBUILD installs
exists; the executable bits are right; the init-suffixed packages are in the
list of their init; there is no systemd unit anywhere; the only hard-coded
account is the live session's `seglinux` user (in `sudoers.d`, live only);
pacman.conf lists the Artix repositories first and Chaotic-AUR last, with
`SigLevel = Required DatabaseOptional`, `Color` and `ParallelDownloads = 5`.

Found, not changed:

* `PKGBUILD:14`: `license=('LicenseRef-SegLinux-unspecified')`: the project has
  no license yet; that is the owner's decision.
* `swap = "none"` in an answer file still gets zram: the module is loaded and
  the udev rule starts it whatever the answer says.
* The two greetd `config.toml` and `pacman.conf` copies in the root overlays
  are identical to each other (and the first to `data/greeter/greetd.toml`).
* `seglinux-upgrade` updates only the `seglinux` package, so a system
  installed before this work does not get `segctl` that way yet.

## Decisions for the maintainers

These are product choices, so they were measured and left alone:

1. **kitty -> foot** for the full edition (Lite already uses foot): about
   65 MiB on disk and, by the 15 October measurement, about 200 MB less RAM
   in a VM with software OpenGL.
2. **Drop the Plasma fallback session**: roughly 250 MiB and a second set of
   desktop services.
3. **Drop Discover**, or the Store's account integration: 283 MiB of
   `qt6-webengine`.
4. **A Qt 6 media player instead of VLC**: removes Qt 5, about 100 MiB.
5. **Brave from Chaotic-AUR**: the only AUR-origin package of the image.

## Verification limits

Run for this work: `cargo fmt --check`, `cargo clippy -- -D warnings` and
`cargo test` for the workspace and for `tools/seg`; `shellcheck` on every
shell script of the repository; the CI jobs of `.github/workflows/ci.yml`
(package lists resolve with `pacman -Sp` in an `artixlinux` container, the
udev rules pass `udevadm verify`, no package is listed twice); `seg` itself
inside an Artix container (`info`, `doctor`, `clean`, `tune`, `zram status`,
`svc` against the real OpenRC and dinit layouts); the fstrim script's stamp
logic; both ISOs built in CI and the Lite one booted in QEMU, as above.

Not done, and not claimed:

* No ISO was built or booted in the sandbox the work was done in (no
  mirrors, no QEMU, no loop devices): every ISO fact above comes from CI.
* Only the OpenRC images were built. The runit, dinit and s6 definitions
  were checked against the files the Artix packages install and by their
  unit tests, not by booting those inits.
* The full edition was not booted, so its idle RAM and boot time are not
  known, and `cfd6ba8` and `6e323a4` (after the image check) were not
  booted.
* The installed-system path (Setup writing the files, `grub-install`) was not
  run; its plan is covered by the unit tests.
* No real hardware: nothing about GPUs, cpufreq or NVMe was exercised.
