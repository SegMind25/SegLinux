#!/usr/bin/env python3
"""Generate SegLinux's default wallpaper, seglinux-flow.jpg, from scratch.

A rose-to-violet sky over a deep blue floor, a dark arc across the middle
and flowing ribbons of light, with grain so the gradients do not band.
Needs Pillow and NumPy:

    python3 data/artwork/make-wallpaper.py [WIDTH HEIGHT] [OUT]
"""

import sys

import numpy as np
from PIL import Image, ImageFilter

W, H = (int(sys.argv[1]), int(sys.argv[2])) if len(sys.argv) > 2 else (2560, 1440)
OUT = sys.argv[3] if len(sys.argv) > 3 else "data/artwork/seglinux-flow.jpg"
rng = np.random.default_rng(7)

y, x = np.mgrid[0:H, 0:W].astype(np.float32)
u, v = x / W, y / H


def mix(a, b, t):
    t = np.clip(t, 0.0, 1.0)[..., None]
    return np.asarray(a, np.float32) * (1 - t) + np.asarray(b, np.float32) * t


def smooth(t):
    t = np.clip(t, 0.0, 1.0)
    return t * t * (3 - 2 * t)


# Sky: rose at the top right, violet towards the left and the horizon.
rose, violet, plum = (214, 92, 112), (112, 62, 140), (58, 34, 92)
sky = mix(rose, violet, smooth(v * 1.6 + (1 - u) * 0.35))
sky = mix(sky, plum, smooth((v - 0.25) * 2.4))

# Floor: deep blue rising to a brighter blue in the centre.
deep, bright = (24, 40, 128), (70, 110, 226)
glow = np.exp(-(((u - 0.5) / 0.32) ** 2) - (((v - 0.86) / 0.22) ** 2))
floor = mix(deep, bright, glow)
horizon = 0.46 + 0.10 * (u - 0.5) ** 2
img = mix(sky, floor, smooth((v - horizon) / 0.16))

# The dark arc across the sky, like a lens over the scene.
arc = np.hypot((u - 0.5) * 1.15, (v - 1.18) * 1.0)
band = np.exp(-(((arc - 0.92) / 0.045) ** 2))
img = mix(img, (28, 22, 44), band * 0.75)
rim = np.exp(-(((arc - 0.875) / 0.010) ** 2))
img = mix(img, (255, 170, 190), rim * 0.25)

# Ribbons: smooth waves, shaded across their width so they look rounded.
ribbons = np.zeros((H, W), np.float32)
light = np.zeros((H, W), np.float32)
for i in range(7):
    base = 0.55 + i * 0.045
    amp = 0.035 + 0.012 * i
    freq = 2.2 + 0.35 * i
    phase = rng.uniform(0, 2 * np.pi)
    width = 0.016 + 0.007 * (i % 3)
    centre = base + amp * np.sin(freq * np.pi * u + phase) * (0.6 + 0.4 * np.cos(np.pi * u))
    d = (v - centre) / width
    body = np.exp(-(d**2) * 1.6) * smooth((u - 0.05) * 3) * smooth((0.97 - u) * 3)
    ribbons = np.maximum(ribbons, body)
    light = np.maximum(light, body * np.clip(1 - np.abs(d + 0.35), 0, 1))
ribbon_col = mix((96, 130, 236), (196, 210, 255), light)
img = mix(img, ribbon_col, ribbons * 0.85)

# Soft bloom of the ribbons, then grain against banding.
bloom = Image.fromarray((ribbons * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(W / 90))
bloom = np.asarray(bloom, np.float32) / 255
img = mix(img, (150, 170, 255), bloom * 0.35)
img += rng.normal(0, 2.2, img.shape).astype(np.float32)

# A gentle vignette keeps the edges calm for icons and the panel.
vignette = smooth(np.hypot(u - 0.5, (v - 0.5) * 1.2) * 1.5 - 0.35)
img = mix(img, (12, 10, 26), vignette * 0.45)

Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).save(OUT, quality=90, optimize=True)
print(f"{OUT}: {W}x{H}")
