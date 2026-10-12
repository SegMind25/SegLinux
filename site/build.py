#!/usr/bin/env python3
"""Assemble the SegLinux website in one folder, ready for GitHub Pages.

Copies the pages in site/, the logo and the screenshots (docs/images), and
writes releases.json: the ISO releases of the repository, from GitHub's API
(with GH_TOKEN or GITHUB_TOKEN when set; needed for a private repository),
or from a file given with --releases.

    python3 site/build.py [--out _site] [--repo OWNER/NAME] [--releases FILE]
"""

import argparse
import json
import os
import shutil
import sys
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
PAGES = ("index.html", "style.css", "app.js")
# The fields of a release the page uses (the same as in GitHub's API).
RELEASE_FIELDS = ("tag_name", "name", "html_url", "published_at", "created_at", "prerelease", "draft")
ASSET_FIELDS = ("name", "size", "browser_download_url")


def fetch_releases(repo):
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    releases, page = [], 1
    while True:
        request = urllib.request.Request(
            f"https://api.github.com/repos/{repo}/releases?per_page=100&page={page}",
            headers={"Accept": "application/vnd.github+json", "User-Agent": "seglinux-site"},
        )
        if token:
            request.add_header("Authorization", f"Bearer {token}")
        with urllib.request.urlopen(request, timeout=30) as response:
            batch = json.load(response)
        releases += batch
        if len(batch) < 100:
            return releases
        page += 1


def slim(release):
    out = {key: release.get(key) for key in RELEASE_FIELDS}
    # The notes name the init system and kernel of version releases.
    out["body"] = (release.get("body") or "")[:400]
    out["assets"] = [{key: asset.get(key) for key in ASSET_FIELDS} for asset in release.get("assets", [])]
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", default=os.path.join(ROOT, "_site"))
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "SegMind25/SegLinux"))
    parser.add_argument("--releases", help="read the releases from this JSON file instead of GitHub")
    args = parser.parse_args()

    if os.path.isdir(args.out):
        shutil.rmtree(args.out)
    assets = os.path.join(args.out, "assets")
    os.makedirs(assets)
    for name in PAGES:
        shutil.copy2(os.path.join(HERE, name), args.out)
    shutil.copy2(os.path.join(ROOT, "data", "artwork", "seglinux.svg"), os.path.join(assets, "logo.svg"))
    images = os.path.join(ROOT, "docs", "images")
    for name in sorted(os.listdir(images)):
        if name.endswith(".webp"):
            shutil.copy2(os.path.join(images, name), assets)

    if args.releases:
        with open(args.releases) as f:
            releases = json.load(f)
    else:
        try:
            releases = fetch_releases(args.repo)
        except OSError as error:
            # The page falls back to GitHub's API in the browser.
            print(f"warning: cannot list the releases of {args.repo}: {error}", file=sys.stderr)
            releases = []
    isos = [
        slim(r)
        for r in releases
        if not r.get("draft") and any(a.get("name", "").endswith(".sha256") for a in r.get("assets", []))
    ]
    with open(os.path.join(args.out, "releases.json"), "w") as f:
        json.dump(isos, f, indent=1)
    # GitHub Pages: serve the files as they are.
    open(os.path.join(args.out, ".nojekyll"), "w").close()
    print(f"{args.out}: {len(isos)} releases with ISO images, {len(os.listdir(assets))} assets")


if __name__ == "__main__":
    main()
