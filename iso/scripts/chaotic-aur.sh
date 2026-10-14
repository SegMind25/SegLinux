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

# From the key server (its HKPS port, then HKP on port 80), else the same
# key over HTTPS; only the key with this ID is trusted below.
receive() {
    local server file
    for server in hkps://keyserver.ubuntu.com hkp://keyserver.ubuntu.com:80; do
        pacman-key --recv-key "$key" --keyserver "$server" && return 0
    done
    file=$(mktemp)
    if curl -fsSL "https://keyserver.ubuntu.com/pks/lookup?op=get&options=mr&search=0x$key" -o "$file" &&
        pacman-key --add "$file"; then
        rm -f "$file"
        return 0
    fi
    rm -f "$file"
    return 1
}
received=false
for attempt in 1 2 3; do
    if receive; then
        received=true
        break
    fi
    echo "chaotic-aur: the key could not be fetched (attempt $attempt of 3)" >&2
    sleep $((attempt * 5))
done
[[ $received == true ]] || { echo "chaotic-aur: cannot get the Chaotic-AUR key" >&2; exit 1; }
pacman-key --lsign-key "$key"
pacman -U --noconfirm --needed "$cdn/chaotic-keyring.pkg.tar.zst" "$cdn/chaotic-mirrorlist.pkg.tar.zst"
