#!/bin/bash
# GitHub Actions: prepares the key SegLinux's updates are signed with, from
# the SEGLINUX_SIGNING_KEY secret (docs/updating.md, "For maintainers").
#
# Imports it into $RUNNER_TEMP/gnupg, writes its public half to
# data/keyrings/ (the seglinux package installs it, so installed systems
# trust what it signs), and sets SEGLINUX_SIGNING_FPR for the next steps.
# Without the secret it warns and sets nothing: nothing is published.
set -euo pipefail

if [[ -z ${SEGLINUX_SIGNING_KEY:-} ]]; then
    echo "::warning title=No SegLinux update published::The SEGLINUX_SIGNING_KEY secret is missing, so installed systems cannot get this version with sudo seglinux-update. Create it once: docs/updating.md, For maintainers."
    exit 0
fi
export GNUPGHOME=${RUNNER_TEMP:-/tmp}/gnupg
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
printf '%s\n' "$SEGLINUX_SIGNING_KEY" | gpg --batch --quiet --import || true
fpr=$(gpg --batch --list-secret-keys --with-colons | awk -F: '/^fpr/ {print $10; exit}')
if [[ -z $fpr ]]; then
    echo "::error::SEGLINUX_SIGNING_KEY holds no secret key: paste the whole seglinux-signing-key.asc, BEGIN and END lines included"
    exit 1
fi
if ! echo test | gpg --batch --pinentry-mode error --local-user "$fpr" --detach-sign > /dev/null 2>&1; then
    echo "::error::The key in SEGLINUX_SIGNING_KEY cannot sign without a passphrase: create it with packaging/make-signing-key.sh"
    exit 1
fi
mkdir -p data/keyrings
gpg --batch --armor --export "$fpr" > data/keyrings/seglinux.gpg
printf '%s:4:\n' "$fpr" > data/keyrings/seglinux-trusted
: > data/keyrings/seglinux-revoked
echo "SEGLINUX_SIGNING_FPR=$fpr" >> "${GITHUB_ENV:-/dev/null}"
echo "SegLinux updates are signed with $fpr"
