# Draws the S of data/artwork/seglinux.svg as text for fastfetch (logo.txt):
# half blocks, one colour per cell from the logo's gradient ($1 to $3,
# config.jsonc). Usage: python3 make-logo.py 18 > logo.txt
import math, sys
from PIL import Image, ImageDraw

rows = int(sys.argv[1])
SC = 16  # supersampling: units -> pixels
img = Image.new("L", (128 * SC, 128 * SC), 0)
d = ImageDraw.Draw(img)
r = 6.5 * SC
def arc(cx, cy, a0, a1):
    n = 400
    for i in range(n + 1):
        a = math.radians(a0 + (a1 - a0) * i / n)
        x, y = (cx + 19 * math.cos(a)) * SC, (cy + 19 * math.sin(a)) * SC
        d.ellipse((x - r, y - r, x + r, y + r), fill=255)
a0 = math.degrees(math.atan2(-8.9, 16.8))
arc(64, 45, a0, -270)          # top half: upper right, over the top, to the centre
arc(64, 83, -90, 180 + a0)     # bottom half: centre, round the right, lower left
d.line((40 * SC, 76 * SC, 88 * SC, 52 * SC), fill=0, width=5 * SC)  # the cut

x0, x1, y0, y1 = 38.5, 89.5, 19.5, 108.5
h = rows * 2
w = round(h * (x1 - x0) / (y1 - y0))
crop = img.crop((int(x0 * SC), int(y0 * SC), int(x1 * SC), int(y1 * SC))).resize((w, h), Image.BOX)
px = crop.load()

def t_at(x, y):
    # Position along the gradient (40,24) -> (88,104), 0..1.
    ux, uy = x0 + (x + 0.5) * (x1 - x0) / w, y0 + (y + 0.5) * (y1 - y0) / h
    gx, gy = 48, 80
    return max(0, min(1, ((ux - 40) * gx + (uy - 24) * gy) / (gx * gx + gy * gy)))

out = []
for row in range(rows):
    line, cur = "", None
    for col in range(w):
        top, bot = px[col, 2 * row] >= 128, px[col, 2 * row + 1] >= 128
        ch = {(1, 1): "█", (1, 0): "▀", (0, 1): "▄", (0, 0): " "}[(top, bot)]
        if ch != " ":
            t = t_at(col, 2 * row + 0.5)
            c = "1" if t < 0.36 else "2" if t < 0.72 else "3"
            if c != cur:
                line += "$" + c
                cur = c
        line += ch
    out.append(line.rstrip())
print("\n".join(out))
