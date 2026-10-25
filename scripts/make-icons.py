#!/usr/bin/env python3
"""Draws the GitChrono app icon and writes every packaged size.

    python3 scripts/make-icons.py      (needs Pillow; macOS for the .icns)

The mark: a mainline, and a branch that leaves it and merges back with the
same rounded elbows the commit graph uses, in the graph's lane colours.
"""
import math
import pathlib
import shutil
import subprocess
import tempfile

from PIL import Image, ImageDraw

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "packaging"
S = 4  # supersampling

BG_TOP, BG_BOTTOM = (38, 43, 54), (17, 19, 24)
BORDER = (58, 64, 77)
CYAN, BLUE, VIOLET = (21, 160, 191), (47, 125, 246), (155, 79, 224)


def draw(size=1024):
    n = size * S
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    u = n / 1024  # design units are on a 1024 grid

    # Rounded square on Apple's icon grid (824 of 1024), vertical gradient.
    inset, radius = 100 * u, 185 * u
    mask = Image.new("L", (n, n), 0)
    ImageDraw.Draw(mask).rounded_rectangle((inset, inset, n - inset, n - inset), radius, fill=255)
    grad = Image.new("RGBA", (n, n))
    gd = ImageDraw.Draw(grad)
    for y in range(n):
        t = min(max((y - inset) / (n - 2 * inset), 0), 1)
        gd.line([(0, y), (n, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM)) + (255,))
    img.paste(grad, (0, 0), mask)
    d.rounded_rectangle((inset, inset, n - inset, n - inset), radius, outline=BORDER, width=round(6 * u))

    w = round(46 * u)
    main_x, branch_x = 405 * u, 625 * u
    top, mid, bottom = 330 * u, 512 * u, 694 * u
    r = 90 * u  # elbow radius

    def stroke(points, color):
        """One continuous line through `points`, with round joins and ends."""
        d.line(points, fill=color, width=w, joint="curve")
        for x, y in (points[0], points[-1]):
            d.ellipse((x - w / 2, y - w / 2, x + w / 2, y + w / 2), fill=color)

    def corner(cx, cy, start_deg, end_deg, steps=24):
        return [(cx + r * math.cos(math.radians(a)), cy + r * math.sin(math.radians(a)))
                for a in (start_deg + (end_deg - start_deg) * i / steps for i in range(steps + 1))]

    d.line([(main_x, top), (main_x, bottom)], fill=CYAN, width=w)
    # Merge: out of the top node to the right, then a rounded turn down to the branch.
    stroke([(main_x, top)] + corner(branch_x - r, top + r, -90, 0) + [(branch_x, mid)], VIOLET)
    # Fork: down the branch lane, then a rounded turn into the bottom node.
    stroke([(branch_x, mid)] + corner(branch_x - r, bottom - r, 0, 90) + [(main_x, bottom)], VIOLET)

    def node(x, y, color):
        R, ring = 66 * u, 16 * u
        d.ellipse((x - R - ring, y - R - ring, x + R + ring, y + R + ring), fill=BG_BOTTOM)
        d.ellipse((x - R, y - R, x + R, y + R), fill=color)

    node(main_x, top, CYAN)
    node(branch_x, mid, VIOLET)
    node(main_x, bottom, BLUE)
    return img.resize((size, size), Image.LANCZOS)


def main():
    master = draw(1024)
    for sub in ("macos", "linux", "windows"):
        (OUT / sub).mkdir(parents=True, exist_ok=True)
    master.save(OUT / "icon-1024.png")
    master.resize((512, 512), Image.LANCZOS).save(OUT / "linux" / "gitchrono.png")
    master.save(OUT / "windows" / "gitchrono.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])

    if shutil.which("iconutil"):
        with tempfile.TemporaryDirectory() as tmp:
            iconset = pathlib.Path(tmp) / "GitChrono.iconset"
            iconset.mkdir()
            for s in (16, 32, 128, 256, 512):
                master.resize((s, s), Image.LANCZOS).save(iconset / f"icon_{s}x{s}.png")
                master.resize((s * 2, s * 2), Image.LANCZOS).save(iconset / f"icon_{s}x{s}@2x.png")
            subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(OUT / "macos" / "GitChrono.icns")], check=True)
    print("wrote", ", ".join(str(p.relative_to(ROOT)) for p in sorted(OUT.rglob("*")) if p.suffix in {".png", ".ico", ".icns"}))


if __name__ == "__main__":
    main()
