#!/bin/bash
# Installs the ISO build tools in the Artix build container (iso/Containerfile).
#
# A mirror part-way through a sync can serve a package that does not match
# its signature ("signature from ... is invalid"), and pacman does not move
# on to the next mirror for that. So the keyring is refreshed first, and when
# the installation fails the package cache is emptied, the first mirror moves
# to the end of the list and the installation is tried again.
set -euo pipefail

packages=(
    artools-base
    artools-iso
    go-yq
    rust
    gtk4
    gtk4-layer-shell
    labwc
    procps-ng
    pkgconf
    squashfs-tools
    libisoburn
    mtools
    dosfstools
    grub
    sudo
    perl
)
mirrorlist=${MIRRORLIST:-/etc/pacman.d/mirrorlist}
attempts=4

# Moves the first Server line of the mirror list to the end.
next_mirror() {
    awk '/^Server/ && !moved { first = $0; moved = 1; next }
         { print }
         END { if (moved) print first }' "$mirrorlist" > "$mirrorlist.new"
    mv "$mirrorlist.new" "$mirrorlist"
}

pacman-key --init
pacman-key --populate artix
for (( attempt = 1; attempt <= attempts; attempt++ )); do
    if pacman -Syy --noconfirm --needed artix-keyring &&
        pacman-key --populate artix &&
        pacman -Su --noconfirm --needed "${packages[@]}"; then
        rm -rf /var/cache/pacman/pkg/*
        exit 0
    fi
    echo "container-packages: attempt $attempt of $attempts failed; trying the next mirror" >&2
    rm -rf /var/cache/pacman/pkg/*
    next_mirror
done
echo "container-packages: could not install the build tools from any of the first $attempts mirrors" >&2
exit 1
