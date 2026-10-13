"""Render repository Markdown as static documentation on the website."""
from pathlib import Path
import html
import os
import re
import markdown

LABELS = {
 'downloads': 'Download & install', 'desktop': 'The desktop', 'gaming': 'Gaming',
 'running-windows-apps': 'Windows applications', 'building': 'Build the ISO',
 'unattended-install': 'Unattended installation', 'windows-to-seglinux': 'Moving from Windows',
 'windows-iso-anatomy': 'Inside the installation image',
}

def source_ref():
    """The commit (or branch) being deployed, so links into the repository
    match the documentation; `main` when building outside CI."""
    for name in ('VERCEL_GIT_COMMIT_SHA', 'GITHUB_SHA'):
        if os.environ.get(name):
            return os.environ[name]
    return 'main'


def build_docs(root, output, repo='SegMind25/SegLinux'):
    blob = f'https://github.com/{repo}/blob/{source_ref()}/'
    dest = Path(output) / 'docs'
    dest.mkdir(exist_ok=True)
    for slug, label in LABELS.items():
        source = Path(root) / 'docs' / (slug + '.md')
        content = markdown.markdown(source.read_text(), extensions=['fenced_code', 'tables', 'toc'])
        content = re.sub(r'href="([\w-]+)\.md([^"]*)"', r'href="/docs/\1.html\2"', content)
        content = content.replace('src="images/', 'src="/assets/')
        content = re.sub(r'href="\.\./([^"]+)"', lambda m: f'href="{blob}{m.group(1)}"', content)
        links = ''.join(f'<a href="/docs/{key}.html"'+(' aria-current="page"' if key==slug else '')+f'>{html.escape(value)}</a>' for key,value in LABELS.items())
        page = f'''<!doctype html><html lang="en" data-theme="dark"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{html.escape(label)} — SegLinux Docs</title><link rel="icon" href="/assets/logo.svg"><link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet"><link rel="stylesheet" href="/style.css"><script src="/preferences.js"></script></head><body><header class="nav doc-nav"><a class="nav-logo" href="/" aria-label="SegLinux home"><img src="/assets/logo.svg" width="28" height="28" alt=""></a><div class="nav-links" style="position:static;display:flex;border:0"><a href="/">Home</a><a href="/docs" class="active">Documentation</a></div><div class="nav-right"><a class="nav-cta" href="/download">Download ISO</a></div></header><main class="docs-layout"><aside class="docs-sidebar"><p class="label">THE FIELD GUIDE</p><input class="doc-search" type="search" aria-label="Filter documentation topics" placeholder="Find a guide…"><nav aria-label="Documentation">{links}</nav></aside><article class="docs-content">{content}<p><a href="/docs">← All guides</a> · <a href="/download">Download SegLinux →</a></p></article></main><script>document.querySelector('.doc-search').addEventListener('input',e=>document.querySelectorAll('.docs-sidebar nav a').forEach(a=>a.hidden=!a.textContent.toLowerCase().includes(e.target.value.toLowerCase())))</script></body></html>'''
        (dest / (slug + '.html')).write_text(page)
    (dest / 'index.html').write_text((dest / 'downloads.html').read_text())
