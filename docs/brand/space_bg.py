# App background in the style of the logo source image (logo-source.png):
# a near-black blue-teal sky, a faint central glow, a soft diagonal Milky Way
# band with dark dust lanes, and sparse, tiny stars.
#
# usage: python3 space_bg.py OUT.webp WIDTH HEIGHT SEED
import math
import random
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

out, W, H, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
rng = random.Random(seed)
M = min(W, H)


def radial(w, h, inner, outer, cx=0.5, cy=0.45, scale=0.75):
    """Radial gradient from `inner` (centre) to `outer` (edges), drawn small."""
    sw, sh = 160, max(1, int(160 * h / w))
    g = Image.new("RGB", (sw, sh))
    for y in range(sh):
        for x in range(sw):
            d = math.hypot((x / sw - cx) * (w / M), (y / sh - cy) * (h / M)) / scale
            t = min(1.0, d) ** 1.4
            g.putpixel((x, y), tuple(int(a + (b - a) * t) for a, b in zip(inner, outer)))
    return g.resize((w, h), Image.BICUBIC)


def noise(size, blur, up):
    """Smooth value noise: random field at 1/up scale, blurred, scaled up."""
    w, h = max(2, size[0] // up), max(2, size[1] // up)
    n = Image.effect_noise((w, h), 120).filter(ImageFilter.GaussianBlur(blur))
    return n.resize(size, Image.BICUBIC)


# Sky: near-black blue-teal, a touch lighter in the middle.
img = radial(W, H, inner=(13, 25, 36), outer=(1, 7, 14))

# Milky Way band: a soft diagonal haze from lower-left to upper-right.
angle = 22 if W >= H else 58
band = Image.new("L", (W * 2, H * 2), 0)
bd = ImageDraw.Draw(band)
thick = M * 0.22
bd.rectangle((0, H - thick / 2, W * 2, H + thick / 2), fill=255)
band = band.rotate(angle, resample=Image.BICUBIC, center=(W, H))
band = band.crop((W // 2, H // 2, W // 2 + W, H // 2 + H)).filter(ImageFilter.GaussianBlur(M * 0.09))
# Break it up: big soft clumps, and darker dust lanes along it.
clumps = ImageChops.multiply(noise((W, H), 3, 24), noise((W, H), 2, 10)).point(lambda v: min(255, int(v * 2.2)))
dust = noise((W, H), 1.5, 14).point(lambda v: 255 if v > 150 else int(80 + v))
haze = ImageChops.multiply(ImageChops.multiply(band, clumps), dust)
haze = haze.filter(ImageFilter.GaussianBlur(M * 0.006))
img = ImageChops.add(img, Image.merge("RGB", [haze.point(lambda v, k=k: int(v * k)) for k in (0.21, 0.25, 0.29)]))

# Stars: dense specks inside the band, sparse ones elsewhere, a few brighter.
stars = Image.new("RGB", (W, H), 0)
d = ImageDraw.Draw(stars)
area = W * H / 1e6
bandpx = band.load()
placed = 0
while placed < int(1400 * area):
    x, y = rng.randrange(W), rng.randrange(H)
    if rng.random() * 255 > bandpx[x, y]:
        continue
    v = int(rng.uniform(40, 140))
    d.point((x, y), fill=(v, int(v * 1.05), int(v * 1.15)))
    placed += 1
for _ in range(int(260 * area)):
    x, y = rng.uniform(0, W), rng.uniform(0, H)
    v = int(rng.uniform(70, 210))
    r = rng.choice([0.5, 0.7, 0.9])
    d.ellipse((x - r, y - r, x + r, y + r), fill=(v, v, min(255, int(v * 1.1))))
glow = Image.new("RGB", (W, H), 0)
g = ImageDraw.Draw(glow)
for _ in range(int(6 * area)):
    x, y = rng.uniform(0, W), rng.uniform(0, H)
    r = rng.uniform(1.1, 1.6)
    d.ellipse((x - r, y - r, x + r, y + r), fill=(235, 242, 255))
    g.ellipse((x - r * 4, y - r * 4, x + r * 4, y + r * 4), fill=(60, 75, 95))
img = ImageChops.add(img, ImageChops.add(stars, glow.filter(ImageFilter.GaussianBlur(4))))

img.save(out, "WEBP", quality=86, method=6)
print(out, img.size)
