#!/usr/bin/env bash
# Check that every package the ISO profile names exists in the repositories
# buildiso will install from, and that each layer's package set resolves.
# Run on Artix (or in the build container); needs pacman and go-yq.
#
#   iso/scripts/check-packages.sh [-i openrc|runit|dinit|s6] [-k kernel]
#                                 [-p profile] [-c pacman.conf]
set -euo pipefail

iso_dir=$(cd "$(dirname "$0")/.." && pwd)
init=openrc
kernel=linux
profile=seglinux
conf=$iso_dir/pacman.conf.d/iso-x86_64.conf
local_repo=/var/lib/seglinux-repo

while getopts "i:k:p:c:h" opt; do
    case $opt in
        i) init=$OPTARG ;;
        k) kernel=$OPTARG ;;
        p) profile=$OPTARG ;;
        c) conf=$OPTARG ;;
        *) sed -n '2,9s/^# \{0,1\}//p' "$0"; exit 2 ;;
    esac
done

prof=$iso_dir/iso-profiles/$profile/profile.yaml
# A profile can bring its own shared package list (the Lite edition does).
common=$iso_dir/iso-profiles/$profile/common.yaml
[[ -f $common ]] || common=$iso_dir/iso-profiles/common/common.yaml
[[ -f $prof ]] || { echo "no profile at $prof" >&2; exit 1; }
command -v yq >/dev/null || { echo "yq (go-yq) is required" >&2; exit 1; }

# Print the list at a yq path (keys contain dashes, hence the brackets).
list() { yq ".$1 // [] | .[]" "$2"; }

xorg=packages-xorg
if [[ $(yq '.["live-session"]["use-xlibre"] // false' "$prof") == true ]]; then
    xorg=packages-xlibre
fi

rootfs=$(
    {
        list '["packages-base"]' "$common"
        list '["packages-apps"]' "$common"
        list "[\"$xorg\"]" "$common"
        list '["packages-misc"]' "$common"
        list "[\"packages-init\"].$init" "$common"
        list 'rootfs.packages' "$prof"
        list "rootfs[\"packages-init\"].$init" "$prof"
    } | sed -e "s/^linux$/$kernel/" -e "s/^linux-headers$/$kernel-headers/" | sort -u
)
livefs=$(
    {
        list 'livefs.packages' "$prof"
        list "livefs[\"packages-init\"].$init" "$prof"
        list '["packages-boot"]' "$common"
    } | sort -u
)

db=$(mktemp -d)
work_conf=$(mktemp)
trap 'rm -rf "$db" "$work_conf"' EXIT
# pacman downloads as the unprivileged DownloadUser, which must reach the DB path.
chmod 755 "$db"
cp "$conf" "$work_conf"

skip=()
if [[ ! -f $local_repo/seglinux.db ]]; then
    # The seglinux package is built during the ISO build; when checking
    # before that, leave the local repository out.
    awk '/^\[seglinux\]/{skip=1; next} /^\[/{skip=0} !skip' "$conf" > "$work_conf"
    skip=(seglinux)
    echo "note: $local_repo has no repository yet; not checking: ${skip[*]}"
fi

pac() { pacman --config "$work_conf" --dbpath "$db" --noconfirm "$@"; }
pac -Sy >/dev/null

mapfile -t available < <(pac -Slq)
declare -A have=()
for name in "${available[@]}"; do have[$name]=1; done

missing=()
for name in $rootfs $livefs; do
    [[ " ${skip[*]} " == *" $name "* ]] && continue
    [[ -n ${have[$name]:-} ]] && continue
    # Not a package name: maybe a group or a virtual package someone provides.
    pac -Sp --print-format '%n' "$name" >/dev/null 2>&1 || missing+=("$name")
done
if ((${#missing[@]})); then
    echo "Packages not found in the repositories (init: $init):" >&2
    printf '  %s\n' "${missing[@]}" >&2
    exit 1
fi

resolve() {
    local layer=$1; shift
    local pkgs=()
    for name in "$@"; do
        [[ " ${skip[*]} " == *" $name "* ]] || pkgs+=("$name")
    done
    if ! out=$(pac -Sp --print-format '%n' "${pkgs[@]}" 2>&1); then
        echo "The $layer package set does not resolve:" >&2
        echo "$out" | grep -iE 'error|conflict|warning' >&2 || echo "$out" >&2
        exit 1
    fi
    echo "$layer: $(echo "$out" | wc -l) packages including dependencies"
}
# shellcheck disable=SC2086 # word splitting of package lists is intended
resolve rootfs $rootfs
# shellcheck disable=SC2086
resolve "rootfs+livefs" $rootfs $livefs
echo "All $(echo "$rootfs $livefs" | wc -w) package names of profile '$profile' ($init) are available."
