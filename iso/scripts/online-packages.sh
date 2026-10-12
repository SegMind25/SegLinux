#!/usr/bin/env bash
# Print the packages the full SegLinux edition installs that the Lite
# edition leaves out of its image: SegLinux Setup downloads them into the
# new system (/usr/share/seglinux/online-packages on the Lite image). Needs
# go-yq.
#
#   iso/scripts/online-packages.sh [-i openrc|runit|dinit] [-k kernel]
set -euo pipefail
export LC_ALL=C # one sort order for sort and comm

profiles=$(cd "$(dirname "$0")/../iso-profiles" && pwd)
init=openrc
kernel=linux

while getopts "i:k:h" opt; do
    case $opt in
        i) init=$OPTARG ;;
        k) kernel=$OPTARG ;;
        *) sed -n '2,7s/^# \{0,1\}//p' "$0"; exit 2 ;;
    esac
done
command -v yq >/dev/null || { echo "yq (go-yq) is required" >&2; exit 1; }

list() { yq ".$1 // [] | .[]" "$2"; }

# The rootfs layer (the installed system) of a profile, as check-packages.sh
# computes it.
rootfs() {
    local common=$1 prof=$2 xorg=packages-xorg
    if [[ $(yq '.["live-session"]["use-xlibre"] // false' "$prof") == true ]]; then
        xorg=packages-xlibre
    fi
    {
        list '["packages-base"]' "$common"
        list '["packages-apps"]' "$common"
        list "[\"$xorg\"]" "$common"
        list '["packages-misc"]' "$common"
        list "[\"packages-init\"].$init" "$common"
        list 'rootfs.packages' "$prof"
        list "rootfs[\"packages-init\"].$init" "$prof"
    } | sed -e "s/^linux$/$kernel/" -e "s/^linux-headers$/$kernel-headers/" | sort -u
}

full=$(rootfs "$profiles/common/common.yaml" "$profiles/seglinux/profile.yaml")
lite=$(rootfs "$profiles/seglinux-lite/common.yaml" "$profiles/seglinux-lite/profile.yaml")

echo "# Packages SegLinux Setup downloads when it installs the Lite edition:"
echo "# the full edition's ($init, $kernel) minus what the Lite image has."
# seglinux itself is built locally and already on the image.
comm -23 <(echo "$full") <(echo "$lite") | grep -vx seglinux
