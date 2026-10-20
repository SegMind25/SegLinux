#!/usr/bin/env bash
# Looks inside a built ISO's system image (without booting it) and prints
# what this branch changed: sizes, what is and is not installed, which
# services are enabled. Usage: inspect.sh ISO
set -uo pipefail
iso=$1
sudo apt-get install -y -qq squashfs-tools >/dev/null
sudo mkdir -p /mnt/iso
sudo mount -o loop,ro "$iso" /mnt/iso
echo "== ISO: $(stat -c %s "$iso") bytes"
ls -l /mnt/iso/LiveOS /mnt/iso/boot | grep -E 'img|vmlinuz'
list=$(mktemp)
sudo unsquashfs -ll /mnt/iso/LiveOS/rootfs.img > "$list" 2>/dev/null
echo "== listing: $(wc -l < "$list") entries"
has() { grep -qE "squashfs-root/$1( ->.*)?\$" "$list" && echo "present  $1" || echo "ABSENT   $1"; }
echo "== packages in the image: $(grep -cE 'squashfs-root/var/lib/pacman/local/[^/]+/desc$' "$list")"
echo "== should be there"
for f in usr/bin/seg etc/init.d/seg-zram etc/init.d/seg-tune etc/runlevels/default/seg-zram etc/runlevels/default/seg-tune \
    etc/seg/tune.conf etc/sysctl.d/99-seg.conf etc/cron.d/seg-fstrim usr/lib/seg/fstrim-weekly \
    etc/makepkg.conf.d/10-seglinux.conf usr/lib/seglinux/zram-setup usr/lib/udev/rules.d/60-seglinux-ioschedulers.rules \
    etc/runit/sv/seg-zram/run etc/dinit.d/seg-zram; do has "$f"; done
if [[ $EDITION == full ]]; then
    for f in usr/bin/rg usr/bin/fd usr/bin/eza usr/bin/zoxide etc/bash/bashrc.d/seglinux-rust-tools.bashrc usr/bin/xwayland usr/bin/Xwayland; do has "$f"; done
fi
echo "== gaming files (both editions)"
for f in etc/sudoers.d/seg-game etc/init.d/seg-game etc/runlevels/default/seg-game etc/runit/sv/seg-game/run \
    etc/dinit.d/seg-game etc/s6/sv/seg-game/type etc/s6/sv/seg-game/up usr/share/seg/mangohud-gaming.conf \
    usr/share/seg/pipewire/pipewire.conf.d/99-seg-lowlatency.conf usr/share/seg/pipewire/pipewire-pulse.conf.d/99-seg-lowlatency.conf \
    etc/sysctl.d/80-seglinux-gaming.conf usr/lib/modprobe.d/seglinux-nvidia.conf etc/profile.d/seglinux-gaming.sh \
    etc/security/limits.d/90-seglinux-esync.conf etc/modules-load.d/10-ntsync.conf; do has "$f"; done
echo "-- sudoers file as installed (mode must not be writable, must be root-owned):"
grep -E 'squashfs-root/etc/sudoers.d/seg-game$' "$list" || true
if [[ $EDITION == full ]]; then
    echo "== gaming packages (full)"
    for p in wine-staging lib32-mangohud mangohud steam gamescope winetricks lutris vulkan-tools lib32-vulkan-icd-loader lib32-mesa linux-zen gamemode; do
        grep -qE "squashfs-root/var/lib/pacman/local/$p-[0-9][^/]*/desc\$" "$list" && echo "present  $p" || echo "ABSENT   $p"
    done
    echo "-- plain wine must be replaced by wine-staging:"
    grep -cE 'squashfs-root/var/lib/pacman/local/wine-[0-9][^/]*/desc$' "$list" | sed 's/^/packages named exactly wine: /'
    echo "-- NVIDIA driver must NOT be on the image (installed by Setup for NVIDIA hardware only):"
    grep -cE 'squashfs-root/var/lib/pacman/local/nvidia-(open-)?dkms-' "$list" | sed 's/^/nvidia dkms packages: /'
fi
echo "== should be gone"
for f in usr/bin/Xorg usr/bin/spectacle usr/bin/inxi; do has "$f"; done
echo "kernel header folders: $(grep -cE 'squashfs-root/usr/src/[^/]+$' "$list")  (0 expected)"
echo "== enabled in the image"
grep -E 'squashfs-root/etc/runlevels/[a-z]+/[^/]+ ->' "$list" | sed -E 's#.*squashfs-root/etc/runlevels/##; s# ->.*##' | sort | tr '\n' ' '
echo
echo "== packages that need the most room (installed size, MiB)"
sudo unsquashfs -f -d /tmp/rt /mnt/iso/LiveOS/rootfs.img var/lib/pacman/local etc/group etc/sudoers.d >/dev/null 2>&1
for d in /tmp/rt/var/lib/pacman/local/*/; do
    n=$(basename "$d"); size=$(awk '/^%SIZE%/{getline; print $1}' "$d/desc"); echo "$((size / 1048576)) $n"
done | sort -rn | head -12
echo "== /etc/group gamemode line:"; grep -E '^gamemode:' /tmp/rt/etc/group || echo "NO gamemode group"
echo "== /etc/sudoers.d/seg-game:"; sudo cat /tmp/rt/etc/sudoers.d/seg-game 2>&1 | grep -v '^#' || true
echo "total installed MiB: $(for d in /tmp/rt/var/lib/pacman/local/*/; do awk '/^%SIZE%/{getline; s+=$1} END{print s+0}' "$d/desc"; done | awk '{t+=$1} END{printf "%d", t / 1048576}')"
