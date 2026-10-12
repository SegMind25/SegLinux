#
# ~/.bash_profile (Wincraft Lite live session)
#
# shellcheck source=/dev/null
[[ -f ~/.bashrc ]] && . ~/.bashrc

# The Lite edition has no display manager: the live user is signed in on
# tty1, which starts the Wincraft desktop. If the desktop stops (or cannot
# start), this console stays open instead of signing in again in a loop.
if [[ -z $WAYLAND_DISPLAY && -z $DISPLAY && $(tty) == /dev/tty1 ]]; then
    log=~/.cache/wincraft-session.log
    mkdir -p ~/.cache
    wincraft-session >"$log" 2>&1
    # The compositor can leave the console in raw mode.
    stty sane 2>/dev/null
    echo "The Wincraft desktop has stopped. Its last messages ($log):"
    tail -n 15 "$log"
    echo
    echo "Type wincraft-session to start it again."
fi
