#!/usr/bin/env python3
"""Draw SegLinux's brand images from scratch: the dark wallpaper and the
GRUB boot menu theme (background, logo icon, highlight and menu frames,
fonts).

The mark is the S of seglinux.svg: two arcs of radius 19 in a 128-unit
box, a 13-unit stroke and a diagonal cut that splits it into two
segments. It is drawn here as a distance field so that every image uses
the same geometry. Needs Pillow and NumPy; the fonts need grub-mkfont and
the Inter font:

    python3 data/artwork/make-brand.py
"""

import os
import shutil
import subprocess
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.dirname(HERE)
GRUB = os.path.join(DATA, "grub", "seglinux")
rng = np.random.default_rng(11)

# The mark, in logo units (see seglinux.svg).
R = 19.0
UPPER = (64.0, 45.0, 90.0, 242.0)  # centre x, y, start angle, span (degrees)
LOWER = (64.0, 83.0, 270.0, 242.0)
STROKE = 13.0
CUT = ((40.0, 76.0), (88.0, 52.0), 5.0)  # from, to, width
CYAN, BLUE, VIOLET = (142, 223, 255), (61, 174, 233), (109, 107, 255)


def mix(a, b, t):
    t = np.clip(t, 0.0, 1.0)[..., None]
    return np.asarray(a, np.float32) * (1 - t) + np.asarray(b, np.float32) * t


def smooth(t):
    t = np.clip(t, 0.0, 1.0)
    return t * t * (3 - 2 * t)


def arc(px, py, arc_, r):
    """Distance to an arc, and how far along it the nearest point is (0..1)."""
    cx, cy, start, span = arc_
    dx, dy = px - cx, py - cy
    theta = np.degrees(np.arctan2(dy, dx))
    along = np.mod(theta - start, 360.0)
    inside = along <= span
    ring = np.abs(np.hypot(dx, dy) - r)
    ends = []
    for a in (start, start + span):
        ex, ey = cx + r * np.cos(np.radians(a)), cy + r * np.sin(np.radians(a))
        ends.append(np.hypot(px - ex, py - ey))
    d = np.where(inside, ring, np.minimum(*ends))
    t = np.where(inside, along / span, np.where(ends[0] < ends[1], 0.0, 1.0))
    return d, t


def mark(px, py, offset=0.0):
    """Distance to the S (or a ribbon beside it) and the position along it:
    0 at the top end, 1 at the bottom end."""
    du, tu = arc(px, py, UPPER, R + offset)
    dl, tl = arc(px, py, LOWER, R - offset)
    # The upper arc is drawn from its bottom (start) to its top end.
    return np.minimum(du, dl), np.where(du < dl, (1 - tu) / 2, 0.5 + tl / 2)


def cut(px, py):
    (x0, y0), (x1, y1), width = CUT
    vx, vy = x1 - x0, y1 - y0
    t = np.clip(((px - x0) * vx + (py - y0) * vy) / (vx * vx + vy * vy), 0, 1)
    return np.hypot(px - (x0 + t * vx), py - (y0 + t * vy)) - width / 2


def gradient(px, py):
    """The stroke colours: cyan at the top left to violet at the bottom right."""
    t = np.clip(((px - 40) * 48 + (py - 24) * 80) / (48 * 48 + 80 * 80), 0, 1)
    return np.where(
        (t < 0.55)[..., None],
        mix(CYAN, BLUE, t / 0.55),
        mix(BLUE, VIOLET, (t - 0.55) / 0.45),
    )


def logo(size, background=True):
    """The logo as an RGBA image, like seglinux.svg rendered at `size`."""
    scale = 4
    n = size * scale
    y, x = np.mgrid[0:n, 0:n].astype(np.float32)
    px, py = (x + 0.5) * 128 / n, (y + 0.5) * 128 / n
    px_unit = 128 / n
    rgba = np.zeros((n, n, 4), np.float32)
    if background:
        # Rounded square, 120 units with a 30-unit radius.
        qx = np.abs(px - 64) - 60 + 30
        qy = np.abs(py - 64) - 60 + 30
        box = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - 30
        cover = np.clip(0.5 - box / px_unit, 0, 1)
        rgba[..., :3] = mix((22, 36, 61), (12, 20, 36), py / 128)
        rgba[..., 3] = cover
    d, _ = mark(px, py)
    s = np.clip(0.5 - (d - STROKE / 2) / px_unit, 0, 1) * np.clip(0.5 + cut(px, py) / px_unit, 0, 1)
    rgba[..., :3] = mix(rgba[..., :3], gradient(px, py), s)
    rgba[..., 3] = np.maximum(rgba[..., 3], s)
    img = Image.fromarray((rgba * [1, 1, 1, 255]).clip(0, 255).astype(np.uint8), "RGBA")
    return img.resize((size, size), Image.LANCZOS)


def dark_wallpaper(w=2560, h=1440, out=os.path.join(HERE, "seglinux-dark.jpg")):
    """A deep navy scene with the S drawn in ribbons of light on the right;
    the left half stays calm for desktop icons."""
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    u, v = x / w, y / h
    img = mix((14, 20, 40), (4, 6, 14), smooth(np.hypot(u - 0.62, (v - 0.45) * 1.3) * 1.4))
    # The mark: 1000 px tall, its centre at 68 % across.
    scale = h * 0.70 / 89
    px = (x - w * 0.68) / scale + 64
    py = (y - h * 0.47) / scale + 64
    gap = smooth(cut(px, py) / 1.2)
    body, along = mark(px, py)
    taper = smooth(along / 0.08) * smooth((1 - along) / 0.08)
    shape = smooth((STROKE / 2 - body) / 0.6) * gap * taper
    colour = gradient(px, py)
    # Glass body: faint inside, brighter towards the edges.
    edge = np.exp(-(((body - STROKE / 2) / 0.9) ** 2)) * gap * taper
    img = mix(img, colour * 0.55, shape * 0.30)
    # Ribbons of light running along the stroke.
    light = np.zeros((h, w), np.float32)
    for i, off in enumerate(np.linspace(-5.2, 5.2, 9)):
        d, along = mark(px, py, off)
        width = 0.20 + 0.10 * (i % 3)
        fade = smooth(along / 0.12) * smooth((1 - along) / 0.12)
        light = np.maximum(light, np.exp(-((d / width) ** 2)) * fade * (0.55 + 0.45 * np.cos(i * 1.3) ** 2))
    light = np.maximum(light * gap, edge * 0.8)
    img = mix(img, mix(colour, (235, 245, 255), light * 0.5), light)
    bloom = Image.fromarray((np.clip(light, 0, 1) * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(w / 70))
    bloom = np.asarray(bloom, np.float32) / 255
    img = img + mix(np.zeros(3), (90, 120, 255), bloom) * 0.9
    glow = Image.fromarray((shape * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(w / 25))
    img = img + mix(np.zeros(3), (70, 60, 200), np.asarray(glow, np.float32) / 255) * 0.35
    # A floor reflection: the bottom glows faintly under the mark.
    floor = np.exp(-(((u - 0.68) / 0.2) ** 2) - (((v - 1.0) / 0.12) ** 2))
    img = img + mix(np.zeros(3), (40, 70, 160), floor) * 0.5
    img += rng.normal(0, 2.0, img.shape).astype(np.float32)
    Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).save(out, quality=90, optimize=True)
    print(f"{out}: {w}x{h}")


def font(size, weight="SemiBold"):
    for path in (
        f"/usr/share/fonts/opentype/inter/Inter-{weight}.otf",
        f"/usr/share/fonts/OTF/Inter-{weight}.otf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ):
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    sys.exit("no Inter or DejaVu font found")


def grub_background(w=1920, h=1080):
    """Boot menu background: dark, with the logo and name at the top."""
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    u, v = x / w, y / h
    img = mix((16, 24, 46), (5, 8, 18), smooth(np.hypot(u - 0.5, (v - 0.3) * 1.4) * 1.3))
    img = img + mix(np.zeros(3), (40, 50, 140), np.exp(-(((u - 0.5) / 0.25) ** 2) - (((v - 0.17) / 0.12) ** 2))) * 0.5
    img += rng.normal(0, 1.6, img.shape).astype(np.float32)
    bg = Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).convert("RGBA")
    mark_img = logo(120)
    name = "SegLinux"
    title = font(64)
    draw = ImageDraw.Draw(bg)
    tw = draw.textlength(name, font=title)
    gap = 28
    left = int((w - (120 + gap + tw)) / 2)
    top = int(h * 0.11)
    bg.alpha_composite(mark_img, (left, top))
    draw.text((left + 120 + gap, top + 60), name, font=title, fill=(240, 246, 255), anchor="lm")
    # A baseline JPEG: GRUB reads it, and it is far smaller than a PNG.
    path = os.path.join(GRUB, "background.jpg")
    bg.convert("RGB").save(path, quality=90, optimize=True, progressive=False)
    print(f"{path}: {w}x{h}")


def nine_slice(prefix, size, radius, fill, border=None):
    """GRUB pixmap boxes: prefix_{nw,n,ne,w,c,e,sw,s,se}.png cut from a
    rounded rectangle."""
    scale = 4
    img = Image.new("RGBA", (size * scale, size * scale))
    ImageDraw.Draw(img).rounded_rectangle(
        (0, 0, size * scale - 1, size * scale - 1),
        radius * scale,
        fill=fill,
        outline=border,
        width=scale if border else 0,
    )
    img = img.resize((size, size), Image.LANCZOS)
    r = radius
    boxes = {
        "nw": (0, 0, r, r), "n": (r, 0, size - r, r), "ne": (size - r, 0, size, r),
        "w": (0, r, r, size - r), "c": (r, r, size - r, size - r), "e": (size - r, r, size, size - r),
        "sw": (0, size - r, r, size), "s": (r, size - r, size - r, size), "se": (size - r, size - r, size, size),
    }
    for name, box in boxes.items():
        img.crop(box).save(os.path.join(GRUB, f"{prefix}_{name}.png"), optimize=True)


def grub_fonts():
    """Bitmap fonts for GRUB from Inter (Latin, arrows and bullets)."""
    mkfont = shutil.which("grub-mkfont") or shutil.which("grub2-mkfont")
    if not mkfont:
        print("grub-mkfont not found: keeping the existing GRUB fonts")
        return
    ranges = "0x20-0x7E,0xA0-0x17F,0x2013-0x2014,0x2022-0x2022,0x2026-0x2026,0x2190-0x2193"
    for weight, size, bold in (("Regular", 18, False), ("SemiBold", 18, True), ("Regular", 14, False)):
        src = font(size, weight).path
        out = os.path.join(GRUB, f"seglinux-{weight.lower()}-{size}.pf2")
        args = [mkfont, "-n", "SegLinux Sans", "-s", str(size), "-r", ranges, "-o", out, src]
        if bold:
            args.insert(1, "-b")
        subprocess.run(args, check=True)
        print(f"{out}")


def main():
    os.makedirs(os.path.join(GRUB, "icons"), exist_ok=True)
    dark_wallpaper()
    grub_background()
    # Menu icons: the logo for SegLinux entries ("seg" is the class GRUB
    # gives installed systems, from GRUB_DISTRIBUTOR).
    for name in ("seglinux", "seg"):
        logo(32).save(os.path.join(GRUB, "icons", f"{name}.png"), optimize=True)
    nine_slice("select", 24, 6, (61, 174, 233, 72), (61, 174, 233, 150))
    nine_slice("menu", 48, 16, (10, 16, 32, 150), (255, 255, 255, 28))
    nine_slice("thumb", 12, 3, (142, 223, 255, 140))
    grub_fonts()


if __name__ == "__main__":
    main()
