#!/usr/bin/env bash
# Build the Wincraft ISO.
#
#   iso/build.sh [-i openrc|dinit] [-e docker|podman] [--native]
#
# By default the build runs in a privileged Artix container (docker or
# podman), so any Linux host works. With --native the build runs directly
# on an Artix host (needs root, artools, base-devel, rust, go-yq).
# The ISO and its .sha256 end up in iso/out/.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
init=openrc
engine=
native=false

usage() { sed -n '2,10s/^# \{0,1\}//p' "$0"; }

while (($#)); do
    case $1 in
        -i | --init) init=$2; shift 2 ;;
        -e | --engine) engine=$2; shift 2 ;;
        --native) native=true; shift ;;
        -h | --help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done

if $native; then
    exec sudo env INIT="$init" OUT_DIR="$repo/iso/out" \
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

image=wincraft-iso-builder
"${sudo[@]}" "$engine" build -t "$image" -f "$repo/iso/Containerfile" "$repo/iso"
# The artools volume lives on the host's file system: buildiso stacks
# overlayfs mounts there, which cannot sit on the container's own overlay.
"${sudo[@]}" "$engine" run --rm --privileged \
    -v "$repo:/src" \
    -v wincraft-artools:/var/lib/artools \
    -v wincraft-pacman-cache:/var/cache/pacman/pkg \
    -e INIT="$init" \
    -e OUT_DIR=/src/iso/out \
    -e OWNER_UID="$(id -u)" \
    -e OWNER_GID="$(id -g)" \
    "$image" /src/iso/scripts/build-in-container.sh
echo "ISO written to $repo/iso/out/"
