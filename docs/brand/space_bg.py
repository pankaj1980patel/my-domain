# Deep-space app background: dark navy, soft nebula clouds, a star field.
# usage: python3 space_bg.py OUT.webp WIDTH HEIGHT SEED
import random
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

out, W, H, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
rng = random.Random(seed)

# Base: vertical gradient, near-black navy.
base = Image.new("RGB", (1, 256))
for y in range(256):
    t = y / 255
    base.putpixel((0, y), (int(5 + 3 * t), int(8 + 4 * t), int(20 + 6 * (1 - abs(t - 0.4)))))
img = base.resize((W, H), Image.BICUBIC)

# Nebula: blurred colour blobs, broken up by noise, drawn small then scaled.
sw, sh = W // 8, H // 8
def cloud(color, cx, cy, rx, ry, strength):
    layer = Image.new("L", (sw, sh), 0)
    d = ImageDraw.Draw(layer)
    for _ in range(14):
        x = cx * sw + rng.gauss(0, rx * sw * 0.45)
        y = cy * sh + rng.gauss(0, ry * sh * 0.45)
        r = rng.uniform(0.25, 0.6) * min(rx * sw, ry * sh)
        d.ellipse((x - r, y - r * rng.uniform(0.5, 1), x + r, y + r * rng.uniform(0.5, 1)), fill=int(255 * strength))
    layer = layer.filter(ImageFilter.GaussianBlur(min(sw, sh) * 0.15))
    noise = Image.effect_noise((sw, sh), 110).filter(ImageFilter.GaussianBlur(2)).resize((sw // 2, sh // 2)).resize((sw, sh), Image.BICUBIC)
    layer = ImageChops.multiply(layer, noise.point(lambda v: min(255, int(v * 1.6))))
    layer = layer.filter(ImageFilter.GaussianBlur(min(sw, sh) * 0.05)).resize((W, H), Image.BICUBIC)
    tint = Image.new("RGB", (W, H), color)
    return ImageChops.multiply(tint, Image.merge("RGB", (layer, layer, layer)))

for color, cx, cy, rx, ry, s in [
    ((70, 40, 150), 0.18, 0.15, 0.55, 0.40, 0.55),   # violet, top-left
    ((20, 90, 160), 0.85, 0.80, 0.55, 0.45, 0.55),   # blue, bottom-right
    ((10, 110, 130), 0.70, 0.30, 0.35, 0.30, 0.30),  # teal
    ((150, 100, 40), 0.30, 0.85, 0.30, 0.25, 0.18),  # faint amber
]:
    img = ImageChops.add(img, cloud(color, cx, cy, rx, ry, s))

# Stars: many faint pin-pricks, fewer bright ones with a soft glow.
stars = Image.new("RGB", (W, H), 0)
d = ImageDraw.Draw(stars)
area = W * H / 1e6
for _ in range(int(900 * area)):
    x, y = rng.uniform(0, W), rng.uniform(0, H)
    v = int(rng.uniform(60, 200))
    tint = rng.choice([(1, 1, 1), (0.85, 0.9, 1), (1, 0.95, 0.85)])
    r = rng.choice([0.6, 0.8, 1.0, 1.2])
    d.ellipse((x - r, y - r, x + r, y + r), fill=tuple(int(v * c) for c in tint))
glow = Image.new("RGB", (W, H), 0)
g = ImageDraw.Draw(glow)
for _ in range(int(14 * area)):
    x, y = rng.uniform(0, W), rng.uniform(0, H)
    r = rng.uniform(1.4, 2.2)
    col = rng.choice([(255, 255, 255), (200, 220, 255), (255, 230, 190)])
    d.ellipse((x - r, y - r, x + r, y + r), fill=col)
    g.ellipse((x - r * 5, y - r * 5, x + r * 5, y + r * 5), fill=tuple(c // 3 for c in col))
glow = glow.filter(ImageFilter.GaussianBlur(6))
img = ImageChops.add(img, ImageChops.add(stars, glow))

# Vignette keeps the edges calm behind the UI.
vig = Image.new("L", (W, H), 0)
ImageDraw.Draw(vig).ellipse((-W * 0.25, -H * 0.25, W * 1.25, H * 1.25), fill=255)
vig = vig.filter(ImageFilter.GaussianBlur(min(W, H) * 0.15))
img = Image.composite(img, Image.new("RGB", (W, H), (3, 4, 10)), vig)

img.save(out, "WEBP", quality=82, method=6)
print(out, img.size)
