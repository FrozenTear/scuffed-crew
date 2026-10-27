"""Draw the PNG icons the 0.6 top bar uses that are not traced from SVG.

Run from this folder with Pillow installed:

    python3 draw_icons.py

It writes tab-icons/overview-dim.png and tab-icons/overview-on.png (four
rounded squares spanning 14 of the 16 px box, as in the spec) and warn.png
(the chip's warning sign, 12 px). Each is drawn at 2x for a 1x box, from an
8x supersampled mask, so a rerun gives the same files.
"""

from PIL import Image, ImageDraw

SUPER = 8
TEXT = (242, 242, 247)  # theme TEXT, selected tab
TEXT_2 = (201, 201, 214)  # theme TEXT_2, other tabs
WARN = (245, 184, 74)  # theme WARN


def paint(mask, size, colour):
    """Shrink an 8x mask to `size` px and fill it with `colour`."""
    alpha = mask.resize((size, size), Image.BOX)
    out = Image.new("RGBA", (size, size), colour + (0,))
    out.putalpha(alpha)
    return out


def overview_mask():
    """Four rounded squares, 12 px each with a 4 px gap, from 2 to 30 of 32."""
    s = SUPER
    mask = Image.new("L", (32 * s, 32 * s), 0)
    draw = ImageDraw.Draw(mask)
    stroke = round(2.2 * s)
    for x in (2, 18):
        for y in (2, 18):
            draw.rounded_rectangle(
                (x * s, y * s, (x + 12) * s - 1, (y + 12) * s - 1),
                radius=round(3.5 * s),
                outline=255,
                width=stroke,
            )
    return mask


def warn_mask():
    """A triangle with round joins and an exclamation mark, 24 px."""
    s = SUPER
    mask = Image.new("L", (24 * s, 24 * s), 0)
    draw = ImageDraw.Draw(mask)
    stroke = round(2.2 * s)
    top, right, left = (12 * s, 3 * s), (22 * s, 21 * s), (2 * s, 21 * s)
    draw.line([top, right, left, top, right], fill=255, width=stroke, joint="curve")
    for x, y in (top, right, left):
        r = stroke // 2
        draw.ellipse((x - r, y - r, x + r, y + r), fill=255)
    half = stroke // 2
    draw.rounded_rectangle(
        (12 * s - half, 9 * s, 12 * s + half, 15 * s), radius=half, fill=255
    )
    dot = round(1.4 * s)
    draw.ellipse((12 * s - dot, 18 * s - dot, 12 * s + dot, 18 * s + dot), fill=255)
    return mask


def main():
    mask = overview_mask()
    paint(mask, 32, TEXT).save("tab-icons/overview-on.png", optimize=True)
    paint(mask, 32, TEXT_2).save("tab-icons/overview-dim.png", optimize=True)
    paint(warn_mask(), 24, WARN).save("warn.png", optimize=True)


if __name__ == "__main__":
    main()
