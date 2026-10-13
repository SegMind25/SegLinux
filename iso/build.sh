#!/usr/bin/env bash
# Build the SegLinux ISO.
#
#   iso/build.sh [--lite] [-i openrc|runit|dinit] [-k linux-zen|linux|linux-lts]
#                [-e docker|podman] [--native]
#
# Defaults: the full edition, OpenRC and the linux-zen kernel. --lite builds
# the small Lite edition (OpenRC only), whose Setup downloads the apps.
# The build runs in a privileged Artix container (docker or podman), so any
# Linux host works; --native runs it on an Artix host (docs/building.md).
# The ISO and its .sha256 end up in iso/out/.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
profile=seglinux
init=openrc
kernel=linux-zen
engine=
native=false

usage() { sed -n '2,12s/^# \{0,1\}//p' "$0"; }

while (($#)); do
    case $1 in
        --lite) profile=seglinux-lite; shift ;;
        -p | --profile) profile=$2; shift 2 ;;
        -i | --init) init=$2; shift 2 ;;
        -k | --kernel) kernel=$2; shift 2 ;;
        -e | --engine) engine=$2; shift 2 ;;
        --native) native=true; shift ;;
        -h | --help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done

case $profile in
    seglinux | seglinux-lite) ;;
    *) echo "error: profile must be seglinux or seglinux-lite" >&2; exit 2 ;;
esac
case $init in
    openrc | runit | dinit) ;;
    *) echo "error: init must be openrc, runit or dinit" >&2; exit 2 ;;
esac
if [[ $profile == seglinux-lite && $init != openrc ]]; then
    echo "error: the Lite edition is built with OpenRC only" >&2
    exit 2
fi
case $kernel in
    linux | linux-zen | linux-lts) ;;
    *) echo "error: kernel must be linux-zen, linux or linux-lts" >&2; exit 2 ;;
esac

if $native; then
    for cmd in buildiso makepkg repo-add yq cargo; do
        if ! command -v "$cmd" >/dev/null; then
            echo "error: missing $cmd. See docs/building.md for the native prerequisites." >&2
            exit 1
        fi
    done
    exec sudo env PROFILE="$profile" INIT="$init" KERNEL="$kernel" OUT_DIR="$repo/iso/out" BUILD_USER="$(id -un)" \
        OWNER_UID="$(id -u)" OWNER_GID="$(id -g)" \
        "$repo/iso/scripts/build-in-container.sh"
fi

if [[ -z $engine ]]; then
    if command -v podman >/dev/null; then
        engine=podman
    elif command -v docker >/dev/null; then
        engine=docker
    else
        echo "error: install docker or podman, or build on Artix with --native" >&2
        exit 1
    fi
fi
sudo=()
# Rootless podman cannot run the privileged build.
if [[ $engine == podman && $(id -u) != 0 ]]; then
    sudo=(sudo)
fi

image=seglinux-iso-builder
"${sudo[@]}" "$engine" build -t "$image" -f "$repo/iso/Containerfile" "$repo/iso"
# The artools volume lives on the host's file system: buildiso stacks
# overlayfs mounts there, which cannot sit on the container's own overlay.
"${sudo[@]}" "$engine" run --rm --privileged \
    -v "$repo:/src" \
    -v seglinux-artools:/var/lib/artools \
    -v seglinux-pacman-cache:/var/cache/pacman/pkg \
    -e PROFILE="$profile" \
    -e INIT="$init" \
    -e KERNEL="$kernel" \
    -e OUT_DIR=/src/iso/out \
    -e OWNER_UID="$(id -u)" \
    -e OWNER_GID="$(id -g)" \
    -e SEGLINUX_PKGVER="${SEGLINUX_PKGVER:-}" \
    "$image" /src/iso/scripts/build-in-container.sh
echo "ISO written to $repo/iso/out/"
