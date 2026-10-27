"""Draw the PNG icons the 0.6 top bar uses that are not traced from SVG.

Run from this folder with Pillow installed:

    python3 draw_icons.py

It writes tab-icons/overview.png (four rounded squares spanning 14 of the
16 px box, as in the spec; a white alpha mask the GUI tints), warn.png
(the chip's warning sign, 12 px), and for the Settings page check.png
(the status strip's check in a circle, 14 px), lock.png (the env lock,
12 px) and fold-open.png / fold-closed.png (the ADVANCED row chevrons,
14 px). For the open Games row it draws upload.png, clock.png, pencil.png,
flag.png and picture.png (12 px, TEXT_2) and warn-loss.png (the warning
sign in the loss colour, 12 px). Each is drawn at 2x for a 1x box, from an 8x supersampled mask,
so a rerun gives the same files.

The other five tab masks (games, heroes, maps, seasons, settings) are not
drawn here. They were rasterised once at 2x (32 x 32, white on alpha)
from the stroke SVG paths in the 0.6 design spec, which is not in this
repository, so this script cannot redraw them. A replacement is any
32 x 32 white alpha mask in the same style.
"""

from PIL import Image, ImageDraw

SUPER = 8
WHITE = (255, 255, 255)  # tab masks; src/icons.rs tints them
TEXT_2 = (201, 201, 214)  # theme TEXT_2, the chevrons
WARN = (245, 184, 74)  # theme WARN
OK = (70, 216, 164)  # theme OK
TEXT_3 = (141, 141, 160)  # theme TEXT_3
DANGER = (255, 92, 122)  # theme DANGER, r1.1 --loss


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


def check_mask():
    """A circle outline with a tick inside, 28 px."""
    s = SUPER
    mask = Image.new("L", (28 * s, 28 * s), 0)
    draw = ImageDraw.Draw(mask)
    stroke = round(2.4 * s)
    draw.ellipse((2 * s, 2 * s, 26 * s - 1, 26 * s - 1), outline=255, width=stroke)
    tick = [(9 * s, 14.5 * s), (12.5 * s, 18 * s), (19 * s, 10.5 * s)]
    draw.line(tick, fill=255, width=stroke, joint="curve")
    for x, y in (tick[0], tick[-1]):
        r = stroke // 2
        draw.ellipse((x - r, y - r, x + r, y + r), fill=255)
    return mask


def lock_mask():
    """A padlock: a shackle over a rounded body, 24 px."""
    s = SUPER
    mask = Image.new("L", (24 * s, 24 * s), 0)
    draw = ImageDraw.Draw(mask)
    stroke = round(2.4 * s)
    draw.arc((7 * s, 3 * s, 17 * s, 15 * s), 180, 360, fill=255, width=stroke)
    draw.line([(7 * s + stroke // 2, 9 * s), (7 * s + stroke // 2, 11 * s)], fill=255, width=stroke)
    draw.line([(17 * s - stroke // 2, 9 * s), (17 * s - stroke // 2, 11 * s)], fill=255, width=stroke)
    draw.rounded_rectangle((4 * s, 10 * s, 20 * s, 22 * s), radius=round(2.5 * s), fill=255)
    return mask


def chevron_mask(down):
    """A chevron pointing right, or down when `down`, 28 px."""
    s = SUPER
    mask = Image.new("L", (28 * s, 28 * s), 0)
    draw = ImageDraw.Draw(mask)
    stroke = round(2.4 * s)
    if down:
        points = [(8 * s, 11 * s), (14 * s, 17 * s), (20 * s, 11 * s)]
    else:
        points = [(11 * s, 8 * s), (17 * s, 14 * s), (11 * s, 20 * s)]
    draw.line(points, fill=255, width=stroke, joint="curve")
    for x, y in (points[0], points[-1]):
        r = stroke // 2
        draw.ellipse((x - r, y - r, x + r, y + r), fill=255)
    return mask


def _caps(draw, points, stroke):
    for x, y in points:
        r = stroke // 2
        draw.ellipse((x - r, y - r, x + r, y + r), fill=255)


def _canvas():
    s = SUPER
    mask = Image.new("L", (24 * s, 24 * s), 0)
    return s, mask, ImageDraw.Draw(mask), round(2.2 * s)


def upload_mask():
    """An arrow pointing up, 24 px."""
    s, mask, draw, stroke = _canvas()
    shaft = [(12 * s, 20 * s), (12 * s, 4 * s)]
    head = [(6 * s, 10 * s), (12 * s, 4 * s), (18 * s, 10 * s)]
    draw.line(shaft, fill=255, width=stroke)
    draw.line(head, fill=255, width=stroke, joint="curve")
    _caps(draw, [shaft[0], head[0], head[1], head[2]], stroke)
    return mask


def clock_mask():
    """A clock face with two hands, 24 px."""
    s, mask, draw, stroke = _canvas()
    draw.ellipse((2 * s, 2 * s, 22 * s - 1, 22 * s - 1), outline=255, width=stroke)
    hands = [(12 * s, 7 * s), (12 * s, 12 * s), (16 * s, 14 * s)]
    draw.line(hands, fill=255, width=stroke, joint="curve")
    _caps(draw, hands, stroke)
    return mask


def pencil_mask():
    """A pencil on a slant, 24 px."""
    s, mask, draw, stroke = _canvas()
    body = [(4 * s, 20 * s), (5 * s, 15 * s), (16 * s, 4 * s), (20 * s, 8 * s), (9 * s, 19 * s), (4 * s, 20 * s)]
    draw.line(body, fill=255, width=stroke, joint="curve")
    draw.line([(13.5 * s, 6.5 * s), (17.5 * s, 10.5 * s)], fill=255, width=stroke)
    return mask


def flag_mask():
    """A flag on a pole, 24 px."""
    s, mask, draw, stroke = _canvas()
    pole = [(5 * s, 21 * s), (5 * s, 3 * s)]
    draw.line(pole, fill=255, width=stroke)
    cloth = [(5 * s, 4 * s), (19 * s, 4 * s), (15 * s, 9 * s), (19 * s, 14 * s), (5 * s, 14 * s)]
    draw.line(cloth, fill=255, width=stroke, joint="curve")
    _caps(draw, [pole[0]], stroke)
    return mask


def picture_mask():
    """A framed picture with a hill and a sun, 24 px."""
    s, mask, draw, stroke = _canvas()
    draw.rounded_rectangle((2 * s, 4 * s, 22 * s - 1, 20 * s - 1), radius=round(2.5 * s), outline=255, width=stroke)
    hill = [(4 * s, 17 * s), (9 * s, 11 * s), (13 * s, 15 * s), (16 * s, 12 * s), (20 * s, 17 * s)]
    draw.line(hill, fill=255, width=stroke, joint="curve")
    dot = round(1.6 * s)
    draw.ellipse((16 * s - dot, 8 * s - dot, 16 * s + dot, 8 * s + dot), fill=255)
    return mask


def main():
    mask = overview_mask()
    paint(mask, 32, WHITE).save("tab-icons/overview.png", optimize=True)
    paint(warn_mask(), 24, WARN).save("warn.png", optimize=True)
    paint(check_mask(), 28, OK).save("check.png", optimize=True)
    paint(lock_mask(), 24, TEXT_3).save("lock.png", optimize=True)
    paint(chevron_mask(True), 28, TEXT_2).save("fold-open.png", optimize=True)
    paint(chevron_mask(False), 28, TEXT_2).save("fold-closed.png", optimize=True)
    paint(upload_mask(), 24, TEXT_2).save("upload.png", optimize=True)
    paint(clock_mask(), 24, TEXT_2).save("clock.png", optimize=True)
    paint(pencil_mask(), 24, TEXT_2).save("pencil.png", optimize=True)
    paint(flag_mask(), 24, TEXT_2).save("flag.png", optimize=True)
    paint(picture_mask(), 24, TEXT_2).save("picture.png", optimize=True)
    paint(warn_mask(), 24, DANGER).save("warn-loss.png", optimize=True)


if __name__ == "__main__":
    main()
