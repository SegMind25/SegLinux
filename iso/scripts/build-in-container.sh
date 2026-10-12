#!/usr/bin/env bash
# Build the Wincraft ISO. Runs as root on Artix Linux, normally inside the
# container that iso/build.sh starts.
#
#   PROFILE=wincraft|wincraft-lite  INIT=openrc|runit|dinit \
#   KERNEL=linux-zen|linux|linux-lts  OUT_DIR=<dir> \
#       iso/scripts/build-in-container.sh
#
# Steps: build the wincraft package from this checkout into a local pacman
# repository, point artools at it, check the package lists, run buildiso
# and copy the ISO (renamed wincraft-*.iso or wincraft-lite-*.iso) plus its
# checksum to OUT_DIR.
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
profile=${PROFILE:-wincraft}
init=${INIT:-openrc}
kernel=${KERNEL:-linux-zen}
out=${OUT_DIR:-$repo/iso/out}
in_container=${WINCRAFT_CONTAINER:-false}
artools_dir=${CHROOTS_DIR:-/var/lib/artools}
local_repo=/var/lib/wincraft-repo
build_user=${BUILD_USER:-builder}

step() { printf '\n==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

case $profile in
    wincraft | wincraft-lite) ;;
    *) die "unknown profile '$profile' (wincraft or wincraft-lite)" ;;
esac
case $init in
    openrc | runit | dinit) ;;
    *) die "unsupported init system '$init' (Wincraft supports openrc, runit and dinit)" ;;
esac
# The Lite live session signs in through OpenRC's agetty (see its profile).
[[ $profile != wincraft-lite || $init == openrc ]] || die "the Lite edition is built with OpenRC only"
case $kernel in
    linux | linux-zen | linux-lts) ;;
    *) die "unsupported kernel '$kernel' (linux-zen, linux or linux-lts)" ;;
esac
[[ $(id -u) == 0 ]] || die "run as root"
[[ -f /etc/artix-release ]] || [[ $in_container == true ]] || die "this script must run on Artix Linux (use iso/build.sh for a container build)"
for cmd in buildiso makepkg repo-add yq cargo; do
    command -v "$cmd" >/dev/null || die "'$cmd' is missing: pacman -S artools-base artools-iso base-devel rust go-yq gtk4 gtk4-layer-shell labwc procps-ng"
done

step "Building the wincraft package"
version=$(sed -n 's/^pkgver=//p' "$repo/packaging/wincraft/PKGBUILD")
pkgwork=/var/tmp/wincraft-pkg
rm -rf "$pkgwork"
mkdir -p "$pkgwork"
cp "$repo/packaging/wincraft/PKGBUILD" "$repo/packaging/wincraft/wincraft.install" "$pkgwork/"
tar -C "$repo" \
    --exclude=./target --exclude=./.git --exclude=./iso/out \
    --exclude=./packaging/wincraft/src --exclude=./packaging/wincraft/pkg \
    --exclude='*.pkg.tar.*' --exclude='*.iso' \
    --transform "s,^\.,wincraft-$version," \
    -czf "$pkgwork/wincraft-$version.tar.gz" .
if ! id "$build_user" >/dev/null 2>&1; then
    [[ $build_user == builder ]] || die "unknown package build user: $build_user"
    useradd -m builder
fi
[[ $(id -u "$build_user") != 0 ]] || die "the package build user must not be root"
chown -R "$build_user:" "$pkgwork"
(cd "$pkgwork" && sudo -H -u "$build_user" env CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" makepkg -f --noconfirm)

step "Creating the local [wincraft] repository"
rm -rf "$local_repo"
mkdir -p "$local_repo"
cp "$pkgwork"/wincraft-*.pkg.tar.zst "$local_repo/"
repo-add -q "$local_repo/wincraft.db.tar.gz" "$local_repo"/*.pkg.tar.zst

step "Configuring artools"
conf="iso-x86_64.conf"
# Use one explicit config location under both sudo and pkexec. artools
# otherwise resolves SUDO_USER while this script resolves root HOME.
export XDG_CONFIG_HOME="$artools_dir/wincraft-config"
targets=("$XDG_CONFIG_HOME/artools/pacman.conf.d")
# Inside the throw-away container also replace artools' default copy.
[[ $in_container == true ]] && targets+=(/usr/share/artools/pacman.conf.d)
for dir in "${targets[@]}"; do
    install -Dm644 "$repo/iso/pacman.conf.d/$conf" "$dir/$conf"
    if [[ $profile == wincraft-lite ]]; then
        # Keep documentation, manuals and translations off the Lite image.
        # The installed system's own pacman.conf (root-overlay) has no such
        # rule, so packages installed later are complete.
        sed -i '/^\[options\]/a NoExtract = usr/share/doc/* usr/share/gtk-doc/* usr/share/help/* usr/share/info/* usr/share/man/*\nNoExtract = usr/share/locale/* !usr/share/locale/locale.alias' \
            "$dir/$conf"
    fi
done

# The Lite profile links to files of the full one; artools copies overlays
# with cp -L, which fails late on a broken link.
broken=$(find "$repo/iso/iso-profiles" -xtype l)
[[ -z $broken ]] || die "broken links in iso/iso-profiles: $broken"

# Stage the profiles so file modes can be fixed without touching the
# checkout (git does not record modes such as 0440 for sudoers files).
workspace=$artools_dir/wincraft-workspace
rm -rf "$workspace"
mkdir -p "$workspace"
cp -a "$repo/iso/iso-profiles" "$workspace/"
chown -R root:root "$workspace/iso-profiles"
chmod 0440 "$workspace"/iso-profiles/wincraft/live-overlay/etc/sudoers.d/*
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
use_kernel() {
    sed -i -E "s/^(\s*-\s*)linux$/\1$kernel/; s/^(\s*-\s*)linux-headers$/\1$kernel-headers/" \
        "$workspace/iso-profiles/common/common.yaml"
    [[ $kernel == linux ]] && return
    # shellcheck disable=SC2016 # literal shell code, not expansions
    local header='LIBDIR=${LIBDIR:-' old='k=$(<"$mnt"/usr/src/linux/version)'
    # shellcheck disable=SC2016
    local new='k=$(basename "$(dirname "$(find "$mnt"/usr/lib/modules -mindepth 2 -maxdepth 2 -name vmlinuz | sort -V | tail -n1)")")'
    grep -qF "$header" "$(command -v buildiso)" || die "this buildiso does not honour LIBDIR; build with -k linux"
    local patched=$artools_dir/wincraft-artools-lib
    rm -rf "$patched"
    cp -a /usr/share/artools/lib "$patched"
    local file=$patched/iso/initcpio.sh text
    grep -qF "$old" "$file" || die "artools changed how it finds the kernel version in $file; build with -k linux"
    text=$(<"$file")
    printf '%s\n' "${text/"$old"/"$new"}" > "$file"
    export LIBDIR=$patched
}
step "Using the $kernel kernel"
use_kernel

step "Checking package names ($profile, $init, $kernel)"
"$repo/iso/scripts/check-packages.sh" -i "$init" -k "$kernel" -p "$profile"
if [[ $profile == wincraft-lite ]]; then
    # Setup downloads the rest of the full edition: its list must resolve too.
    "$repo/iso/scripts/check-packages.sh" -i "$init" -k "$kernel" -p wincraft
    step "Listing the packages Setup downloads"
    online=$workspace/iso-profiles/$profile/root-overlay/usr/share/wincraft/online-packages
    mkdir -p "$(dirname "$online")"
    "$repo/iso/scripts/online-packages.sh" -i "$init" -k "$kernel" > "$online"
    echo "$(grep -vc '^#' "$online") packages"
    # Smaller than artools' default zstd, at the cost of a slower live boot.
    export COMPRESSION=xz
fi

if [[ $in_container == true ]]; then
    modprobe loop 2>/dev/null || true
    [[ -e /dev/loop-control ]] || mknod -m 0660 /dev/loop-control c 10 237
    for i in $(seq 0 7); do
        [[ -e /dev/loop$i ]] || mknod -m 0660 "/dev/loop$i" b 7 "$i"
    done
fi

step "Running buildiso (profile $profile, init $init, kernel $kernel)"
buildiso -p "$profile" -i "$init"

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
    echo "Largest packages of the installed system (uncompressed):"
    pacman --dbpath "$db" -Qi | awk '
        /^Name / { name = $3 }
        /^Installed Size/ {
            unit = $5; size = $4
            if (unit == "B") size /= 1048576; else if (unit == "KiB") size /= 1024; else if (unit == "GiB") size *= 1024
            total += size; printf "%9.1f MiB  %s\n", size, name
        }
        END { printf "%9.1f MiB  TOTAL\n", total }' | sort -rn | head -40
}
step "What takes the space"
report_sizes || true
step "Done: $out/$name ($(du -h "$out/$name" | cut -f1))"
