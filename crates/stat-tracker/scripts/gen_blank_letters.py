#!/usr/bin/env python3
"""Build the letter templates the report blanker compares digit glyphs with.

    python3 -I crates/stat-tracker/scripts/gen_blank_letters.py <fonts_root> <out_dir>

`fonts_root` is the directory holding the font files listed in FONTS
(`/usr/share/fonts` on Arch). Writes `letters_1440.png` and
`letters_1080.png`: 8-bit greyscale, 22 px wide, one 22x26 canvas per
letter stacked top to bottom, before normalisation (value / 255 is the
canvas value). `src/report/blank/letters.rs` loads them and pins their hash.

Each letter is drawn the way a real glyph reaches the digit reader: white on
black at the board's digit height (13 px at 1440p, 10 px at 1080p, measured
as the height of 'H'), at two horizontal sub-pixel phases, then cut with the
reader's strip and canvas code (`shadow/digits.rs` `scaled_strip`,
`piece_canvas`). Letters that are the digits' own shape at this size are left
out (O D Q o look like 0; I l i j J t look like 1); names made only of them
are a residual risk the blanker's other layers cover.

Fonts: Noto Sans and Liberation Sans (SIL Open Font License 1.1) and DejaVu
Sans (Bitstream Vera license); the canvases are rendered glyph images, not
font software.
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

CH, CW = 26, 22
DESC = 1.35
INK_THR = 0.45
LETTERS = [c for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz" if c not in "ODQoIlijJt"]
FONTS = [
    "noto/NotoSans-Regular.ttf",
    "noto/NotoSans-Medium.ttf",
    "noto/NotoSans-Bold.ttf",
    "noto/NotoSans-Black.ttf",
    "noto/NotoSans-Italic.ttf",
    "noto/NotoSans-MediumItalic.ttf",
    "noto/NotoSans-BoldItalic.ttf",
    "noto/NotoSans-BlackItalic.ttf",
    "TTF/DejaVuSans.ttf",
    "TTF/DejaVuSans-Bold.ttf",
    "TTF/DejaVuSans-BoldOblique.ttf",
    "TTF/DejaVuSansCondensed.ttf",
    "TTF/DejaVuSansCondensed-Bold.ttf",
    "TTF/DejaVuSansCondensed-BoldOblique.ttf",
    "liberation/LiberationSans-Regular.ttf",
    "liberation/LiberationSans-Bold.ttf",
    "liberation/LiberationSans-Italic.ttf",
    "liberation/LiberationSans-BoldItalic.ttf",
]
CAPS = {"1440": 13, "1080": 10}


def pyround(x):
    return int(round(x))


def pil_resize(p, nw, nh):
    return np.asarray(Image.fromarray(p.astype(np.float32)).resize((nw, nh), Image.BILINEAR), dtype=np.float64)


def strip_canvas(ink, top, dh):
    y1 = min(ink.shape[0], top + pyround(DESC * dh))
    g = ink[top:y1]
    sc = CH / (DESC * dh)
    nh = max(pyround(g.shape[0] * sc), 1)
    nw = max(pyround(g.shape[1] * sc), 1)
    r = pil_resize(g, nw, nh)
    out = np.zeros((CH, nw))
    rr = min(nh, CH)
    out[:rr] = r[:rr]
    return out, sc


def piece_canvas(strip, sc, x0, x1):
    xa = min(pyround(x0 * sc), strip.shape[1])
    xb = min(max(pyround(x1 * sc), xa + 1), strip.shape[1])
    ga = xa
    w = max(xb - xa, 0)
    if w > CW:
        ga += (w - CW) // 2
        w = CW
    off = (CW - w) // 2
    out = np.zeros((CH, CW))
    out[:, off:off + w] = strip[:, ga:ga + w]
    return out


def canvases(root, cap):
    out = []
    for rel in FONTS:
        path = os.path.join(root, rel)
        best = None
        for sz in range(6, 40):
            f = ImageFont.truetype(path, sz)
            im = Image.new("L", (80, 60), 0)
            ImageDraw.Draw(im).text((10, 10), "H", font=f, fill=255)
            rows = np.nonzero((np.asarray(im) > 115).any(axis=1))[0]
            h = rows[-1] - rows[0] + 1
            if best is None or abs(h - cap) < abs(best[1] - cap):
                best = (sz, h)
        fb = ImageFont.truetype(path, best[0] * 4)
        for phase in (0, 2):
            href = Image.new("L", (360, 240), 0)
            ImageDraw.Draw(href).text((40 + phase, 40), "H", font=fb, fill=255)
            hr = np.asarray(href.resize((90, 60), Image.BOX)) / 255.0
            hrows = np.nonzero((hr > INK_THR).any(axis=1))[0]
            top, dh = int(hrows[0]), int(hrows[-1] - hrows[0] + 1)
            for ch in LETTERS:
                big = Image.new("L", (360, 240), 0)
                ImageDraw.Draw(big).text((40 + phase, 40), ch, font=fb, fill=255)
                ink = np.asarray(big.resize((90, 60), Image.BOX)) / 255.0
                cols = np.nonzero((ink > INK_THR).any(axis=0))[0]
                if len(cols) == 0:
                    continue
                strip, sc = strip_canvas(ink, top, dh)
                out.append(piece_canvas(strip, sc, int(cols[0]), int(cols[-1]) + 1))
    return out


def main(argv):
    if len(argv) != 3:
        sys.stderr.write(__doc__)
        return 2
    root, out_dir = argv[1], argv[2]
    os.makedirs(out_dir, exist_ok=True)
    for key, cap in CAPS.items():
        cv = canvases(root, cap)
        a = np.concatenate(cv, axis=0)
        q = np.clip(np.rint(a * 255.0), 0, 255).astype(np.uint8)
        Image.fromarray(q).save(os.path.join(out_dir, "letters_%s.png" % key), optimize=True)
        print(key, len(cv), "letters", q.shape)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
