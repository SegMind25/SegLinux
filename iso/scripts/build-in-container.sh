#!/usr/bin/env bash
# Build the Wincraft ISO. Runs as root on Artix Linux, normally inside the
# container that iso/build.sh starts.
#
#   INIT=openrc|dinit  OUT_DIR=<dir>  iso/scripts/build-in-container.sh
#
# Steps: build the wincraft package from this checkout into a local pacman
# repository, point artools at it, check the package lists, run buildiso
# and copy the ISO (renamed wincraft-*.iso) plus its checksum to OUT_DIR.
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
init=${INIT:-openrc}
out=${OUT_DIR:-$repo/iso/out}
in_container=${WINCRAFT_CONTAINER:-false}
artools_dir=${CHROOTS_DIR:-/var/lib/artools}
local_repo=/var/lib/wincraft-repo

step() { printf '\n==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

case $init in
    openrc | dinit) ;;
    *) die "unsupported init system '$init' (Wincraft supports openrc and dinit)" ;;
esac
[[ $(id -u) == 0 ]] || die "run as root"
[[ -f /etc/artix-release ]] || [[ $in_container == true ]] || die "this script must run on Artix Linux (use iso/build.sh for a container build)"
for cmd in buildiso makepkg repo-add yq cargo; do
    command -v "$cmd" >/dev/null || die "'$cmd' is missing: pacman -S artools base-devel rust go-yq"
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
id builder >/dev/null 2>&1 || useradd -m builder
chown -R builder: "$pkgwork"
(cd "$pkgwork" && sudo -u builder makepkg -f --noconfirm)

step "Creating the local [wincraft] repository"
rm -rf "$local_repo"
mkdir -p "$local_repo"
cp "$pkgwork"/wincraft-*.pkg.tar.zst "$local_repo/"
repo-add -q "$local_repo/wincraft.db.tar.gz" "$local_repo"/*.pkg.tar.zst

step "Configuring artools"
conf="iso-x86_64.conf"
targets=("${XDG_CONFIG_HOME:-$HOME/.config}/artools/pacman.conf.d")
# Inside the throw-away container also replace artools' default copy.
[[ $in_container == true ]] && targets+=(/usr/share/artools/pacman.conf.d)
for dir in "${targets[@]}"; do
    install -Dm644 "$repo/iso/pacman.conf.d/$conf" "$dir/$conf"
done

# Stage the profiles so file modes can be fixed without touching the
# checkout (git does not record modes such as 0440 for sudoers files).
workspace=$artools_dir/wincraft-workspace
rm -rf "$workspace"
mkdir -p "$workspace"
cp -a "$repo/iso/iso-profiles" "$workspace/"
chown -R root:root "$workspace/iso-profiles"
chmod 0440 "$workspace"/iso-profiles/wincraft/live-overlay/etc/sudoers.d/*
export WORKSPACE_DIR=$workspace

step "Checking package names ($init)"
"$repo/iso/scripts/check-packages.sh" -i "$init"

if [[ $in_container == true ]]; then
    modprobe loop 2>/dev/null || true
    [[ -e /dev/loop-control ]] || mknod -m 0660 /dev/loop-control c 10 237
    for i in $(seq 0 7); do
        [[ -e /dev/loop$i ]] || mknod -m 0660 "/dev/loop$i" b 7 "$i"
    done
fi

step "Running buildiso (profile wincraft, init $init)"
buildiso -p wincraft -i "$init"

step "Collecting the ISO"
iso=$(find "$workspace/iso/wincraft" -maxdepth 1 -name '*.iso' -printf '%T@ %p\n' | sort -n | tail -1 | cut -d' ' -f2-)
[[ -n $iso ]] || die "buildiso did not produce an ISO"
name=$(basename "$iso")
name=${name/artix-wincraft/wincraft}
mkdir -p "$out"
mv "$iso" "$out/$name"
(cd "$out" && sha256sum "$name" > "$name.sha256")
if [[ -n ${OWNER_UID:-} ]]; then
    chown "$OWNER_UID:${OWNER_GID:-$OWNER_UID}" "$out" "$out/$name" "$out/$name.sha256"
fi
step "Done: $out/$name"
