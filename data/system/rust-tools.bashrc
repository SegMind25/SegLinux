# SegLinux: conveniences for the Rust command-line tools, when they are
# installed (ripgrep and fd need none). Aliases only for new names: ls, cd,
# grep and find keep working as everyone expects, in scripts and out.
[[ $- == *i* ]] || return 0
if command -v eza >/dev/null; then
    alias ll='eza -l --group-directories-first'
    alias la='eza -la --group-directories-first'
fi
if command -v zoxide >/dev/null; then
    # `z folder` jumps to the folder you use most that matches.
    eval "$(zoxide init bash)"
fi
