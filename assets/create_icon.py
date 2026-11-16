"""Generate the Windows icon asset; Pillow is needed only by this design script."""
from math import comb
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
HERE = Path(__file__).resolve().parent


def bezier(points, count=80):
    degree = len(points) - 1
    result = []
    for n in range(count + 1):
        t = n / count
        result.append(tuple(round(sum(comb(degree, i) * (1 - t) ** (degree - i) * t ** i * p[axis]
                                      for i, p in enumerate(points))) for axis in (0, 1)))
    return result


def shield(inset=0):
    path = [(512, 58), (862, 175)]
    path += bezier([(862, 175), (864, 493), (820, 770), (512, 955)])
    path += bezier([(512, 955), (204, 770), (160, 493), (162, 175)])
    path.append((512, 58))
    if inset:
        scale = 1 - inset
        path = [(round(512 + (x - 512) * scale), round(505 + (y - 505) * scale)) for x, y in path]
    return path


def gradient(top, bottom):
    im = Image.new("RGBA", (SIZE, SIZE))
    pixels = im.load()
    for y in range(SIZE):
        t = y / (SIZE - 1)
        color = tuple(round(a * (1 - t) + b * t) for a, b in zip(top, bottom)) + (255,)
        for x in range(SIZE):
            pixels[x, y] = color
    return im


def mask_shape(points):
    mask = Image.new("L", (SIZE, SIZE))
    ImageDraw.Draw(mask).polygon(points, fill=255)
    return mask


icon = Image.new("RGBA", (SIZE, SIZE))
shadow = mask_shape(shield())
shadow = shadow.filter(ImageFilter.GaussianBlur(26))
shadow_layer = Image.new("RGBA", (SIZE, SIZE), (0, 26, 49, 145))
icon.alpha_composite(Image.composite(shadow_layer, Image.new("RGBA", (SIZE, SIZE)), shadow))

outer = gradient((101, 244, 237), (4, 108, 168))
outer.putalpha(mask_shape(shield()))
icon.alpha_composite(outer)
inner = gradient((5, 109, 153), (4, 45, 97))
inner.putalpha(mask_shape(shield(.075)))
icon.alpha_composite(inner)

# A compact, high-contrast Wi-Fi mark stays clear at small tray sizes.
draw = ImageDraw.Draw(icon)
for box, width in [((253, 265, 771, 783), 57), ((355, 371, 669, 685), 57)]:
    draw.arc(box, 225, 315, fill=(236, 255, 255, 255), width=width)
draw.ellipse((474, 496, 550, 572), fill=(236, 255, 255, 255))

icon.save(HERE / "guardian.png")
icon.save(HERE / "guardian.ico", sizes=[(16, 16), (20, 20), (24, 24), (32, 32),
                                         (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)])
