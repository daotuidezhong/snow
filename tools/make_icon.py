"""Generate assets/snow.ico: a six-armed snowflake on a frosty blue disc."""
import math, sys
from PIL import Image, ImageDraw

def flake(size):
    S = size * 8  # supersample
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    c = S / 2
    d.ellipse([S*0.02, S*0.02, S*0.98, S*0.98], fill=(38, 92, 150, 255))
    w = max(int(S * 0.075), 1)
    col = (255, 255, 255, 255)
    R = S * 0.40
    for k in range(6):
        a = math.pi / 2 + k * math.pi / 3
        ex, ey = c + R * math.cos(a), c - R * math.sin(a)
        d.line([c, c, ex, ey], fill=col, width=w)
        for t, bl in ((0.55, 0.20), (0.80, 0.13)):
            px, py = c + R * t * math.cos(a), c - R * t * math.sin(a)
            for s in (-1, 1):
                b = a + s * math.pi / 4
                d.line([px, py, px + S * bl * math.cos(b), py - S * bl * math.sin(b)], fill=col, width=w)
    d.ellipse([c - w, c - w, c + w, c + w], fill=col)
    return img.resize((size, size), Image.LANCZOS)

sizes = [16, 20, 24, 32, 40, 48, 64, 256]
big = flake(256)
big.save(sys.argv[1], format="ICO", sizes=[(s, s) for s in sizes],
         append_images=[flake(s) for s in sizes[:-1]])
