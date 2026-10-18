#!/bin/bash
# GitHub Actions: publishes the signed seglinux package as the update that
# installed systems get with `sudo seglinux-upgrade` (docs/updating.md).
#
#   iso/scripts/publish-update.sh DIR    (DIR holds seglinux-*.pkg.tar.zst)
#
# Needs the key iso/scripts/signing-key.sh imported (SEGLINUX_SIGNING_FPR),
# GH_TOKEN and GITHUB_REPOSITORY. The repository is the release "packages":
# the package and its signature, the pacman database naming it, and the
# public key, at https://github.com/<repo>/releases/download/packages/.
set -euo pipefail

dir=$1
tag=packages
export GNUPGHOME=${RUNNER_TEMP:-/tmp}/gnupg
: "${SEGLINUX_SIGNING_FPR:?run iso/scripts/signing-key.sh first}" "${GITHUB_REPOSITORY:?}"
cd "$dir"
pkg=$(ls seglinux-*.pkg.tar.zst)
[[ $(wc -l <<< "$pkg") == 1 ]] || { echo "expected one package in $dir, found: $pkg" >&2; exit 1; }
gpg --batch --yes --local-user "$SEGLINUX_SIGNING_FPR" --detach-sign --output "$pkg.sig" "$pkg"
gpg --batch --verify "$pkg.sig" "$pkg"
gpg --batch --armor --export "$SEGLINUX_SIGNING_FPR" > seglinux.gpg

# repo-add (pacman's) writes the database, with the package's signature in it.
rm -f seglinux.db* seglinux.files*
docker run --rm -v "$PWD:/repo" -w /repo docker.io/artixlinux/artixlinux:base \
    repo-add seglinux.db.tar.gz "$pkg"
sudo chown -R "$(id -u):$(id -g)" .
# Release assets cannot be links: upload the databases as plain files.
cp -L seglinux.db.tar.gz seglinux.db.upload && mv -f seglinux.db.upload seglinux.db
cp -L seglinux.files.tar.gz seglinux.files.upload && mv -f seglinux.files.upload seglinux.files

if ! gh release view "$tag" --repo "$GITHUB_REPOSITORY" > /dev/null 2>&1; then
    gh release create "$tag" --repo "$GITHUB_REPOSITORY" --target "${GITHUB_SHA:-main}" --prerelease \
        --title "SegLinux updates" \
        --notes "The package repository installed SegLinux systems update from: run \`sudo seglinux-upgrade\` (docs/updating.md). Not an ISO."
fi
# The package first, then the database that names it; then the old
# versions go (pacman only fetches what the database names).
gh release upload "$tag" "$pkg" "$pkg.sig" --repo "$GITHUB_REPOSITORY" --clobber
gh release upload "$tag" seglinux.db seglinux.files seglinux.gpg --repo "$GITHUB_REPOSITORY" --clobber
old=$(gh release view "$tag" --repo "$GITHUB_REPOSITORY" --json assets --jq '.assets[].name' |
    grep -E '^seglinux-.*\.pkg\.tar\.zst(\.sig)?$' | grep -vxF -e "$pkg" -e "$pkg.sig" || true)
for name in $old; do
    gh release delete-asset "$tag" "$name" --repo "$GITHUB_REPOSITORY" --yes
done
echo "::notice title=SegLinux update published::$pkg: installed systems get it with sudo seglinux-upgrade"
