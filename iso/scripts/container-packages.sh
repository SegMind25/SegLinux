#!/bin/bash
# Installs the ISO build tools in the Artix build container (iso/Containerfile).
#
# A mirror part-way through a sync can serve a package that does not match
# its signature ("signature from ... is invalid"), and pacman does not move
# on to the next mirror for that. So the keyring is refreshed first, and when
# the installation fails the package cache is emptied, the first mirror moves
# to the end of the list and the installation is tried again.
#
# Rust comes from rustup (an Artix package) rather than Artix's rust package:
# that one needs compiler-rt, whose signature did not verify on any mirror in
# October 2026. rustup installs the official toolchain (checksums verified)
# into RUSTUP_HOME, shared with the user who builds the package.
set -euo pipefail

packages=(
    artools-base
    artools-iso
    go-yq
    rustup
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
export RUSTUP_HOME=${RUSTUP_HOME:-/opt/rustup}

# Moves the first Server line of the mirror list to the end.
next_mirror() {
    awk '/^Server/ && !moved { first = $0; moved = 1; next }
         { print }
         END { if (moved) print first }' "$mirrorlist" > "$mirrorlist.new"
    mv "$mirrorlist.new" "$mirrorlist"
}

pacman-key --init
pacman-key --populate artix
installed=false
for (( attempt = 1; attempt <= attempts; attempt++ )); do
    if pacman -Syy --noconfirm --needed artix-keyring &&
        pacman-key --populate artix &&
        pacman -Su --noconfirm --needed "${packages[@]}"; then
        installed=true
        break
    fi
    echo "container-packages: attempt $attempt of $attempts failed; trying the next mirror" >&2
    rm -rf /var/cache/pacman/pkg/*
    next_mirror
done
rm -rf /var/cache/pacman/pkg/*
if [[ $installed != true ]]; then
    echo "container-packages: could not install the build tools from any of the first $attempts mirrors" >&2
    exit 1
fi

# Brave comes from Chaotic-AUR (iso/pacman.conf.d/iso-x86_64.conf).
bash "$(dirname "$0")/chaotic-aur.sh"
rm -rf /var/cache/pacman/pkg/*

rustup toolchain install stable --profile minimal
rustup default stable
# The package is built by an unprivileged user.
chmod -R a+rwX "$RUSTUP_HOME"
cargo --version
