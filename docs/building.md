# Building and testing the SegLinux ISO

SegLinux is built with Artix Linux's own ISO tooling: `buildiso` from
[artools](https://gitea.artixlinux.org/artix/artools) reads
`iso/iso-profiles/seglinux/profile.yaml`. The Rust tools are packaged with
`packaging/seglinux/PKGBUILD` and served to `buildiso` from a local pacman
repository during the build.

## Quick start (any Linux host with docker or podman)

```sh
git clone https://github.com/SegMind25/SegLinux
cd SegLinux
./iso/build.sh                  # OpenRC + linux-zen (default)
./iso/build.sh -i runit         # runit (or -i dinit)
./iso/build.sh -k linux         # stock Arch kernel instead of linux-zen
./iso/build.sh --lite           # the small Lite edition (see below)
```

The ISO and its checksum are written to `iso/out/`, for example
`iso/out/seglinux-openrc-20261012-x86_64.iso`.

`common.yaml` names the stock `linux` kernel, which is what artools expects.
For `linux-zen` (or `linux-lts`) the build swaps the kernel in a staged copy
of the profile and runs `buildiso` with a patched copy of the artools
library (`LIBDIR`): artools reads the live kernel's version from
`/usr/src/linux/version`, which only the stock kernel's headers provide, and
the copy reads it from `/usr/lib/modules` instead. The installed artools
files are not modified. If artools changes that code, the build stops with a
message suggesting `-k linux`.

Needs: about 25 GB of free disk space, a network connection, and root
rights for the container (`--privileged`, because `buildiso` mounts file
systems and uses loop devices). The first build downloads roughly 3 GB of
packages; they are cached in the `seglinux-pacman-cache` volume.

## Build on your own PC, step by step

Any 64-bit Linux works (Artix, Arch, Ubuntu, Debian, Mint, Fedora, ...).
You need about 30 GB of free disk space, 8 GB of RAM, an internet
connection (about 4 GB of downloads) and 30-90 minutes.

1. Install git and a container engine:

   ```sh
   # Artix
   sudo pacman -S --needed git docker docker-openrc && sudo rc-service docker start
   # Arch, Manjaro, EndeavourOS
   sudo pacman -S --needed git docker && sudo systemctl start docker
   # Ubuntu, Debian, Linux Mint
   sudo apt install git docker.io && sudo systemctl start docker
   # Fedora
   sudo dnf install git podman
   ```

2. Get the sources. The repository is private, so sign in first, for
   example with the GitHub CLI (`gh auth login`) or a personal access token:

   ```sh
   git clone https://github.com/SegMind25/SegLinux.git
   cd SegLinux
   ```

3. Build:

   ```sh
   sudo modprobe loop
   sudo ./iso/build.sh                 # full edition: OpenRC + linux-zen
   sudo ./iso/build.sh --lite          # Lite edition (smaller download)
   # other choices: sudo ./iso/build.sh -i runit     or     -k linux
   ```

   (`sudo` is not needed when your user is in the `docker` group.)

4. The result is in `iso/out/`:

   ```sh
   ls -lh iso/out/
   sha256sum -c iso/out/seglinux-*.iso.sha256
   ```

   Try it in QEMU (see below) or write it to a USB stick.

Windows PCs: use a Linux virtual machine, or GitHub Actions (next section);
Docker Desktop on WSL2 does not reliably provide the loop devices the build
needs.

## The Lite edition

The full edition is a large image: the Plasma fallback, Steam, Wine,
Firefox and every driver and firmware file are on it. The **Lite** edition
(`iso/iso-profiles/seglinux-lite`) is the smallest image that still boots
the SegLinux desktop and gets online: about 740 MB (the first build was
868 MB before trimming), and each release's notes give its size:

| | Full | Lite |
|---|---|---|
| Live desktop | SegLinux (Rust) on labwc, Plasma fallback, SDDM | SegLinux (Rust) on labwc, signed in on tty1 |
| Terminal | Konsole | foot |
| Firmware | all (`linux-firmware`) | graphics and Wi-Fi/Ethernet of PCs: AMD, Intel, Radeon, Realtek, Atheros, MediaTek, Broadcom |
| Wine, Steam, Firefox, Lutris, drivers... | on the image | **downloaded by Setup** |
| Init systems | OpenRC, runit, dinit | OpenRC |
| Image compression | zstd (artools default) | xz, 1 MiB blocks, x86 filter |
| Manuals, documentation, translations; Bluetooth, camera and server firmware | included | left out of the image ([list](../iso/iso-profiles/seglinux-lite/pacman-noextract.conf)), restored by Setup |

Setup on the Lite image installs the same system as the full edition: after
copying the small image it runs `pacman -Syu` for everything else (the list
is generated at build time by `iso/scripts/online-packages.sh` into
`/usr/share/seglinux/online-packages`), then reinstalls the packages whose
files the image left out (`/usr/lib/seglinux/restore-trimmed-files`). So
**installing Lite needs an internet connection** (wired networks connect by themselves; for Wi-Fi use
*Network* in the panel, or `nmtui`). Setup's summary warns when there is no
network. Without one, the installed system boots to a text login: sign in
and run `seglinux-session`, then install the rest later with
`sudo pacman -Syu --needed $(grep -v '^#' /usr/share/seglinux/online-packages)`
followed by `sudo /usr/lib/seglinux/restore-trimmed-files`.

Each build ends with a report of the largest files, packages and firmware
folders, which is where to look when trimming the image further.

The size of each build is in its release notes.

## Building on GitHub instead

Actions → **ISO** → *Run workflow*, pushing a `v*` tag (both editions), or
pushing a commit to `main` whose message contains `[build iso]` (full
edition), `[build lite iso]` (Lite edition) or both builds the ISO on GitHub's servers
(OpenRC and linux-zen unless chosen otherwise in *Run workflow*) and
publishes it under **Releases** as a pre-release. ISOs over 2 GB are split
into `.part00`, `.part01`, ... files; join them with
`cat seglinux-*.iso.part* > seglinux.iso` and check the `.sha256` file. A
private repository's Actions minutes count towards your plan.

## What the build does

`iso/build.sh` builds `iso/Containerfile` (based on
`docker.io/artixlinux/artixlinux:base-devel` plus artools, Rust and
go-yq, installed by `iso/scripts/container-packages.sh`, which refreshes
the keyring first and moves on to the next mirror when one serves a
package whose signature does not match, as happens mid-sync) and runs
`iso/scripts/build-in-container.sh` in it, which:

1. Packs the working tree and builds the `seglinux` package with
   `makepkg` (as an unprivileged `builder` user; `check()` runs the Rust
   test suite).
2. Creates the local repository `/var/lib/seglinux-repo` with `repo-add`.
3. Installs `iso/pacman.conf.d/iso-x86_64.conf` as artools'
   `pacman.conf.d/iso-x86_64.conf`: `[seglinux]` first, then Artix's
   `[system]`, `[world]`, `[galaxy]` and `[lib32]`.
4. Stages `iso/iso-profiles` as the artools workspace (fixing modes such as
   0440 for sudoers files, which git cannot store).
5. Runs `iso/scripts/check-packages.sh`: every package name must exist and
   each layer must resolve, so a typo fails in seconds instead of after
   half an hour.
6. Runs a copy of `buildiso` that names the image SegLinux (see
   [Branding](#branding)): `buildiso -p seglinux -i <init>` (`-p seglinux-lite` for Lite,
   after copying its own `common.yaml` over the shared one and writing the
   download list), which builds:
   * **rootfs** (`LiveOS/rootfs.img`): `common.yaml`'s base packages, the
     profile's `rootfs.packages`, and `root-overlay/`. This is the system
     `seglinux-setup` installs.
   * **livefs** (`LiveOS/livefs.img`): live-only packages (the live boot
     menu, `artix-live-<init>`, `squashfs-tools`) and `live-overlay/`
     (the "Install SegLinux" icon, the installer autostart, passwordless
     sudo for the live user `seglinux`).
   * the live initramfs (`iso-initcpio`), the GRUB menus for BIOS and UEFI,
     and the hybrid ISO.
7. Renames the image to `seglinux-*.iso` and writes a `.sha256` file.

## Branding

SegLinux uses Artix's packages and ISO tools but shows its own name and
artwork. The `seglinux` package installs the logo (`seglinux` icon),
wallpapers, os-release and the boot menu theme
(`/usr/share/grub/themes/seglinux`, from `data/grub/seglinux`). The build
runs a copy of artools' `buildiso` that:

* labels the image `SEGLINUX_<yyyymm>` (the name a computer shows for the
  DVD or USB stick) and its EFI partition `SEGLINUX`;
* loads `iso/scripts/buildiso-hooks.sh` after artools' libraries, which puts
  the SegLinux theme on the boot menu, renames its entries ("Start SegLinux
  (DVD or ISO image)", "Start SegLinux (USB stick or hard disk)") and sets
  the image's application and publisher fields. The build log lists the
  boot menu entries.

Setup gives installed systems the same theme (`GRUB_THEME`), `SegLinux` as
the UEFI boot entry and "Seg Linux" in the GRUB menu (GRUB adds " Linux"
to `GRUB_DISTRIBUTOR`, which is therefore `Seg`).

What stays Artix's: the package repositories and keyring
(`pacman-key --populate artix`), the live scripts (`artix-live`, the
`/run/artix` mount point), `ID_LIKE="artix arch"` in os-release (so tools
treat SegLinux like Artix and Arch), and the line "OpenRC ... is starting
up Artix Linux" early in the boot log, which is compiled into Artix's
OpenRC package.

The artwork is drawn by `data/artwork/make-brand.py` (see
`data/artwork/README.md`).

## Building natively on Artix

```sh
sudo pacman -Syu --needed artools-base artools-iso base-devel rust go-yq squashfs-tools \
  gtk4 gtk4-layer-shell labwc procps-ng libisoburn mtools dosfstools grub sudo
./iso/build.sh --native -i openrc        # add --lite for the Lite edition
```

The build must run with administrator privileges. Codex cannot enter your
sudo password. If native tooling is missing, install the prerequisites above
in your own terminal and then run the build command. Allow at least 25 GB free;
the full Plasma fallback, Windows compatibility packages and build caches make
this a large image.

The live session uses **SegLinux (Rust)**. `live-overlay/usr/share/artools/desktop.map`
lets `artix-live` detect the new session instead of resetting it to Plasma.
The installer also uses `seglinux.desktop` for optional automatic login.
Plasma remains selectable in SDDM as a fallback while the Rust desktop matures.

## Changing what is on the ISO

* **Packages**: edit `rootfs.packages` (installed system) or
  `livefs.packages` (live session only) in
  `iso/iso-profiles/seglinux/profile.yaml`; shared base packages are in
  `iso/iso-profiles/common/common.yaml`. Init-specific service packages go
  under `packages-init: openrc:` / `dinit:`.
* **Files**: put them in `root-overlay/` (installed system) or
  `live-overlay/` (live session). Keep each overlay non-empty: artools
  copies `overlay/*`.
* **Defaults applied by the package** (desktop layout, file associations,
  binfmt, sysctl, ...) live in `data/` and are installed by the PKGBUILD.
* **An answer file baked into the ISO**: add
  `live-overlay/etc/seglinux/autounattend.toml`.
* **Check package names** without building, on Artix or in the container:
  `iso/scripts/check-packages.sh -i openrc` (add `-p seglinux-lite` for
  the Lite edition).

## Testing in QEMU

```sh
qemu-img create -f qcow2 seglinux-disk.qcow2 40G

# UEFI (Arch/Artix: edk2-ovmf; Debian/Ubuntu: ovmf, files in /usr/share/OVMF/)
cp /usr/share/edk2/x64/OVMF_VARS.4m.fd ovmf-vars.fd
qemu-system-x86_64 -enable-kvm -cpu host -smp 4 -m 4G \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd \
  -drive if=pflash,format=raw,file=ovmf-vars.fd \
  -drive file=seglinux-disk.qcow2,if=virtio \
  -cdrom iso/out/seglinux-openrc-*.iso -boot menu=on \
  -device virtio-vga-gl -display gtk,gl=on

# Legacy BIOS: the same without the two pflash drives.
```

`virtio-vga-gl` with `gl=on` gives the virtual machine 3D acceleration.
VirtualBox: *Display → Graphics Controller: VMSVGA* and *Enable 3D
Acceleration*; VMware: *Accelerate 3D graphics*. Without it there is no GPU
driver to render with: the desktop then falls back to Mesa's software
renderer (slow, and no games), and `~/.cache/seglinux-session.log` tells
why it stopped if it does not start at all.

In the live session, double-click **Install SegLinux**: the graphical Setup
(`seglinux-installer`) asks for the region, disk and account, shows the plan
and then runs `seglinux-setup install --unattend` with the answers, following
its progress. *Install SegLinux (text mode)* in the menu starts the
keyboard-driven text Setup instead. `seglinux-installer --demo` (or
`--demo-page=summary` and the other page names) shows the wizard on any
desktop with an invented computer and installs nothing. To test an
unattended installation, add `-drive file=answer.img,format=raw,if=virtio`
with a small FAT image holding `autounattend.toml` at its root, or boot
with `seglinux.unattend=/path` on the kernel command line.

`seglinux-setup plan --unattend autounattend.toml` (as root, in the live
session) prints every command and file without changing anything.

## Writing a USB stick

```sh
sudo dd if=iso/out/seglinux-openrc-*.iso of=/dev/sdX bs=4M status=progress oflag=sync
```

The image is hybrid: the same file boots from DVD and USB, on UEFI and BIOS.
Ventoy also works.

## Secure Boot

Artix's boot chain (GRUB, kernel) is not signed with Microsoft's key, so
disable Secure Boot in the firmware to boot the ISO. After installing you
can enroll your own keys and sign the kernel and GRUB with `sbctl`.

## Continuous integration

`.github/workflows/ci.yml` runs on every push and pull request: Rust
formatting, clippy and tests, the Settings window's integration test under
Xvfb, shellcheck, YAML, XML and answer-file validation, the website's build,
downloader tests and browser regression test (Playwright), and
`check-packages.sh` for OpenRC, runit and dinit (and the Lite edition's
online packages) inside an Artix container.
`.github/workflows/iso.yml` builds the ISO on demand (Actions → ISO → Run
workflow), for `v*` tags (both editions), and for pushes to `main` whose
commit message contains `[build iso]` (Full) and/or `[build lite iso]`
(Lite). A Full build takes about 20 minutes, Lite about 15. Each image is
published as a GitHub release (a pre-release named
`iso-DATE-EDITION-INIT-KERNEL-runN`), with a `.sha256` file; images over
2 GB are split into `.partNN` pieces.

The website's download page then offers it without any manual step: the
page asks GitHub for the releases in the visitor's browser (the repository
is public), shows the newest image of each edition on the Lite and Full
cards and every image in **All builds**. The workflow also asks Vercel to
rebuild the site when the `VERCEL_DEPLOY_HOOK` secret is set, so the list
built into the page is current too. Previously published images are not
changed by new commits: download a newer build to get them.

## Troubleshooting

* `check-packages.sh` reports a package as missing: Artix renamed or dropped
  it; search with `pacman -Ss` in the container and update the profile.
* `overlayfs: upper fs does not support ...`: `/var/lib/artools` must be on
  a regular file system (ext4, xfs, btrfs), not on the container's overlay
  or on ZFS. `build.sh` uses a named volume for this.
* `losetup: cannot find an unused loop device`: load the module on the host
  (`sudo modprobe loop`).
* Logs of a failed build are in the container output; buildiso's work
  directory is in the `seglinux-artools` volume.
* The live desktop stops at once and prints renderer errors: no GPU driver
  could render (often a virtual machine without 3D acceleration, see
  *Testing in QEMU*). The session retries with software rendering; the Lite
  edition shows the last messages on the console and keeps them in
  `~/.cache/seglinux-session.log`.
