#!/bin/bash
# Creates the key SegLinux's updates are signed with, for the repository's
# SEGLINUX_SIGNING_KEY secret (docs/updating.md, "For maintainers"). Run it
# once, as yourself (not root), on any Linux with gpg, SegLinux included:
#
#   curl -fsSL https://raw.githubusercontent.com/SegMind25/SegLinux/main/packaging/make-signing-key.sh | bash
#
# It writes ~/seglinux-signing-key.asc (or the file given as argument) and
# opens GitHub's page for the secret. Keep that file safe, and keep using
# the same key: installed systems trust only it.
set -euo pipefail

out=${1:-$HOME/seglinux-signing-key.asc}
secret_page=https://github.com/SegMind25/SegLinux/settings/secrets/actions/new

if [[ -e $out ]]; then
    echo "$out already exists. Add it as the secret (below), or move it away to make a new key." >&2
else
    command -v gpg >/dev/null || { echo "gpg is needed: sudo pacman -S gnupg" >&2; exit 1; }
    work=$(mktemp -d)
    trap 'gpgconf --homedir "$work" --kill all 2>/dev/null || true; rm -rf "$work"' EXIT
    # No passphrase: GitHub Actions signs unattended; the secret store
    # protects the key.
    GNUPGHOME=$work gpg --batch --quiet --passphrase '' \
        --quick-gen-key "SegLinux Package Signing" ed25519 sign never
    (umask 077 && GNUPGHOME=$work gpg --batch --armor --export-secret-keys > "$out")
    echo "Created $out"
fi

cat <<EOF

Now, once:
  1. Open  $secret_page
     (GitHub, the SegLinux repository: Settings > Secrets and variables >
     Actions > New repository secret).
  2. Name:   SEGLINUX_SIGNING_KEY
     Secret: the whole content of $out, from
             -----BEGIN PGP PRIVATE KEY BLOCK----- to the END line.
     (Show it with:  cat $out)
  3. Add secret. Then Actions > SegLinux update > Run workflow publishes the
     first update; after that every change to main is published by itself.

Keep $out somewhere safe and offline; do not make a new key later.
EOF
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
    read -r -p "The GitHub CLI is signed in: add the secret with it now? [Y/n] " answer < /dev/tty || answer=n
    if [[ $answer != [nN]* ]]; then
        gh secret set SEGLINUX_SIGNING_KEY --repo SegMind25/SegLinux < "$out"
        echo "Secret added."
        exit 0
    fi
fi
command -v xdg-open >/dev/null && xdg-open "$secret_page" >/dev/null 2>&1 &
exit 0
