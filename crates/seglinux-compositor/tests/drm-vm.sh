#!/bin/bash
# seglinux-compositor on a real DRM device, as a sign-in starts it
# (--tty: libseat through seatd, KMS, GBM, EGL): a virtual machine boots
# this system's own files (virtme-ng) with vkms, the kernel's virtual
# display driver. A wallpaper, a Wayland and an X11 terminal open; the
# screen is listed, turned off and on, and photographed.
#
#   crates/seglinux-compositor/tests/drm-vm.sh OUTPUT_DIR [COMPOSITOR]
#
# Needs: qemu-system-x86, a distribution kernel with its modules
# (linux-image-virtual linux-modules-extra-*), virtme-ng (pip), busybox-static,
# zstd, seatd, foot, swaybg, grim, wlr-randr, wlopm, xwayland, xterm.
# Uses KVM when there is one; without it the run takes a few minutes.
set -euo pipefail

out=$(realpath -m "${1:?usage: drm-vm.sh OUTPUT_DIR [COMPOSITOR]}")
compositor=$(realpath "${2:-target/debug/seglinux-compositor}")
wallpaper=$(realpath "$(dirname "$0")/../../../data/artwork/seglinux-dark.jpg")
# KERNEL: a kernel with vkms among its modules; else the newest in /boot.
kernel=${KERNEL:-$(find /boot -maxdepth 1 -name 'vmlinuz-*' | sort -V | tail -n 1)}
[[ -n $kernel ]] || { echo "no kernel in /boot" >&2; exit 1; }
mkdir -p "$out"
rm -f "$out"/*
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cat > "$work/inside.sh" <<EOF
#!/bin/sh
exec > "$out/inside.log" 2>&1
set -x
modprobe vkms
sleep 2
ls -l /dev/dri
export XDG_RUNTIME_DIR=/tmp/xdg-vm XDG_CONFIG_HOME=/tmp/vmcfg RUST_LOG=info
mkdir -p -m 700 "\$XDG_RUNTIME_DIR" /tmp/vmcfg/seglinux
cat > /tmp/vmcfg/seglinux/compositor.toml <<'TOML'
autostart = ["swaybg -i $wallpaper -m fill", "foot", "sh $work/probe.sh"]
TOML
timeout 300 seatd-launch -- "$compositor" --tty > "$out/compositor.log" 2>&1
echo "compositor exit \$?"
EOF

cat > "$work/probe.sh" <<EOF
#!/bin/sh
exec > "$out/probe.log" 2>&1
# Until the terminal is up (slow without KVM).
for _ in \$(seq 120); do
    [ "\$(wlr-randr 2>/dev/null | grep -c Enabled)" -ge 1 ] && sleep 20 && break
    sleep 1
done
wlr-randr && echo randr-ok
wlopm --off '*' && sleep 2 && wlopm | grep -q ' off' && echo power-off-ok
wlopm --on '*' && sleep 2 && wlopm | grep -q ' on' && echo power-on-ok
xterm -geometry 60x10 -title X11 &
sleep 20
grim "$out/screen.png" && echo grim-ok
grim -g "0,0 400x100" "$out/corner.png" && echo region-ok
pkill -f seatd-launch
EOF
chmod +x "$work"/*.sh

kvm=()
[[ -w /dev/kvm ]] || kvm=(--disable-kvm)
vng -r "$kernel" "${kvm[@]}" --force-9p --memory 3G --cpus 4 \
    --rwdir="$out" --rodir="$work" -- sh "$work/inside.sh" > "$out/vm.log" 2>&1 || true

grep -- '-ok' "$out/probe.log" 2>/dev/null || true
failed=0
for check in randr-ok power-off-ok power-on-ok grim-ok region-ok; do
    grep -qx "$check" "$out/probe.log" 2>/dev/null || { echo "missing: $check" >&2; failed=1; }
done
grep -q 'screen .* at ' "$out/compositor.log" 2>/dev/null || { echo "no screen was set up" >&2; failed=1; }
if ((failed)); then
    echo "--- inside.log"; tail -n 20 "$out/inside.log" 2>/dev/null || true
    echo "--- compositor.log"; grep -iE 'warn|error|panic' "$out/compositor.log" | tail -n 20 || true
    echo "--- vm.log"; tail -n 20 "$out/vm.log" || true
    exit 1
fi
echo "seglinux-compositor ran on a DRM device: $out/screen.png"
