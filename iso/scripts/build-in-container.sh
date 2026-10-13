#!/usr/bin/env bash
# Build the SegLinux ISO. Runs as root on Artix Linux, normally inside the
# container that iso/build.sh starts.
#
#   PROFILE=seglinux|seglinux-lite  INIT=openrc|runit|dinit \
#   KERNEL=linux-zen|linux|linux-lts  OUT_DIR=<dir> \
#       iso/scripts/build-in-container.sh
#
# Steps: build the seglinux package from this checkout into a local pacman
# repository, point artools at it, check the package lists, run buildiso
# and copy the ISO (renamed seglinux-*.iso or seglinux-lite-*.iso) plus its
# checksum to OUT_DIR.
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
profile=${PROFILE:-seglinux}
init=${INIT:-openrc}
kernel=${KERNEL:-linux-zen}
out=${OUT_DIR:-$repo/iso/out}
in_container=${SEGLINUX_CONTAINER:-false}
artools_dir=${CHROOTS_DIR:-/var/lib/artools}
local_repo=/var/lib/seglinux-repo
build_user=${BUILD_USER:-builder}

step() { printf '\n==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

case $profile in
    seglinux | seglinux-lite) ;;
    *) die "unknown profile '$profile' (seglinux or seglinux-lite)" ;;
esac
case $init in
    openrc | runit | dinit) ;;
    *) die "unsupported init system '$init' (SegLinux supports openrc, runit and dinit)" ;;
esac
# The Lite live session signs in through OpenRC's agetty (see its profile).
[[ $profile != seglinux-lite || $init == openrc ]] || die "the Lite edition is built with OpenRC only"
case $kernel in
    linux | linux-zen | linux-lts) ;;
    *) die "unsupported kernel '$kernel' (linux-zen, linux or linux-lts)" ;;
esac
[[ $(id -u) == 0 ]] || die "run as root"
[[ -f /etc/artix-release ]] || [[ $in_container == true ]] || die "this script must run on Artix Linux (use iso/build.sh for a container build)"
for cmd in buildiso makepkg repo-add yq cargo; do
    command -v "$cmd" >/dev/null || die "'$cmd' is missing: pacman -S artools-base artools-iso base-devel rust (or rustup) go-yq gtk4 gtk4-layer-shell labwc procps-ng"
done

step "Building the seglinux package"
version=$(sed -n 's/^pkgver=//p' "$repo/packaging/seglinux/PKGBUILD")
pkgwork=/var/tmp/seglinux-pkg
rm -rf "$pkgwork"
mkdir -p "$pkgwork"
cp "$repo/packaging/seglinux/PKGBUILD" "$repo/packaging/seglinux/seglinux.install" "$pkgwork/"
# CI numbers each commit (0.1.0.<commits>): seglinux-update installs a newer
# package when the number grows.
if [[ -n ${SEGLINUX_PKGVER:-} ]]; then
    [[ $SEGLINUX_PKGVER =~ ^[0-9][0-9.]*$ ]] || die "invalid SEGLINUX_PKGVER: $SEGLINUX_PKGVER"
    version=$SEGLINUX_PKGVER
    sed -i "s/^pkgver=.*/pkgver=$version/" "$pkgwork/PKGBUILD"
fi
tar -C "$repo" \
    --exclude=./target --exclude=./.git --exclude=./iso/out \
    --exclude=./packaging/seglinux/src --exclude=./packaging/seglinux/pkg \
    --exclude='*.pkg.tar.*' --exclude='*.iso' \
    --transform "s,^\.,seglinux-$version," \
    -czf "$pkgwork/seglinux-$version.tar.gz" .
if ! id "$build_user" >/dev/null 2>&1; then
    [[ $build_user == builder ]] || die "unknown package build user: $build_user"
    useradd -m builder
fi
[[ $(id -u "$build_user") != 0 ]] || die "the package build user must not be root"
chown -R "$build_user:" "$pkgwork"
# sudo resets the environment: pass on where rustup keeps the toolchain.
(cd "$pkgwork" && sudo -H -u "$build_user" env CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
    ${RUSTUP_HOME:+"RUSTUP_HOME=$RUSTUP_HOME"} makepkg -f --noconfirm)

step "Creating the local [seglinux] repository"
rm -rf "$local_repo"
mkdir -p "$local_repo"
cp "$pkgwork"/seglinux-*.pkg.tar.zst "$local_repo/"
repo-add -q "$local_repo/seglinux.db.tar.gz" "$local_repo"/*.pkg.tar.zst
# The package also goes next to the ISO: CI publishes it as an update.
mkdir -p "$out/packages"
rm -f "$out"/packages/seglinux-*.pkg.tar.zst
cp "$pkgwork"/seglinux-*.pkg.tar.zst "$out/packages/"
[[ -z ${OWNER_UID:-} ]] || chown -R "$OWNER_UID:${OWNER_GID:-$OWNER_UID}" "$out/packages"

step "Configuring artools"
conf="iso-x86_64.conf"
# Use one explicit config location under both sudo and pkexec. artools
# otherwise resolves SUDO_USER while this script resolves root HOME.
export XDG_CONFIG_HOME="$artools_dir/seglinux-config"
targets=("$XDG_CONFIG_HOME/artools/pacman.conf.d")
# Inside the throw-away container also replace artools' default copy.
[[ $in_container == true ]] && targets+=(/usr/share/artools/pacman.conf.d)
for dir in "${targets[@]}"; do
    install -Dm644 "$repo/iso/pacman.conf.d/$conf" "$dir/$conf"
    if [[ $profile == seglinux-lite ]]; then
        # Files the Lite image leaves out; Setup restores them on install.
        # The installed system's own pacman.conf (root-overlay) has no such
        # rule, so packages installed later are complete.
        sed -i "/^\[options\]/r $repo/iso/iso-profiles/seglinux-lite/pacman-noextract.conf" "$dir/$conf"
    fi
done

# The Lite profile links to files of the full one; artools copies overlays
# with cp -L, which fails late on a broken link.
broken=$(find "$repo/iso/iso-profiles" -xtype l)
[[ -z $broken ]] || die "broken links in iso/iso-profiles: $broken"

# Stage the profiles so file modes can be fixed without touching the
# checkout (git does not record modes such as 0440 for sudoers files).
workspace=$artools_dir/seglinux-workspace
rm -rf "$workspace"
mkdir -p "$workspace"
cp -a "$repo/iso/iso-profiles" "$workspace/"
chown -R root:root "$workspace/iso-profiles"
chmod 0440 "$workspace"/iso-profiles/seglinux/live-overlay/etc/sudoers.d/*
export WORKSPACE_DIR=$workspace
# A profile can bring its own shared package list (the Lite edition does).
if [[ -f $workspace/iso-profiles/$profile/common.yaml ]]; then
    cp "$workspace/iso-profiles/$profile/common.yaml" "$workspace/iso-profiles/common/common.yaml"
fi

# Kernel: common.yaml names the stock kernel, which artools expects; swap in
# the requested one. artools derives the live initramfs' kernel version from
# /usr/src/linux/version, which only the stock linux-headers provides, so for
# other kernels buildiso runs with a patched copy of its library (buildiso
# honours LIBDIR) that reads the version from /usr/lib/modules instead.
# Copy artools' library once so that buildiso (which honours LIBDIR) runs
# with patched code; the installed artools files stay untouched.
patched_lib() {
    local patched=$artools_dir/seglinux-artools-lib
    if [[ ${LIBDIR:-} != "$patched" ]]; then
        # shellcheck disable=SC2016 # literal shell code, not expansions
        grep -qF 'LIBDIR=${LIBDIR:-' "$(command -v buildiso)" || return 1
        rm -rf "$patched"
        cp -a /usr/share/artools/lib "$patched"
        export LIBDIR=$patched
    fi
}
# Replace the text $2 with $3 in the artools library file $1.
patch_lib() {
    local file=$LIBDIR/$1 text
    grep -qF "$2" "$file" || return 1
    text=$(<"$file")
    printf '%s\n' "${text/"$2"/"$3"}" > "$file"
}
use_kernel() {
    sed -i -E "s/^(\s*-\s*)linux$/\1$kernel/; s/^(\s*-\s*)linux-headers$/\1$kernel-headers/" \
        "$workspace/iso-profiles/common/common.yaml"
    [[ $kernel == linux ]] && return
    patched_lib || die "this buildiso does not honour LIBDIR; build with -k linux"
    # shellcheck disable=SC2016 # literal shell code, not expansions
    patch_lib iso/initcpio.sh 'k=$(<"$mnt"/usr/src/linux/version)' \
        'k=$(basename "$(dirname "$(find "$mnt"/usr/lib/modules -mindepth 2 -maxdepth 2 -name vmlinuz | sort -V | tail -n1)")")' ||
        die "artools changed how it finds the kernel version in iso/initcpio.sh; build with -k linux"
}
step "Using the $kernel kernel"
use_kernel

step "Checking package names ($profile, $init, $kernel)"
"$repo/iso/scripts/check-packages.sh" -i "$init" -k "$kernel" -p "$profile"
if [[ $profile == seglinux-lite ]]; then
    # Setup downloads the rest of the full edition: its list must resolve too.
    "$repo/iso/scripts/check-packages.sh" -i "$init" -k "$kernel" -p seglinux
    step "Listing the packages Setup downloads"
    online=$workspace/iso-profiles/$profile/root-overlay/usr/share/seglinux/online-packages
    mkdir -p "$(dirname "$online")"
    "$repo/iso/scripts/online-packages.sh" -i "$init" -k "$kernel" > "$online"
    echo "$(grep -vc '^#' "$online") packages"
    # Smaller than artools' default zstd, at the cost of a slower live boot:
    # xz with 1 MiB blocks and the x86 filter for programs and libraries.
    export COMPRESSION=xz
    export SEGLINUX_MKSQUASHFS_OPTS="-b 1M -Xbcj x86"
fi

# Before each squashfs image: stop processes that package scripts left
# running inside the build chroots and unmount what they kept mounted.
# artools cannot unmount a busy chroot, and mksquashfs then reads its /proc
# and blocks for ever on files such as /proc/kmsg (a full-edition build hung
# for four hours on it).
seglinux_release_chroots() {
    local root=$SEGLINUX_CHROOTS p m
    [[ -n $root && -d $root ]] || return 0
    for p in /proc/[0-9]*; do
        [[ $(readlink "$p/root" 2>/dev/null) == "$root"/* ]] || continue
        echo "seglinux: stopping $(cat "$p/comm" 2>/dev/null) (pid ${p#/proc/}), left running in the chroot" >&2
        kill -KILL "${p#/proc/}" 2>/dev/null || true
    done
    findmnt -rn -o TARGET | awk -v r="$root/" 'index($0, r) == 1' | sort -r |
        while read -r m; do
            echo "seglinux: unmounting $m, left mounted in the chroot" >&2
            umount -l "$m" || true
        done
}
export -f seglinux_release_chroots
export SEGLINUX_CHROOTS=$artools_dir/buildiso/$profile
# shellcheck disable=SC2016 # literal shell code, not expansions
if patched_lib && patch_lib iso/squash.sh 'mksquashfs "${mksfs_args[@]}"' \
    '{ declare -F seglinux_release_chroots >/dev/null && seglinux_release_chroots; } || true
        mksquashfs "${mksfs_args[@]}" ${SEGLINUX_MKSQUASHFS_OPTS:-}'; then
    echo "squashfs: chroots released before squashing; extra options: ${SEGLINUX_MKSQUASHFS_OPTS:-none}"
else
    echo "warning: cannot patch artools' mksquashfs call; a busy chroot can hang the build" >&2
fi

if [[ $in_container == true ]]; then
    modprobe loop 2>/dev/null || true
    [[ -e /dev/loop-control ]] || mknod -m 0660 /dev/loop-control c 10 237
    for i in $(seq 0 7); do
        [[ -e /dev/loop$i ]] || mknod -m 0660 "/dev/loop$i" b 7 "$i"
    done
fi

# SegLinux's name and artwork on the install media: run a copy of buildiso
# that labels the image SEGLINUX_<date> instead of ARTIX_<date> and loads
# iso/scripts/buildiso-hooks.sh after artools' libraries (boot menu theme
# and entries, the image's metadata).
seglinux_buildiso() {
    local copy=$artools_dir/seglinux-bin/buildiso
    mkdir -p "$(dirname "$copy")"
    awk -v hooks="$repo/iso/scripts/buildiso-hooks.sh" '
        { line[NR] = $0 }
        /^[[:space:]]*(source|\.)[[:space:]].*LIBDIR/ { last = NR }
        END {
            for (i = 1; i <= NR; i++) {
                print line[i]
                if (i == last) printf "source \"%s\"\n", hooks
            }
        }' "$(command -v buildiso)" | sed 's/iso_label="ARTIX_/iso_label="SEGLINUX_/' > "$copy"
    chmod 755 "$copy"
    grep -q 'iso_label="SEGLINUX_' "$copy" ||
        echo "warning: artools names the image differently; it keeps the Artix volume label" >&2
    grep -qF "$repo/iso/scripts/buildiso-hooks.sh" "$copy" ||
        echo "warning: cannot load the SegLinux boot menu branding into buildiso" >&2
    # The EFI partition of the image, which also shows up when the USB
    # stick is plugged in.
    if patched_lib; then
        grep -rlF ARTIX_EFI "$LIBDIR" | xargs -r sed -i 's/ARTIX_EFI/SEGLINUX/g'
    fi
    buildiso=$copy
}
seglinux_buildiso

step "Running buildiso (profile $profile, init $init, kernel $kernel)"
"$buildiso" -p "$profile" -i "$init"

step "Collecting the ISO"
iso=$(find "$workspace/iso/$profile" -maxdepth 1 -name '*.iso' -printf '%T@ %p\n' | sort -n | tail -1 | cut -d' ' -f2-)
[[ -n $iso ]] || die "buildiso did not produce an ISO"
name=$(basename "$iso")
name=${name/"artix-$profile"/$profile}
mkdir -p "$out"
mv "$iso" "$out/$name"
(cd "$out" && sha256sum "$name" > "$name.sha256")
if [[ -n ${OWNER_UID:-} ]]; then
    chown "$OWNER_UID:${OWNER_GID:-$OWNER_UID}" "$out" "$out/$name" "$out/$name.sha256"
fi

# Where the space goes, for keeping the images small. Informational only.
report_sizes() {
    echo "Largest files on the image:"
    xorriso -indev "$out/$name" -find / -type f -exec lsdl -- 2>/dev/null |
        awk '$5 > 1048576 {printf "%9.1f MiB  %s\n", $5 / 1048576, $NF}' | sort -rn | head -15
    local db=$artools_dir/buildiso/$profile/artix/rootfs/var/lib/pacman
    [[ -d $db/local ]] || return 0
    echo "Largest packages of the installed system (uncompressed), and what needs them:"
    pacman --dbpath "$db" -Qi 2>/dev/null | awk -F' *: ' '
        /^Name / { name = $2 }
        /^Required By/ { needed = $2 }
        /^Installed Size/ {
            split($2, v, " "); size = v[1]; unit = v[2]
            if (unit == "B") size /= 1048576; else if (unit == "KiB") size /= 1024; else if (unit == "GiB") size *= 1024
            total += size; printf "%9.1f MiB  %-28s <- %.90s\n", size, name, needed
        }
        END { printf "%9.1f MiB  TOTAL\n", total }' | sort -rn | head -45
    local root=${db%/var/lib/pacman}
    echo "Largest firmware and driver folders on the image:"
    du -sm "$root"/usr/lib/firmware/* "$root"/usr/lib/modules/*/kernel/drivers/* 2>/dev/null |
        sort -rn | head -20 | sed "s|$root||"
}
step "What takes the space"
report_sizes || true
step "Done: $out/$name ($(du -h "$out/$name" | cut -f1))"
