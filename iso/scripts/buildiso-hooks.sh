# shellcheck shell=bash
# Sourced by SegLinux's copy of artools' buildiso once artools' libraries
# are loaded (see build-in-container.sh). It replaces the Artix name and
# artwork that artools puts on the install media with SegLinux's: the boot
# menu theme, the boot menu entries and the image's metadata. Artix's
# packages and live scripts stay as they are.

seglinux_theme=/usr/share/grub/themes/seglinux

# Read by artools for the ISO's application and publisher fields.
get_osname() { printf '%s\n' SegLinux; }
get_disturl() { printf '%s\n' https://github.com/SegMind25/SegLinux; }

# The image layers to look in: the ones artools passes, else its work folders.
seglinux_layers() {
    local layer
    for layer in "$@" "${work_dir:-}/rootfs" "${work_dir:-}/livefs"; do
        [[ -n $layer && $layer != / && -d $layer ]] && printf '%s\n' "$layer"
    done
}

# The SegLinux boot theme in the image, from the seglinux package.
seglinux_theme_source() {
    local layer
    while read -r layer; do
        if [[ -d $layer$seglinux_theme ]]; then
            printf '%s\n' "$layer$seglinux_theme"
            return
        fi
    done < <(seglinux_layers "$@")
}

# artools' menu rests on its third entry (the keyboard layout) and waits
# forever (timeout=-1): Enter opened the keyboard list instead of starting
# the system. Start SegLinux is the default, after a 10 second countdown.
seglinux_default_entry() {
    local grub=$1
    if [[ -f $grub/kernels.cfg ]] && ! grep -q '^default="Start SegLinux' "$grub/kernels.cfg"; then
        printf '\n# SegLinux: start the live system by default (after the menu sets default=2).\ndefault="Start SegLinux (DVD or ISO image)"\n' >> "$grub/kernels.cfg"
    fi
    if [[ -f $grub/variable.cfg ]]; then
        sed -i 's/^timeout=-1$/timeout=10/' "$grub/variable.cfg"
    fi
}

# Rename what the boot menu shows; kernel arguments and paths stay.
seglinux_brand_grub() {
    # shellcheck disable=SC2154 # iso_root is set by buildiso
    local grub=$iso_root/boot/grub src cfgs
    src=$(seglinux_theme_source "$@")
    if [[ -z $src ]]; then
        echo "seglinux: the SegLinux boot theme is missing from the image; the boot menu keeps artools' theme" >&2
        return 0
    fi
    cfgs=("$grub"/*.cfg)
    rm -rf "$grub/themes/artix"
    cp -r "$src" "$grub/themes/seglinux"
    sed -i 's|themes/artix/|themes/seglinux/|g; s/ARTIX_EFI/SEGLINUX/g' "${cfgs[@]}"
    if ! grep -q 'themes/seglinux/' "${cfgs[@]}"; then
        # The menu names its theme some other way: put ours where it looks.
        cp -r "$src" "$grub/themes/artix"
    fi
    perl -pi -e '
        next unless /^\s*(?:menuentry|submenu|menu)\b/;
        s/--class[= ]artix(?:\.x86_64)?\b/--class=seglinux/g;
        s{^([^"]*")([^"]*)"}{
            my ($head, $title) = ($1, $2);
            $title =~ s#From CD/DVD/ISO: artix\.x86_64\s*#Start SegLinux (DVD or ISO image)#;
            $title =~ s#From Stick/HDD: artix\.x86_64\s*#Start SegLinux (USB stick or hard disk)#;
            $title =~ s#[Aa]rtix(?:\.x86_64)?(?: Linux)?#SegLinux#g;
            "$head$title\""
        }e;
    ' "${cfgs[@]}"
    seglinux_default_entry "$grub"
    echo "seglinux: boot menu entries:"
    grep -ho '^[[:space:]]*\(menuentry\|submenu\|menu\) [^{]*SegLinux[^{]*' "${cfgs[@]}" | sed 's/^[[:space:]]*/    /'
    if grep -n -i 'artix' "${cfgs[@]}" | grep -v -i 'artix\(label\|basedir\)\|/artix\b' | grep -q .; then
        echo "seglinux: boot menu lines that still name Artix (internal names are expected):"
        grep -n -i 'artix' "${cfgs[@]}" | sed 's/^/    /' | head -20
    fi
}

if declare -F prepare_grub >/dev/null; then
    eval "seglinux_artools_$(declare -f prepare_grub)"
    # artools copies the theme from usr/share/grub/themes/artix, which the
    # images do not have (they carry the SegLinux theme instead): lend it ours
    # for the copy, then brand the result.
    prepare_grub() {
        local src layer themes lent=() made=() status=0
        src=$(seglinux_theme_source "$@")
        while read -r layer; do
            themes=$layer/usr/share/grub/themes
            [[ -n $src && ! -e $themes/artix ]] || continue
            if [[ ! -d $themes ]]; then
                mkdir -p "$themes"
                made+=("$themes")
            fi
            cp -r "$src" "$themes/artix"
            lent+=("$themes/artix")
        done < <(seglinux_layers "$@")
        seglinux_artools_prepare_grub "$@" || status=$?
        if ((${#lent[@]})); then
            rm -rf "${lent[@]}"
        fi
        for themes in "${made[@]}"; do
            rmdir -p --ignore-fail-on-non-empty "$themes"
        done
        ((status == 0)) || return "$status"
        seglinux_brand_grub "$@"
    }
else
    echo "seglinux: artools has no prepare_grub; the boot menu keeps artools' branding" >&2
fi
