#!/usr/bin/env python3
"""Download SegLinux Full as one verified ISO. Requires Python 3.9+, no packages.
Run: python3 download-full.py [--directory PATH]
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
import signal
import urllib.request

API = 'https://api.github.com/repos/SegMind25/SegLinux/releases?per_page=100'
HEADERS = {'User-Agent': 'SegLinux-Full-Downloader', 'Accept': 'application/vnd.github+json'}

def open_url(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers=HEADERS), timeout=60)

def select_image(releases):
    for release in sorted(releases, key=lambda r: r.get('published_at') or '', reverse=True):
        if release.get('draft'):
            continue
        assets = release.get('assets', [])
        groups = {}
        for asset in assets:
            if re.fullmatch(r'(seglinux|wincraft)-[a-z0-9]+-\d{8}-x86_64\.iso(?:\.part\d+)?', asset['name']):
                base = re.sub(r'\.part\d+$', '', asset['name'])
                groups.setdefault(base, []).append(asset)
        for base, parts in groups.items():
            checksum = next((a for a in assets if a['name'] == base + '.sha256'), None)
            if not checksum:
                continue
            parts.sort(key=lambda a: a['name'])
            whole = [p for p in parts if p['name'] == base]
            if whole:
                parts = whole
            elif [int(p['name'].rsplit('part', 1)[1]) for p in parts] != list(range(len(parts))):
                raise ValueError('The release has missing ISO parts.')
            return base, parts, checksum
    raise ValueError('No published Full ISO with a checksum was found.')

def download_image(releases, directory):
    name, parts, checksum = select_image(releases)
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / name
    partial = directory / (name + '.downloading')
    if target.exists() or partial.exists():
        raise FileExistsError(f'{target} or its partial download already exists. Choose another directory or remove the partial file to retry.')
    with open_url(checksum['browser_download_url']) as response:
        expected = response.read(4096).decode().split()[0].lower()
    if not re.fullmatch('[0-9a-f]{64}', expected):
        raise ValueError('Invalid SHA-256 checksum in release.')
    total = sum(p['size'] for p in parts)
    done = 0
    digest = hashlib.sha256()
    print(f'Downloading {name} ({total / 1024**3:.2f} GiB)')
    try:
        with partial.open('xb') as output:
            for part in parts:
                received = 0
                with open_url(part['browser_download_url']) as response:
                    while True:
                        chunk = response.read(1024 * 1024)
                        if not chunk:
                            break
                        output.write(chunk)
                        digest.update(chunk)
                        received += len(chunk)
                        done += len(chunk)
                        print(f'\r{done / total:.1%} · {done / 1024**2:.0f} / {total / 1024**2:.0f} MiB', end='', flush=True)
                if received != part['size']:
                    raise ValueError('Incomplete download. Please retry.')
        if digest.hexdigest() != expected:
            raise ValueError('SHA-256 mismatch. The incomplete ISO has been removed; please retry.')
        # Refuse to replace a file created while downloading.
        if target.exists():
            raise FileExistsError(f'{target} already exists.')
        partial.rename(target)
    except BaseException:
        partial.unlink(missing_ok=True)
        raise
    print(f'\nSHA-256 verified. Ready: {target.resolve()}')
    return target

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', default='.', help='Folder to save the ISO (default: current folder)')
    args = parser.parse_args()
    try:
        with open_url(API) as response:
            releases = json.load(response)
        download_image(releases, args.directory)
    except (OSError, ValueError, KeyError) as error:
        print(f'\nDownload failed: {error}', file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print('\nDownload cancelled; partial file removed.', file=sys.stderr)
        return 130
    return 0

if __name__ == '__main__':
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    sys.exit(main())
