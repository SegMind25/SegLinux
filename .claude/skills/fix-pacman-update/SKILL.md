---
name: fix-pacman-update
description: Fix a SegLinux/Artix update that fails or stops with a question - "failed to synchronize all databases", "<repo>.db failed to download", a slow or huge repository download, or yay asking "There are N providers available for <pkg>". Use when the user shares a pacman/yay/seglinux-upgrade error (often as a photo) and wants it fixed on their computer and in the next update.
---

# Fixing a failed SegLinux update

The fix has two parts: the commands the user runs now, and a change to
`data/system/seglinux-upgrade` so every installed system gets it with the next
`sudo seglinux-upgrade`. Keep the reply short; the user is often on a phone.

## Read the error

| Error | Cause | Fix now | Fix in the update |
|---|---|---|---|
| `failed retrieving file '<repo>.db'`, `failed to synchronize all databases` | a repository in `/etc/pacman.conf` is gone (often one the user added) | `sudo nvim /etc/pacman.conf`, put `#` before its `[repo]` and `Server` lines | `disable_dead_repos` already does this; extend it if the repo uses `Include` |
| a repository is slow or huge on every update | a big third-party repo (Chaotic-AUR's list is several MB) | remove it from `pacman.conf` | retire it like `retire_chaotic`; serve what it provided from the AUR (`aur_install`) |
| `There are N providers available for X` (yay) | an AUR package the user installed depends on X | press Enter (1 is fine), or find it with `yay -Qua` and remove it: `sudo pacman -Rns <pkg>` | nothing: SegLinux does not install it |
| `exists in filesystem` | a hand-made file where a package puts one | move the file away | nothing |
| `db.lck` | another pacman is running, or a stale lock | wait, or `sudo rm /var/lib/pacman/db.lck` | already handled |

## Changing seglinux-upgrade

- It is `bash` with `set -euo pipefail`; back up `pacman.conf` to
  `$conf.before-seglinux-update` before editing it, and keep every change
  idempotent (a second run changes nothing).
- AUR builds run as `$SUDO_USER` (makepkg refuses root), never as root.
- Test the `pacman.conf` edits on a copy: `source data/system/seglinux-upgrade`,
  set `conf=<copy>`, stub `pacman() { :; }`, run the function twice.
- Update `docs/updating.md` (Troubleshooting), `CHANGELOG.md`, and the
  `update-command` job in `.github/workflows/ci.yml` and
  `.github/workflows/upgrade-test.yml` when what they check changes.
