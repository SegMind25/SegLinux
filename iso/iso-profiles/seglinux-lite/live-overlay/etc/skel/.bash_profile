#
# ~/.bash_profile (SegLinux Lite live session)
#
# shellcheck source=/dev/null
[[ -f ~/.bashrc ]] && . ~/.bashrc

# The Lite edition has no display manager: the live user is signed in on
# tty1, which starts the SegLinux desktop. If the desktop stops (or cannot
# start), this console stays open instead of signing in again in a loop.
if [[ -z $WAYLAND_DISPLAY && -z $DISPLAY && $(tty) == /dev/tty1 ]]; then
    log=~/.cache/seglinux-session.log
    mkdir -p ~/.cache
    seglinux-session >"$log" 2>&1
    # The compositor can leave the console in raw mode.
    stty sane 2>/dev/null
    echo "The SegLinux desktop has stopped. Its last messages ($log):"
    tail -n 15 "$log"
    echo
    echo "Type seglinux-session to start it again."
fi
