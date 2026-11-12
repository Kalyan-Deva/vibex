"""Builds the VibeX app icon and logos from the final icon artwork.

    python assets/make_icon.py

Source: vibex-icon-source.png (the final VibeX icon, 1254 x 1254: a glass
rounded tile with the ribbon mark, orbit, sparkle and cursor, on black).
The black outside the tile is cut away with an anti-aliased rounded mask.

Writes, next to this script:
  vibex-<n>.png          16 .. 1024 px app icons, transparent corners
  vibex.ico              multi-size Windows icon (embedded in vibex.exe by build.rs)
  vibex-logo.png         icon + "VibeX Terminal" wordmark, light text, for dark backgrounds
  vibex-logo-light.png   the same with dark text, for light backgrounds
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
SRC = HERE / "vibex-icon-source.png"

# the tile inside the source image (measured), and its corner radius
TILE = (102, 114, 1149, 1147)
RADIUS = 232
PAD = 0.04  # transparent margin around the tile in the output, as a fraction

TEXT_LIGHT, MUTED_LIGHT = (0xEE, 0xEC, 0xFF), (0xA6, 0xA2, 0xD8)
TEXT_DARK, MUTED_DARK = (0x10, 0x0E, 0x26), (0x5A, 0x55, 0x8A)


def master(size=2048):
    """The tile cut out of the source, centred on a transparent square."""
    src = Image.open(SRC).convert("RGB")
    x0, y0, x1, y1 = TILE
    tile = src.crop(TILE)
    # anti-aliased mask: draw at 4x then downsample
    w, h = tile.size
    big = Image.new("L", (w * 4, h * 4), 0)
    ImageDraw.Draw(big).rounded_rectangle((0, 0, w * 4 - 1, h * 4 - 1), RADIUS * 4, fill=255)
    tile = tile.convert("RGBA")
    tile.putalpha(big.resize((w, h), Image.LANCZOS))
    inner = round(size * (1 - 2 * PAD))
    tile = tile.resize((inner, round(inner * h / w)), Image.LANCZOS)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.paste(tile, ((size - tile.width) // 2, (size - tile.height) // 2), tile)
    return out


def fonts(size):
    path = r"C:\Windows\Fonts\SegUIVar.ttf"
    big = ImageFont.truetype(path, size)
    big.set_variation_by_name("Semibold Display")
    small = ImageFont.truetype(path, size)
    small.set_variation_by_name("Light Display")
    return big, small


def main():
    m = master()
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256, 512, 1024]
    pngs = {n: m.resize((n, n), Image.LANCZOS) for n in sizes}
    for n, im in pngs.items():
        im.save(HERE / f"vibex-{n}.png")
    ico = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    pngs[256].save(HERE / "vibex.ico", sizes=[(n, n) for n in ico],
                   append_images=[pngs[n] for n in ico if n != 256])

    # icon + wordmark: "VibeX" Segoe UI Variable Semibold, "Terminal" Light
    h, size = 256, 132
    big, small = fonts(size)
    probe = ImageDraw.Draw(Image.new("L", (1, 1)))
    gap = size * 0.26
    text_w = probe.textlength("VibeX", font=big) + gap + probe.textlength("Terminal", font=small)
    width = round(h + 40 + text_w + 40)
    baseline = h // 2 + round(size * 0.36)
    for name, ink, muted in (("vibex-logo.png", TEXT_LIGHT, MUTED_LIGHT),
                             ("vibex-logo-light.png", TEXT_DARK, MUTED_DARK)):
        logo = Image.new("RGBA", (width, h), (0, 0, 0, 0))
        logo.paste(pngs[256], (0, 0), pngs[256])
        d = ImageDraw.Draw(logo)
        x = h + 40
        d.text((x, baseline), "VibeX", font=big, fill=ink + (255,), anchor="ls")
        x += probe.textlength("VibeX", font=big) + gap
        d.text((x, baseline), "Terminal", font=small, fill=muted + (255,), anchor="ls")
        logo.save(HERE / name)
    print("wrote", ", ".join(sorted(p.name for p in HERE.glob("vibex*") if p != SRC)))


if __name__ == "__main__":
    main()
