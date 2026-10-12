#
# ~/.bash_profile (Wincraft Lite live session)
#
# shellcheck source=/dev/null
[[ -f ~/.bashrc ]] && . ~/.bashrc

# The Lite edition has no display manager: the live user is signed in on
# tty1, which starts the Wincraft desktop. If the desktop stops (or cannot
# start), this console stays open instead of signing in again in a loop.
if [[ -z $WAYLAND_DISPLAY && -z $DISPLAY && $(tty) == /dev/tty1 ]]; then
    wincraft-session
    echo "The Wincraft desktop has stopped. Type wincraft-session to start it again."
fi
