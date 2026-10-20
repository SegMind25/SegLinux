#!/usr/bin/env bash
# Check that every package the ISO profile names exists in the repositories
# buildiso will install from, and that each layer's package set resolves.
# Run on Artix (or in the build container); needs pacman and go-yq.
#
#   iso/scripts/check-packages.sh [-i openrc|runit|dinit|s6] [-k kernel]
#                                 [-p profile] [-c pacman.conf] [-x file]
#
# -x names a file of packages that tools install later (one per line, `#`
# comments): they must exist and resolve together with the root file system.
set -euo pipefail

iso_dir=$(cd "$(dirname "$0")/.." && pwd)
init=openrc
kernel=linux
profile=seglinux
conf=$iso_dir/pacman.conf.d/iso-x86_64.conf
local_repo=/var/lib/seglinux-repo
extra_file=

while getopts "i:k:p:c:x:h" opt; do
    case $opt in
        i) init=$OPTARG ;;
        k) kernel=$OPTARG ;;
        p) profile=$OPTARG ;;
        c) conf=$OPTARG ;;
        x) extra_file=$OPTARG ;;
        *) sed -n '2,12s/^# \{0,1\}//p' "$0"; exit 2 ;;
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

extra=
if [[ -n $extra_file ]]; then
    [[ -f $extra_file ]] || { echo "no package list at $extra_file" >&2; exit 1; }
    extra=$(grep -v '^[[:space:]]*#' "$extra_file" | sed -e "s/^linux-headers$/$kernel-headers/" | sort -u)
fi

skip=()
if [[ ! -f $local_repo/seglinux.db ]]; then
    # The seglinux and segctl packages are built during the ISO build; when
    # checking before that, leave the local repository out.
    awk '/^\[seglinux\]/{skip=1; next} /^\[/{skip=0} !skip' "$conf" > "$work_conf"
    skip=(seglinux segctl)
    echo "note: $local_repo has no repository yet; not checking: ${skip[*]}"
fi

pac() { pacman --config "$work_conf" --dbpath "$db" --noconfirm "$@"; }
pac -Sy >/dev/null

mapfile -t available < <(pac -Slq)
declare -A have=()
for name in "${available[@]}"; do have[$name]=1; done

missing=()
for name in $rootfs $livefs $extra; do
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
if [[ -n $extra ]]; then
    # shellcheck disable=SC2086
    resolve "rootfs+tools ($extra_file)" $rootfs $extra
fi
echo "All $(echo "$rootfs $livefs $extra" | wc -w) package names of profile '$profile' ($init) are available."
