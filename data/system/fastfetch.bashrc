# SegLinux: system information with SegLinux's logo (fastfetch) when a
# terminal opens: in the shell a terminal window, tab or console login
# starts, not in shells started from another shell (nor in scripts), nor in
# the terminal panels of Dolphin and Kate. To turn it off:
#   mkdir -p ~/.config/seglinux && touch ~/.config/seglinux/no-fastfetch
if [[ $- == *i* && -t 1 && $TERM != dumb ]] &&
    [[ ! -e ${XDG_CONFIG_HOME:-$HOME/.config}/seglinux/no-fastfetch ]] &&
    command -v fastfetch >/dev/null; then
    { read -r _seglinux_parent < "/proc/$PPID/comm"; } 2>/dev/null || _seglinux_parent=
    case $_seglinux_parent in
        bash | sh | zsh | fish | dash | su | sudo | doas | dolphin | kate) ;;
        *) fastfetch ;;
    esac
    unset _seglinux_parent
fi
