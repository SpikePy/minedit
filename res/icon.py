# Regenerates minedit.ico: python3 res/icon.py res/minedit.ico preview.png
import sys
from PIL import Image, ImageDraw

BG, FG, CARET = (34, 37, 43), (205, 210, 218), (79, 157, 255)

def render(n):
    ss = 8
    N = n * ss
    im = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    m = 0 if n <= 24 else round(0.06 * N)
    d.rounded_rectangle([m, m, N - m - 1, N - m - 1], radius=round(0.2 * N), fill=BG)
    if n <= 24:   # fewer, bolder strokes for tiny sizes
        t, x0, lines = 0.12, 0.22, [(0.32, 0.56), (0.60, 0.30)]
    else:
        t, x0, lines = 0.07, 0.24, [(0.30, 0.52), (0.47, 0.36), (0.64, 0.22)]
    for y, w in lines:
        d.rounded_rectangle([x0 * N, (y - t / 2) * N, (x0 + w) * N, (y + t / 2) * N], radius=t * N / 2, fill=FG)
    y, w = lines[-1]
    cx = (x0 + w + (0.08 if n > 24 else 0.10)) * N
    cw, ch = (0.06 if n > 24 else 0.11) * N, (0.18 if n > 24 else 0.30) * N
    d.rectangle([cx, y * N - ch / 2, cx + cw, y * N + ch / 2], fill=CARET)
    return im.resize((n, n), Image.LANCZOS)

sizes = [256, 64, 48, 40, 32, 24, 20, 16]
imgs = [render(n) for n in sizes]
imgs[0].save(sys.argv[1], sizes=[(n, n) for n in sizes], append_images=imgs[1:])
prev = Image.new("RGBA", (256 + 64 + 48 + 32 + 24 + 16 + 70, 256), (240, 240, 240, 255))
x = 0
for im in imgs:
    if im.width in (256, 64, 48, 32, 24, 16):
        prev.alpha_composite(im, (x, 0)); x += im.width + 10
prev.save(sys.argv[2])
