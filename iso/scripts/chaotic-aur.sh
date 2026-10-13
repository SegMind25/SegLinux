#!/bin/bash
# Sets up the Chaotic-AUR repository (https://aur.chaotic.cx) on an Artix
# system: its signing key, then its keyring and mirror list packages, which
# pacman checks against that key. SegLinux takes Brave from it: Brave is in
# neither Artix's nor Arch's repositories. Garuda Linux uses it by default.
#
# Used by the ISO build container and CI; installed systems get the same
# steps from seglinux-update. A pacman.conf then needs:
#   [chaotic-aur]
#   Include = /etc/pacman.d/chaotic-mirrorlist
set -euo pipefail

key=3056513887B78AEB
cdn=https://cdn-mirror.chaotic.cx/chaotic-aur

# Signing the key locally needs pacman's own key, which a fresh container
# may not have yet (this does nothing when it has).
pacman-key --init

received=false
for attempt in 1 2 3; do
    if pacman-key --recv-key "$key" --keyserver keyserver.ubuntu.com; then
        received=true
        break
    fi
    echo "chaotic-aur: the key server did not answer (attempt $attempt of 3)" >&2
    sleep $((attempt * 5))
done
[[ $received == true ]] || { echo "chaotic-aur: cannot get the Chaotic-AUR key" >&2; exit 1; }
pacman-key --lsign-key "$key"
pacman -U --noconfirm --needed "$cdn/chaotic-keyring.pkg.tar.zst" "$cdn/chaotic-mirrorlist.pkg.tar.zst"
