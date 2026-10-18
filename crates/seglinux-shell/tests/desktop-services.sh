#!/bin/bash
# The desktop's session services, end to end, on a private session bus and
# an X server (run under `xvfb-run -a dbus-run-session --`): notifications,
# the system tray and the volume key. Needs the shell and the
# tray-test-item example built (cargo build -p seglinux-shell --examples).
set -euo pipefail

target=${CARGO_TARGET_DIR:-target}/debug
work=$(mktemp -d)
export XDG_CONFIG_HOME=$work/config XDG_STATE_HOME=$work/state
export GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none
# A stand-in for PipeWire's wpctl that records its calls.
mkdir -p "$work/bin"
cat > "$work/bin/wpctl" <<STUB
#!/bin/sh
echo "\$*" >> "$work/wpctl.log"
[ "\$1" = get-volume ] && echo "Volume: 0.55"
exit 0
STUB
chmod +x "$work/bin/wpctl"
export PATH=$work/bin:$PATH

fail() { echo "FAIL: $*" >&2; exit 1; }
# Waits up to 20 s for a command to succeed.
wait_for() {
    for _ in $(seq 40); do
        "$@" > /dev/null 2>&1 && return 0
        sleep 0.5
    done
    return 1
}
N=(gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications)
items() {
    gdbus call --session --dest org.kde.StatusNotifierWatcher --object-path /StatusNotifierWatcher \
        --method org.freedesktop.DBus.Properties.Get org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems
}

"$target/seglinux-shell" --preview > "$work/shell.log" 2>&1 &
shell=$!
trap 'kill $shell 2> /dev/null || true; kill $(jobs -p) 2> /dev/null || true' EXIT
wait_for "${N[@]}" --method org.freedesktop.Notifications.GetServerInformation ||
    fail "no notification service: $(cat "$work/shell.log")"

# Notifications: ids, replacing, closing (with its signal).
gdbus monitor --session --dest org.freedesktop.Notifications > "$work/signals.log" 2>&1 &
sleep 1
id=$("${N[@]}" --method org.freedesktop.Notifications.Notify test 0 dialog-information "Hello" "World" "[]" "{}" 5000)
[[ $id == "(uint32 1,)" ]] || fail "first notification got $id"
id=$("${N[@]}" --method org.freedesktop.Notifications.Notify test 1 dialog-information "Hello again" "" "[]" "{}" 5000)
[[ $id == "(uint32 1,)" ]] || fail "replacing kept no id: $id"
"${N[@]}" --method org.freedesktop.Notifications.CloseNotification 1 > /dev/null
wait_for grep -q "NotificationClosed (uint32 1, uint32 3)" "$work/signals.log" ||
    fail "no NotificationClosed signal: $(cat "$work/signals.log")"
caps=$("${N[@]}" --method org.freedesktop.Notifications.GetCapabilities)
[[ $caps == *actions* && $caps == *body* ]] || fail "capabilities: $caps"
echo "notifications: ok"

# The tray: an item registers, and leaves with its application.
"$target/examples/tray-test-item" > "$work/item.log" 2>&1 &
item=$!
wait_for bash -c "$(declare -f items); items | grep -q NotificationItem/test" ||
    fail "the tray item did not register: $(cat "$work/item.log")"
kill $item
wait_for bash -c "$(declare -f items); items | grep -q '@as \[\]'" ||
    fail "the tray item stayed after its application quit: $(items)"
echo "tray: ok"

# The volume key, as labwc sends it: three quick presses, three steps (one
# after another, none lost).
for _ in 1 2 3; do gapplication action org.seglinux.Shell.Preview volume "'up'"; done
raised() { grep -c "set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+" "$work/wpctl.log" 2> /dev/null; }
wait_for bash -c "[ \"\$(grep -c 'get-volume' '$work/wpctl.log')\" -ge 3 ]" ||
    fail "the volume key did nothing: $(cat "$work/wpctl.log" 2> /dev/null)"
[[ $(raised) == 3 ]] || fail "volume raised $(raised) times for 3 presses"
echo "volume key: ok"

kill -0 $shell 2> /dev/null || fail "the shell stopped: $(cat "$work/shell.log")"
echo "desktop services: all ok"
