# Building and testing the Wincraft ISO

Wincraft is built with Artix Linux's own ISO tooling: `buildiso` from
[artools](https://gitea.artixlinux.org/artix/artools) reads
`iso/iso-profiles/wincraft/profile.yaml`. The Rust tools are packaged with
`packaging/wincraft/PKGBUILD` and served to `buildiso` from a local pacman
repository during the build.

## Quick start (any Linux host with docker or podman)

```sh
git clone https://github.com/SegMind25/Wincraft
cd Wincraft
./iso/build.sh                  # OpenRC + linux-zen (default)
./iso/build.sh -i runit         # runit (or -i dinit)
./iso/build.sh -k linux         # stock Arch kernel instead of linux-zen
./iso/build.sh --lite           # the small Lite edition (see below)
```

The ISO and its checksum are written to `iso/out/`, for example
`iso/out/wincraft-openrc-20261012-x86_64.iso`.

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
packages; they are cached in the `wincraft-pacman-cache` volume.

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
   git clone https://github.com/SegMind25/Wincraft.git
   cd Wincraft
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
   sha256sum -c iso/out/wincraft-*.iso.sha256
   ```

   Try it in QEMU (see below) or write it to a USB stick.

Windows PCs: use a Linux virtual machine, or GitHub Actions (next section);
Docker Desktop on WSL2 does not reliably provide the loop devices the build
needs.

## The Lite edition

The full edition is a large image: the Plasma fallback, Steam, Wine,
Firefox and every driver and firmware file are on it. The **Lite** edition
(`iso/iso-profiles/wincraft-lite`) is the smallest image that still boots
the Wincraft desktop and gets online; the first build was 868 MB, and each
release's notes give its size:

| | Full | Lite |
|---|---|---|
| Live desktop | Wincraft (Rust) on labwc, Plasma fallback, SDDM | Wincraft (Rust) on labwc, signed in on tty1 |
| Terminal | Konsole | foot |
| Firmware | all (`linux-firmware`) | graphics and Wi-Fi/Ethernet of PCs: AMD, Intel, Radeon, Realtek, Atheros, MediaTek, Broadcom |
| Wine, Steam, Firefox, Lutris, drivers... | on the image | **downloaded by Setup** |
| Init systems | OpenRC, runit, dinit | OpenRC |
| Image compression | zstd (artools default) | xz, 1 MiB blocks, x86 filter |
| Manuals, documentation, translations; Bluetooth, camera and server firmware; TV tuner and data-centre drivers | included | left out of the image ([list](../iso/iso-profiles/wincraft-lite/pacman-noextract.conf)), restored by Setup |

Setup on the Lite image installs the same system as the full edition: after
copying the small image it runs `pacman -Syu` for everything else (the list
is generated at build time by `iso/scripts/online-packages.sh` into
`/usr/share/wincraft/online-packages`), then reinstalls the packages whose
files the image left out (`/usr/lib/wincraft/restore-trimmed-files`). So
**installing Lite needs an internet connection** (wired networks connect by themselves; for Wi-Fi use
*Network* in the panel, or `nmtui`). Setup's summary warns when there is no
network. Without one, the installed system boots to a text login: sign in
and run `wincraft-session`, then install the rest later with
`sudo pacman -Syu --needed $(grep -v '^#' /usr/share/wincraft/online-packages)`
followed by `sudo /usr/lib/wincraft/restore-trimmed-files`.

Each build ends with a report of the largest files, packages and firmware
folders, which is where to look when trimming the image further.

The size of each build is in its release notes.

## Building on GitHub instead

Actions → **ISO** → *Run workflow*, pushing a `v*` tag (both editions), or
pushing a commit whose message contains `[build iso]` (full edition),
`[build lite iso]` (Lite edition) or both builds the ISO on GitHub's servers
(OpenRC and linux-zen unless chosen otherwise in *Run workflow*) and
publishes it under **Releases** as a pre-release. ISOs over 2 GB are split
into `.part00`, `.part01`, ... files; join them with
`cat wincraft-*.iso.part* > wincraft.iso` and check the `.sha256` file. A
private repository's Actions minutes count towards your plan.

## What the build does

`iso/build.sh` builds `iso/Containerfile` (based on
`docker.io/artixlinux/artixlinux:base-devel` plus artools, Rust and
go-yq) and runs `iso/scripts/build-in-container.sh` in it, which:

1. Packs the working tree and builds the `wincraft` package with
   `makepkg` (as an unprivileged `builder` user; `check()` runs the Rust
   test suite).
2. Creates the local repository `/var/lib/wincraft-repo` with `repo-add`.
3. Installs `iso/pacman.conf.d/iso-x86_64.conf` as artools'
   `pacman.conf.d/iso-x86_64.conf`: `[wincraft]` first, then Artix's
   `[system]`, `[world]`, `[galaxy]` and `[lib32]`.
4. Stages `iso/iso-profiles` as the artools workspace (fixing modes such as
   0440 for sudoers files, which git cannot store).
5. Runs `iso/scripts/check-packages.sh`: every package name must exist and
   each layer must resolve, so a typo fails in seconds instead of after
   half an hour.
6. Runs `buildiso -p wincraft -i <init>` (`-p wincraft-lite` for Lite,
   after copying its own `common.yaml` over the shared one and writing the
   download list), which builds:
   * **rootfs** (`LiveOS/rootfs.img`): `common.yaml`'s base packages, the
     profile's `rootfs.packages`, and `root-overlay/`. This is the system
     `wincraft-setup` installs.
   * **livefs** (`LiveOS/livefs.img`): live-only packages (the live boot
     menu, `artix-live-<init>`, `squashfs-tools`) and `live-overlay/`
     (the "Install Wincraft" icon, the installer autostart, passwordless
     sudo for the live user `wincraft`).
   * the live initramfs (`iso-initcpio`), the GRUB menus for BIOS and UEFI,
     and the hybrid ISO.
7. Renames the image to `wincraft-*.iso` and writes a `.sha256` file.

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

The live session uses **Wincraft (Rust)**. `live-overlay/usr/share/artools/desktop.map`
lets `artix-live` detect the new session instead of resetting it to Plasma.
The installer also uses `wincraft.desktop` for optional automatic login.
Plasma remains selectable in SDDM as a fallback while the Rust desktop matures.

## Changing what is on the ISO

* **Packages**: edit `rootfs.packages` (installed system) or
  `livefs.packages` (live session only) in
  `iso/iso-profiles/wincraft/profile.yaml`; shared base packages are in
  `iso/iso-profiles/common/common.yaml`. Init-specific service packages go
  under `packages-init: openrc:` / `dinit:`.
* **Files**: put them in `root-overlay/` (installed system) or
  `live-overlay/` (live session). Keep each overlay non-empty: artools
  copies `overlay/*`.
* **Defaults applied by the package** (desktop layout, file associations,
  binfmt, sysctl, ...) live in `data/` and are installed by the PKGBUILD.
* **An answer file baked into the ISO**: add
  `live-overlay/etc/wincraft/autounattend.toml`.
* **Check package names** without building, on Artix or in the container:
  `iso/scripts/check-packages.sh -i openrc` (add `-p wincraft-lite` for
  the Lite edition).

## Testing in QEMU

```sh
qemu-img create -f qcow2 wincraft-disk.qcow2 40G

# UEFI (Arch/Artix: edk2-ovmf; Debian/Ubuntu: ovmf, files in /usr/share/OVMF/)
cp /usr/share/edk2/x64/OVMF_VARS.4m.fd ovmf-vars.fd
qemu-system-x86_64 -enable-kvm -cpu host -smp 4 -m 4G \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd \
  -drive if=pflash,format=raw,file=ovmf-vars.fd \
  -drive file=wincraft-disk.qcow2,if=virtio \
  -cdrom iso/out/wincraft-openrc-*.iso -boot menu=on \
  -device virtio-vga-gl -display gtk,gl=on

# Legacy BIOS: the same without the two pflash drives.
```

In the live session, double-click **Install Wincraft**. To test an
unattended installation, add `-drive file=answer.img,format=raw,if=virtio`
with a small FAT image holding `autounattend.toml` at its root, or boot
with `wincraft.unattend=/path` on the kernel command line.

`wincraft-setup plan --unattend autounattend.toml` (as root, in the live
session) prints every command and file without changing anything.

## Writing a USB stick

```sh
sudo dd if=iso/out/wincraft-openrc-*.iso of=/dev/sdX bs=4M status=progress oflag=sync
```

The image is hybrid: the same file boots from DVD and USB, on UEFI and BIOS.
Ventoy also works.

## Secure Boot

Artix's boot chain (GRUB, kernel) is not signed with Microsoft's key, so
disable Secure Boot in the firmware to boot the ISO. After installing you
can enroll your own keys and sign the kernel and GRUB with `sbctl`.

## Continuous integration

`.github/workflows/ci.yml` runs on every push: Rust formatting, clippy and
tests, shellcheck, YAML and answer-file validation, and
`check-packages.sh` for OpenRC and dinit inside an Artix container.
`.github/workflows/iso.yml` builds the ISO on demand (Actions → ISO → Run
workflow) and for `v*` tags, and uploads it as a workflow artifact.

## Troubleshooting

* `check-packages.sh` reports a package as missing: Artix renamed or dropped
  it; search with `pacman -Ss` in the container and update the profile.
* `overlayfs: upper fs does not support ...`: `/var/lib/artools` must be on
  a regular file system (ext4, xfs, btrfs), not on the container's overlay
  or on ZFS. `build.sh` uses a named volume for this.
* `losetup: cannot find an unused loop device`: load the module on the host
  (`sudo modprobe loop`).
* Logs of a failed build are in the container output; buildiso's work
  directory is in the `wincraft-artools` volume.
