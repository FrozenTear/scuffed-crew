#!/usr/bin/env python3
"""Reader pack generator for the stat tracker (heroes-v2, maps-v1, words-v1).

The script is in git. Its inputs and its output are not: every input is game
art (DataTool extracts) or a member capture, so both stay on the build
machine. The script reads only the paths given on the command line and
refuses to write into a git work tree.

Output layout (``--out DIR``, created with mode 0700)::

    DIR/packs/manifest.json      PACKS_DIR list: [{name, version, sha256, size}]
    DIR/packs/<name>.tar         ustar, one top-level manifest.json per pack
    DIR/staging/<name>/          the unpacked files, for a copy data dir
    DIR/pack-versions.json       {name: version}, as a data dir records it
    DIR/pack-sources.json        {name: {source facts}}; not read by the tracker

Pack formats (they match the readers in ``src/``):

* ``heroes-v2.tar`` / ``heroes-v2-1080.tar``: ``<hero-key>.png`` (RGBA, alpha
  is the foreground mask) plus ``special/<class>*.png``; the layout of
  ``shadow/heroes.rs`` and ``packs.rs`` (installs to ``templates/heroes``).
* ``maps-v1.tar``: ``atlas.json`` plus one 8-bit grey PNG per glyph, H=32
  rows (``shadow/banner.rs``; installs to ``templates/maps``).
* ``words-v1.tar``: ``<source>_<word>.png`` result-word templates, 8-bit grey
  at the 720p work scale (``shadow/result.rs``; installs to
  ``templates/result``), an optional ``layouts.json`` (extra or replaced
  result.rs regions), ``phrase_<id>.png`` phase-title masks plus
  ``phrases.json``, other masks a later reader picks up by name (for example
  ``p4_final_score.png``, ``rank_title_competitive.png``), and ``words.json``:
  per file, cut or rendered, ``validated``, region and fit notes. The result
  loader reads only ``<layout>_<victory|defeat|draw>.png`` and ignores the rest.

Game fonts: ``maps --fill-font`` and ``render`` spec lines draw glyphs or
words with a font file from the build machine. The font never enters a pack;
rendered files are marked ``rendered: true, validated: false`` so a reader
can keep them shadow-only until real frames confirm them.

Avoid the top ~3% of the frame in any region: overlays such as MangoHud draw
a stats strip there on member captures.

Subcommands (run each inside a capped scope on a shared host, e.g.
``systemd-run --user --scope -p MemoryMax=3G nice -n19 python3 -I packgen.py``)::

    # 1. hero portraits from DataTool `extract-hero-icons` + `list-heroes --json`
    packgen.py heroes --icons EXTRACT/HeroIcons --heroes-json list-heroes.json \\
        --special V1_TEMPLATES/special --keys-from V1_TEMPLATES --build 154088 \\
        --size 74 --name heroes-v2.tar --out BUILD
    packgen.py heroes ... --size 56 --name heroes-v2-1080.tar --out BUILD

    # 2. banner glyph atlas from labelled banners (TSV: image<TAB>spelling;
    #    an image is a full frame or a debug/mapcrops label box; "A|B" lists
    #    alternative spellings of one map)
    packgen.py maps --labels labels.tsv --fill-font FONTS/futura.ttf --out BUILD

    # 3. result words, phase phrases, rendered words (TSV; the line kinds are
    #    listed in the cmd_words docstring)
    packgen.py words --spec words.tsv --out BUILD

    # PACKS_DIR for the site server: BUILD/packs (manifest.json + the tars)

Slot ``0x40C9`` of ``list-heroes`` is the scoreboard portrait (256x256, the
same art as heroes-v1). It is area-downscaled with premultiplied alpha, and
colour is bled into fully transparent pixels so a resize never pulls black
into the foreground.

Python 3.11+, numpy and Pillow. No network, no cargo.
"""
from __future__ import annotations

import argparse
import functools
import hashlib
import io
import json
import os
import re
import sys
import tarfile
import unicodedata
from pathlib import Path

import numpy as np
from PIL import Image

PORTRAIT_SLOT = "0000000040C9.01C"
H = 32  # banner.rs H
ELASTIC = 3
MAX_RATIO = 0.15
STEP_COST = 0.02
WORK = 0.5  # result.rs WORK (720p work scale)
RESULT_LAYOUTS = {  # result.rs Layout::builtin
    "accolade": ((0.005, 0.035, 0.255, 0.095), "max"),
    "rank_screen": ((0.010, 0.145, 0.250, 0.225), "min"),
    "end_title": ((0.320, 0.340, 0.680, 0.520), "max"),
    "tab_header": ((0.300, 0.020, 0.700, 0.240), "min"),
}
PHRASE_ROI = (0.36, 0.11, 0.64, 0.235)  # centred phase title, fractions of the 16:9 rect


# ------------------------------------------------------------------ output

def in_git_tree(p: Path) -> bool:
    p = p.resolve()
    for d in [p, *p.parents]:
        if (d / ".git").exists():
            return True
    return False


def prepare_out(out: Path) -> Path:
    out = out.expanduser()
    if in_git_tree(out):
        sys.exit(f"refusing to write game-derived packs inside a git work tree: {out}")
    for d in (out, out / "packs", out / "staging"):
        d.mkdir(parents=True, exist_ok=True)
        os.chmod(d, 0o700)
    return out


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def png_bytes(img: Image.Image) -> bytes:
    buf = io.BytesIO()
    img.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def write_pack(out: Path, name: str, version: str, files: dict[str, bytes], source: dict) -> dict:
    """Write staging/<name>/, packs/<name> (ustar), and update the lists."""
    stem = name[:-4] if name.endswith(".tar") else name
    stage = out / "staging" / stem
    if stage.exists():
        for f in sorted(stage.rglob("*"), reverse=True):
            f.unlink() if f.is_file() else f.rmdir()
    stage.mkdir(parents=True, exist_ok=True)
    os.chmod(stage, 0o700)
    entries = []
    for path in sorted(files):
        parts = path.split("/")
        assert len(parts) == 1 or (len(parts) == 2 and parts[0] == "special"), path
        data = files[path]
        dest = stage / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)
        os.chmod(dest, 0o600)
        entries.append({"path": path, "sha256": sha256(data), "size": len(data)})
    manifest = {"name": name, "version": version, "files": entries, "source": source}
    mbytes = json.dumps(manifest, indent=1, ensure_ascii=False).encode()
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.USTAR_FORMAT) as tf:
        for path, data in [("manifest.json", mbytes)] + [(p, files[p]) for p in sorted(files)]:
            ti = tarfile.TarInfo(path)
            ti.size, ti.mtime, ti.mode, ti.uid, ti.gid = len(data), 0, 0o644, 0, 0
            ti.uname = ti.gname = ""
            tf.addfile(ti, io.BytesIO(data))
    tar = buf.getvalue()
    (out / "packs" / name).write_bytes(tar)
    os.chmod(out / "packs" / name, 0o600)
    entry = {"name": name, "version": version, "sha256": sha256(tar), "size": len(tar)}
    lst_p = out / "packs" / "manifest.json"
    lst = json.loads(lst_p.read_text()) if lst_p.exists() else []
    lst = [e for e in lst if e["name"] != name] + [entry]
    lst_p.write_text(json.dumps(sorted(lst, key=lambda e: e["name"]), indent=1))
    for fname, val in (("pack-versions.json", version), ("pack-sources.json", source)):
        p = out / fname
        d = json.loads(p.read_text()) if p.exists() else {}
        d[name] = val
        p.write_text(json.dumps(d, indent=1, sort_keys=True, ensure_ascii=False))
        os.chmod(p, 0o600)
    print(json.dumps({**entry, "files": len(files)}))
    return entry


# ------------------------------------------------------------------ geometry (ports)

def rust_round(x: float) -> int:
    return int(np.floor(x + 0.5)) if x >= 0 else -int(np.floor(-x + 0.5))


def game_rect_16_9(w: int, h: int):
    """ocr/preprocess.rs game_rect_16_9."""
    t = 16 / 9
    if h == 0:
        return 0, 0, w, h
    a = w / h
    if abs(a - t) < 0.01:
        return 0, 0, w, h
    if a > t:
        gw = rust_round(h * t)
        return (w - gw) // 2, 0, gw, h
    gh = rust_round(w / t)
    return 0, (h - gh) // 2, w, gh


def map_label_box(w: int, h: int):
    """preprocess.rs map_name_rect: the debug/mapcrops box."""
    gx, gy, gw, gh = game_rect_16_9(w, h)
    return gx + int(gw * 0.68), gy + int(gh * 0.022), int(gw * 0.27), int(gh * 0.040)


def gray_resize(a: np.ndarray, w: int, h: int) -> np.ndarray:
    """banner.rs Gray::resize (triangle filter, support grows when shrinking)."""
    w, h = max(1, w), max(1, h)
    if a.shape == (h, w):
        return a.copy()
    im = Image.fromarray(a.astype(np.float32), mode="F")
    return np.asarray(im.resize((w, h), Image.BILINEAR), dtype=np.float32).clip(0, 1)


@functools.lru_cache(maxsize=256)
def area_matrix(src: int, dst: int) -> np.ndarray:
    """heroes.rs area_weights as a (dst, src) matrix (OpenCV INTER_AREA)."""
    scale = src / dst
    o = np.arange(dst)[:, None]
    i = np.arange(src)[None, :]
    lo = np.maximum(o * scale, i)
    hi = np.minimum((o + 1) * scale, i + 1)
    return (np.clip(hi - lo, 0, None) / scale).astype(np.float32)


def area_resize(plane: np.ndarray, w: int, h: int) -> np.ndarray:
    return area_matrix(plane.shape[0], h) @ plane.astype(np.float32) @ area_matrix(plane.shape[1], w).T


# ------------------------------------------------------------------ banner (port of shadow/banner.rs)

def load_maps(banner_rs: Path):
    """[(key, name, [banner spellings])] parsed from banner.rs MAPS."""
    text = banner_rs.read_text()
    out = []
    for m in re.finditer(r'MapInfo \{\s*key: "([^"]+)",\s*name: "([^"]+)",\s*banner: &\[([^\]]*)\]', text):
        out.append((m.group(1), m.group(2), re.findall(r'"([^"]+)"', m.group(3))))
    if len(out) < 50:
        sys.exit(f"could not parse MAPS from {banner_rs}")
    return out


def default_banner_rs() -> Path:
    return Path(__file__).resolve().parent.parent / "src" / "shadow" / "banner.rs"


def locate_full(rgb: np.ndarray):
    """banner.rs locate(), plus the internals the atlas cutter needs."""
    h, w = rgb.shape[:2]
    gx, gy, gw, gh = game_rect_16_9(w, h)
    y0 = gy + int(0.026 * gh)
    y1 = min(gy + int(0.068 * gh), h)
    x0 = gx + int(0.55 * gw)
    x1 = min(gx + int(0.99 * gw), w)
    if y1 <= y0 + 4 or x1 <= x0 + 4:
        return None
    band = rgb[y0:y1, x0:x1].min(axis=2).astype(np.float32)
    bh, bw = band.shape
    bg = float(np.percentile(band, 30.0))
    pk = float(np.percentile(band, 99.7))
    if pk - bg < 50.0:
        return None
    ink = band > bg + 0.5 * (pk - bg)
    col = ink.any(axis=0)
    runs = []
    x = 0
    while x < bw:
        if col[x]:
            s = x
            while x < bw and col[x]:
                x += 1
            sub = ink[:, s:x]
            rows = np.nonzero(sub.any(axis=1))[0]
            runs.append(dict(s=s, e=x, t=int(rows[0]), b=int(rows[-1]) + 1, fill=int(sub.sum(axis=0).max())))
        else:
            x += 1
    best = None
    for i, r in enumerate(runs):
        hh = r["b"] - r["t"]
        thin = (r["e"] - r["s"]) <= max(0.0022 * gw, 4.0)
        if not (thin and 0.0150 * gh <= hh <= 0.026 * gh and r["fill"] >= 0.85 * hh):
            continue
        nb = [q["b"] for q in runs[i + 1:min(i + 6, len(runs))]]
        if nb and r["b"] - float(np.median(nb)) < 0.08 * hh:
            continue
        cap = 0.68 * hh
        name, prev_e = [], r["e"]
        for q in runs[i + 1:]:
            gap = q["s"] - prev_e
            if gap > (1.6 * cap if not name else 0.8 * cap):
                break
            if q["t"] < r["t"] - 0.35 * hh or q["b"] > r["b"] + 0.35 * hh:
                break
            name.append(q)
            prev_e = q["e"]
        if len(name) >= 2:
            best = (r, name, hh)
    if best is None:
        return None
    bar, name, hh = best
    wt = max(0, bar["t"] - round(0.15 * hh))
    wb = min(bar["b"] + round(0.12 * hh), bh)
    xs, xe = name[0]["s"], name[-1]["e"]
    norm = ((band - bg) / (pk - bg)).clip(0, 1)
    s = H / (wb - wt)
    sw = max(1, round((xe - xs) * s))
    strip = gray_resize(norm[wt:wb, xs:xe], sw, H)
    return dict(strip=strip, norm=norm, wt=wt, wb=wb, xs=xs, xe=xe, s=s, runs=name, hh=hh,
                window=(x0 + xs, y0 + wt, x0 + xe, y0 + wb))


def frame_or_labelbox(img: Image.Image) -> np.ndarray:
    """An RGB frame for locate(). A debug/mapcrops label box (no frame around
    it) is pasted back at its place in a 1440p or 1080p frame filled with the
    box's background colour."""
    rgb = np.asarray(img.convert("RGB"))
    h, w = rgb.shape[:2]
    for fw, fh in ((2560, 1440), (1920, 1080), (3840, 2160)):
        bx, by, bw, bh = map_label_box(fw, fh)
        if (w, h) == (bw, bh):
            fill = np.median(rgb.reshape(-1, 3), axis=0).astype(np.uint8)
            canvas = np.empty((fh, fw, 3), np.uint8)
            canvas[:] = fill
            canvas[by:by + bh, bx:bx + bw] = rgb
            return canvas
    return rgb


class Tpl:
    def __init__(self, parts, wd):
        self.parts, self.wd = parts, wd

    def render(self, offs):
        out = np.zeros((H, self.wd), np.float32)
        for (px, g), off in zip(self.parts, offs):
            ix = px + off
            s0, e = max(ix, 0), min(ix + g.shape[1], self.wd)
            if e > s0:
                out[:, s0:e] = np.maximum(out[:, s0:e], g[:, s0 - ix:e - ix])
        return out

    def score(self, strip: np.ndarray) -> float:
        wo, wt = strip.shape[1], self.wd
        if self.wd == 0 or abs(wo / wt - 1.0) > MAX_RATIO or not self.parts:
            return -1.0
        o = gray_resize(strip, self.wd, H).astype(np.float64)
        d, k, n = ELASTIC, 2 * ELASTIC + 1, len(self.parts)
        loc = np.full((n, k), -1.0)
        for i, (px, g) in enumerate(self.parts):
            g = g.astype(np.float64)
            gz = g - g.mean()
            gn = np.sqrt((gz * gz).sum()) or 1.0
            for j, off in enumerate(range(-d, d + 1)):
                s0 = px + off
                if s0 < 0 or s0 + g.shape[1] > self.wd:
                    continue
                win = o[:, s0:s0 + g.shape[1]]
                wz = win - win.mean()
                wn = np.sqrt((wz * wz).sum()) or 1.0
                loc[i, j] = (gz * wz).sum() / (gn * wn)
        best = loc[0].copy()
        back = np.zeros((n, k), int)
        for i in range(1, n):
            nb = np.full(k, -1e9)
            for j in range(k):
                for kk in (j - 1, j, j + 1):
                    if 0 <= kk < k:
                        v = best[kk] - STEP_COST * abs(j - kk)
                        if v > nb[j]:
                            nb[j], back[i, j] = v, kk
            best = nb + loc[i]
        j = int(np.argmax(best))
        path = [j]
        for i in range(n - 1, 0, -1):
            j = back[i, j]
            path.append(j)
        offs = [p - d for p in reversed(path)]
        return ncc(o, self.render(offs).astype(np.float64))


def ncc(a, b) -> float:
    a = a - a.mean()
    b = b - b.mean()
    den = np.sqrt((a * a).sum() * (b * b).sum())
    return float((a * b).sum() / den) if den > 0 else 0.0


BASE = {**{c: "A" for c in "ÁÀÂÃÄ"}, **{c: "E" for c in "ÉÈÊË"}, **{c: "I" for c in "ÍÌÎÏ"},
        **{c: "O" for c in "ÓÒÔÕÖ"}, **{c: "U" for c in "ÚÙÛÜ"}, "Ç": "C", "Ñ": "N", "’": "'"}


def compose(glyphs: dict, gap: float, space: float, text: str):
    parts, x = [], 0.0
    for c in text:
        if c == " ":
            x += space - gap
            continue
        g = glyphs.get(c)
        if g is None:
            g = glyphs.get(BASE.get(c, c))
        if g is None:
            return None
        parts.append((round(x), g))
        x += g.shape[1] + gap
    return Tpl(parts, max(0, round(x - gap)))


class MapTemplates:
    def __init__(self, glyphs, gap, space, maps):
        self.maps = maps
        self.tpls, self.unrenderable = [], []
        for mi, (_, _, banners) in enumerate(maps):
            for v in banners:
                t = compose(glyphs, gap, space, v)
                (self.tpls.append((mi, v, t)) if t else self.unrenderable.append(v))

    @classmethod
    def load_dir(cls, d: Path, maps):
        a = json.loads((d / "atlas.json").read_text())
        if a["height"] != H:
            return None
        glyphs = {}
        for k, f in a["glyphs"].items():
            g = np.asarray(Image.open(d / f).convert("L"), np.float32) / 255.0
            if g.shape[0] == H and g.shape[1] > 0:
                glyphs[k[0]] = g
        return cls(glyphs, a["gap"], a["space"], maps) if glyphs else None

    def read_strip(self, strip):
        """(best map key, score, margin, sure), banner.rs read_strip."""
        best = {}
        for mi, _, t in self.tpls:
            best[mi] = max(best.get(mi, -np.inf), t.score(strip))
        order = sorted(best, key=lambda i: -best[i])
        s1 = best[order[0]] if order else -1.0
        s2 = best[order[1]] if len(order) > 1 else -1.0
        sure = s1 >= 0.80 and s1 - s2 >= 0.06
        top = self.maps[order[0]][0] if order and s1 > 0 else None
        return top, max(s1, 0.0), max(s1 - s2, 0.0), sure


# ------------------------------------------------------------------ heroes

def hero_key(name: str) -> str:
    s = unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode().lower()
    s = s.replace(".", "").replace(":", "").replace("'", "")
    return re.sub(r"[^a-z0-9]+", "-", s).strip("-")


def downscale_rgba(a: np.ndarray, n: int) -> np.ndarray:
    """Premultiplied area downscale to n x n; colour bleeds into alpha 0."""
    f = a.astype(np.float32) / 255.0
    al = f[..., 3]
    pre = [area_resize(f[..., c] * al, n, n) for c in range(3)]
    ao = area_resize(al, n, n)
    rgb = np.zeros((n, n, 3), np.float32)
    have = ao > 1e-4
    for c in range(3):
        rgb[..., c][have] = pre[c][have] / ao[have]
    # bleed: fill alpha-0 pixels from their filled neighbours, ring by ring
    filled = have.copy()
    for _ in range(n):
        if filled.all():
            break
        acc = np.zeros_like(rgb)
        cnt = np.zeros((n, n), np.float32)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                sf = np.roll(np.roll(filled, dy, 0), dx, 1)
                sr = np.roll(np.roll(rgb, dy, 0), dx, 1)
                acc += sr * sf[..., None]
                cnt += sf
        new = (~filled) & (cnt > 0)
        rgb[new] = acc[new] / cnt[new][:, None]
        filled |= new
    out = np.dstack([rgb, ao]).clip(0, 1)
    return np.round(out * 255).astype(np.uint8)


def cmd_heroes(a):
    out = prepare_out(Path(a.out))
    heroes = json.loads(Path(a.heroes_json).read_text())
    items = heroes.values() if isinstance(heroes, dict) else heroes
    icons = Path(a.icons)
    keys_ok = None
    if a.keys_from:
        keys_ok = {p.stem for p in Path(a.keys_from).glob("*.png")}
    files, missing, seen = {}, [], set()
    for h in items:
        if not h.get("IsHero"):
            continue
        key = hero_key(h["Name"])
        tex = next((i["TextureGUID"] for i in h.get("Images", []) if i["Id"] == a.slot), None)
        cands = [d for d in icons.iterdir() if d.is_dir() and hero_key(d.name.replace("_", ":")) == key]
        src = None
        if tex and cands:
            p = cands[0] / (tex.split(".")[0] + ".png")
            src = p if p.is_file() else None
        if src is None:
            missing.append(key)
            continue
        img = np.asarray(Image.open(src).convert("RGBA"))
        small = downscale_rgba(img, a.size)
        files[f"{key}.png"] = png_bytes(Image.fromarray(small, "RGBA"))
        seen.add(key)
    if a.special:
        for p in sorted(Path(a.special).glob("*.png")):
            files[f"special/{p.name}"] = p.read_bytes()
    extra = sorted(seen - keys_ok) if keys_ok is not None else []
    lacking = sorted(keys_ok - seen) if keys_ok is not None else []
    source = {"datatool_build": a.build, "slot": a.slot, "size_px": a.size,
              "heroes": len(seen), "special_from": "heroes-v1" if a.special else None}
    write_pack(out, a.name, a.version or f"{a.build}.1", files, source)
    print(json.dumps({"heroes": len(seen), "missing_portrait": missing,
                      "not_in_v1": extra, "v1_not_built": lacking}))


# ------------------------------------------------------------------ fonts
# Font files stay on the build machine; only rendered masks go into a pack.

def render_text(font_path, text, size, xscale=1.0, skew=0.0, track=0.0, ss=4):
    """White-on-black render of one line, 0..1, supersampled. Returns
    (array, cap_top, baseline) in output pixels. `size` is the font size in
    px, `track` extra advance per glyph as a share of the size, `skew` the
    shear (x shift per px of height above the baseline), `xscale` a
    horizontal stretch."""
    from PIL import ImageDraw, ImageFont
    f = ImageFont.truetype(str(font_path), size * ss)
    asc, desc = f.getmetrics()
    pad = int(0.4 * size * ss) + 4
    xs = []
    for i, c in enumerate(text):
        xs.append(f.getlength(text[:i]) + i * track * size * ss)
    width = int(f.getlength(text) + len(text) * track * size * ss + 2 * pad)
    height = asc + desc + 2 * pad
    im = Image.new("L", (width, height), 0)
    d = ImageDraw.Draw(im)
    for c, x in zip(text, xs):
        d.text((pad + x, pad), c, font=f, fill=255)
    base = pad + asc
    hb = f.getbbox("H")
    cap_top = pad + hb[1]
    # x_out = xscale * x + skew * (base - y); PIL wants the inverse map
    off = -min(skew * base, skew * (base - height))
    w2 = int(width * xscale + abs(skew) * height) + 2
    im = im.transform((w2, height), Image.AFFINE,
                      (1 / xscale, skew / xscale, -(off + skew * base) / xscale, 0, 1, 0),
                      resample=Image.BICUBIC)
    a = np.asarray(im, np.float32) / 255.0
    ys, xs_ = np.nonzero(a > 0.02)
    if len(ys) == 0:
        return None, 0, 0
    a = a[:, xs_.min():xs_.max() + 1]
    out = area_resize(a, max(1, round(a.shape[1] / ss)), max(1, round(a.shape[0] / ss))).clip(0, 1)
    return out, cap_top / ss, base / ss


def ink_rows(g, thr=0.5):
    ys = np.nonzero((g > thr * g.max()).any(axis=1))[0]
    return (int(ys[0]), int(ys[-1]) + 1) if len(ys) else (0, g.shape[0])


def render_glyph_like(font_path, c, cap_top, cap_bot, xscale, level):
    """One atlas glyph: cap height and top row as measured on cut glyphs,
    cropped to its >50% ink columns like a banner run."""
    cap = cap_bot - cap_top
    size = 64
    probe, ct, cb = render_text(font_path, "H", size)
    k = cap / (cb - ct)
    g, ct, cb = render_text(font_path, c, max(4, round(size * k)), xscale=xscale)
    if g is None:
        return None
    top = round(cap_top - ct)
    out = np.zeros((H, g.shape[1]), np.float32)
    y0, y1 = max(0, top), min(H, top + g.shape[0])
    out[y0:y1] = g[y0 - top:y1 - top]
    cols = np.nonzero((out > 0.5).any(axis=0))[0]
    if len(cols) == 0:
        return None
    return out[:, cols[0]:cols[-1] + 1] * level


def fill_from_font(glyphs: dict, need, font_path):
    """Render the glyphs of `need` the atlas lacks, fitted to the cut ones. Returns
    (new glyphs, report)."""
    ref = [c for c in "HETLIN" if c in glyphs]
    rows = [ink_rows(glyphs[c]) for c in ref]
    cap_top = float(np.median([r[0] for r in rows]))
    cap_bot = float(np.median([r[1] for r in rows]))
    level = float(np.median([np.percentile(g, 99) for g in glyphs.values()]))
    common = [c for c in glyphs if c.isascii() and c.isalnum()]
    ratios = []
    for c in common:
        g = render_glyph_like(font_path, c, cap_top, cap_bot, 1.0, level)
        if g is not None:
            ratios.append(glyphs[c].shape[1] / g.shape[1])
    xscale = float(np.median(ratios)) if ratios else 1.0
    fits = []
    for c in common:
        g = render_glyph_like(font_path, c, cap_top, cap_bot, xscale, level)
        if g is not None:
            fits.append(ncc(gray_resize(g, glyphs[c].shape[1], H), glyphs[c]))
    new = {}
    for c in need:
        if c not in glyphs:
            g = render_glyph_like(font_path, c, cap_top, cap_bot, xscale, level)
            if g is not None:
                new[c] = g
    rep = {"xscale": round(xscale, 3), "cap_rows": [cap_top, cap_bot],
           "rendered_vs_cut_ncc_median": round(float(np.median(fits)), 3) if fits else None,
           "rendered_vs_cut_ncc_min": round(float(np.min(fits)), 3) if fits else None,
           "rendered": sorted(new)}
    return new, rep


# ------------------------------------------------------------------ maps

NARROW = {"I": 0.30, "'": 0.18, ":": 0.18, "J": 0.45}
WIDE = {"M": 0.95, "W": 1.05}


def _partition(widths, run_ws, gap):
    """Split chars (estimated widths) into len(run_ws) contiguous groups that
    best fit the run widths. Returns group sizes or None."""
    n, m = len(widths), len(run_ws)
    inf = float("inf")
    best = [[inf] * (n + 1) for _ in range(m + 1)]
    back = [[0] * (n + 1) for _ in range(m + 1)]
    best[0][0] = 0.0
    for r in range(1, m + 1):
        for j in range(r, n + 1):
            for i in range(r - 1, j):
                if best[r - 1][i] == inf:
                    continue
                est = sum(widths[i:j]) + gap * (j - i - 1)
                v = best[r - 1][i] + abs(run_ws[r - 1] - est) / max(run_ws[r - 1], 1.0)
                if v < best[r][j]:
                    best[r][j], back[r][j] = v, i
    if best[m][n] == inf:
        return None
    sizes, j = [], n
    for r in range(m, 0, -1):
        i = back[r][j]
        sizes.append(j - i)
        j = i
    return sizes[::-1]


def _segments(loc, text, widths):
    """Native column spans, one per non-space char, or None."""
    runs, s = loc["runs"], loc["s"]
    chars = [c for c in text if c != " "]
    if len(runs) == len(chars):
        return [(r["s"], r["e"]) for r in runs], True
    if len(runs) > len(chars):
        return None
    est = [widths.get(c, H * (NARROW.get(BASE.get(c, c)) or WIDE.get(BASE.get(c, c)) or 0.62)) / s
           for c in chars]
    sizes = _partition(est, [r["e"] - r["s"] for r in runs], 2.0 / s)
    if sizes is None:
        return None
    ink = (loc["norm"][loc["wt"]:loc["wb"]] > 0.5).sum(axis=0)
    spans, k = [], 0
    for r, n in zip(runs, sizes):
        if n == 1:
            spans.append((r["s"], r["e"]))
            k += 1
            continue
        part = est[k:k + n]
        tot = sum(part)
        cuts, acc = [r["s"]], 0.0
        for w in part[:-1]:
            acc += w
            x = r["s"] + round(acc / tot * (r["e"] - r["s"]))
            lo, hi = max(r["s"] + 1, x - 3), min(r["e"] - 1, x + 3)
            if hi > lo:
                x = lo + int(np.argmin(ink[lo:hi + 1]))
            cuts.append(x)
        cuts.append(r["e"])
        spans += list(zip(cuts[:-1], cuts[1:]))
        k += n
    if any(b - a < 2 for a, b in spans):
        return None
    return spans, False


def cut_instances(lab_path: Path):
    """{char: [(glyph, clean)]}, in-word gaps, space gaps, counts.
    Pass 1 keeps banners with one ink run per glyph; pass 2 splits touching
    glyphs (kerned pairs such as TO, AV) at the faintest column near the
    width the clean glyphs predict."""
    inst, gaps, spaces = {}, [], []
    stats = {"images": 0, "located": 0, "segmented": 0, "split": 0}
    jobs = []
    for line in lab_path.read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        path, text = line.split("	")[:2]
        stats["images"] += 1
        loc = locate_full(frame_or_labelbox(Image.open(path)))
        if loc is None:
            continue
        stats["located"] += 1
        jobs.append((loc, text.split("|")))
    widths = {}
    for clean_pass in (True, False):
        for loc, alts in jobs:
            got = None
            for text in alts:
                seg = _segments(loc, text, widths)
                if seg and seg[1] == clean_pass:
                    got = (text, seg[0])
                    break
            if got is None:
                continue
            text, spans = got
            stats["segmented" if clean_pass else "split"] += 1
            s, wt, wb = loc["s"], loc["wt"], loc["wb"]
            chars = [c for c in text if c != " "]
            word_end, k = set(), 0
            for c in text:
                if c == " ":
                    word_end.add(k - 1)
                else:
                    k += 1
            for i, (c, (a, b)) in enumerate(zip(chars, spans)):
                gw = max(1, round((b - a) * s))
                inst.setdefault(c, []).append((gray_resize(loc["norm"][wt:wb, a:b], gw, H), clean_pass))
                if clean_pass and i + 1 < len(spans):
                    gap = (spans[i + 1][0] - b) * s
                    (spaces if i in word_end else gaps).append(gap)
        if clean_pass:
            widths = {c: float(np.median([g.shape[1] for g, _ in v])) for c, v in inst.items()}
    return inst, gaps, spaces, stats


def merge_glyph(gs: list[np.ndarray]) -> np.ndarray:
    w = int(np.median([g.shape[1] for g in gs]))
    stack = np.stack([gray_resize(g, w, H) for g in gs])
    return np.median(stack, axis=0) if len(gs) >= 3 else stack.mean(axis=0)


def build_atlas(lab_path: Path, maps):
    inst, gaps, spaces, stats = cut_instances(lab_path)
    glyphs = {}
    for c, v in inst.items():
        clean = [g for g, ok in v if ok]
        glyphs[c] = merge_glyph(clean if len(clean) >= 2 else [g for g, _ in v])
    gap = float(np.median(gaps)) if gaps else 2.0
    space = float(np.median(spaces)) if spaces else 3 * gap
    counts = {c: len(v) for c, v in inst.items()}
    return glyphs, round(gap, 2), round(space, 2), counts, stats


def glyph_file(c: str) -> str:
    return f"g{ord(c):04x}.png"


def cmd_maps(a):
    out = prepare_out(Path(a.out))
    maps = load_maps(Path(a.banner_rs) if a.banner_rs else default_banner_rs())
    glyphs, gap, space, counts, stats = build_atlas(Path(a.labels), maps)
    need = sorted({BASE.get(c, c) for _, _, bs in maps for b in bs for c in b if c != " "})
    fill = {"rendered": []}
    if a.fill_font:
        new, fill = fill_from_font(glyphs, need, Path(a.fill_font))
        glyphs.update(new)
    files = {glyph_file(c): png_bytes(Image.fromarray(np.round(g * 255).astype(np.uint8), "L"))
             for c, g in glyphs.items()}
    files["atlas.json"] = json.dumps({"height": H, "gap": gap, "space": space,
                                      "glyphs": {c: glyph_file(c) for c in sorted(glyphs)},
                                      "rendered": fill["rendered"], "validated": False if fill["rendered"] else True},
                                     ensure_ascii=False, indent=1).encode()
    mt = MapTemplates(glyphs, gap, space, maps)
    source = {"from": "Tab banner captures", **stats, "glyphs": len(glyphs), "font_fill": fill,
              "templates": len(mt.tpls), "unrenderable": len(mt.unrenderable)}
    write_pack(out, a.name, a.version, files, source)
    print(json.dumps({"font_fill": fill, "stats": stats, "gap": gap, "space": space, "instances": counts,
                      "missing_glyphs": [c for c in need if c not in glyphs],
                      "unrenderable": mt.unrenderable}, ensure_ascii=False))


# ------------------------------------------------------------------ words

def roi_plane(rgb: np.ndarray, roi, plane: str) -> np.ndarray:
    """result.rs region(): plane of the ROI, 0..1, at the 720p work scale."""
    h, w = rgb.shape[:2]
    gx, gy, gw, gh = game_rect_16_9(w, h)
    x0, y0 = gx + int(roi[0] * gw), gy + int(roi[1] * gh)
    x1, y1 = min(gx + int(roi[2] * gw), w), min(gy + int(roi[3] * gh), h)
    sub = rgb[y0:y1, x0:x1]
    g = (sub.max(axis=2) if plane == "max" else sub.min(axis=2)).astype(np.float32) / 255.0
    s = WORK * 1440.0 / gh
    return gray_resize(g, round(g.shape[1] * s), round(g.shape[0] * s))


def ink_groups(p: np.ndarray, frac=0.5, split=0.35, rows=None):
    """Column groups of bright ink: [(x0, x1, y0, y1)] left to right."""
    bg, pk = np.percentile(p, 30), np.percentile(p, 99.5)
    ink = p > bg + frac * (pk - bg)
    if rows is not None:
        ink[: rows[0]] = False
        ink[rows[1]:] = False
    ys = np.nonzero(ink.sum(axis=1) >= 2)[0]
    if len(ys) == 0:
        return []
    hgt = ys[-1] - ys[0] + 1
    col = ink.sum(axis=0) >= 1
    groups, x, n = [], 0, len(col)
    while x < n:
        if col[x]:
            s = x
            while x < n and col[x]:
                x += 1
            if groups and s - groups[-1][1] <= split * hgt:
                groups[-1][1] = x
            else:
                groups.append([s, x])
        else:
            x += 1
    out = []
    for s, e in groups:
        yy = np.nonzero(ink[:, s:e].any(axis=1))[0]
        out.append((s, e, int(yy[0]), int(yy[-1]) + 1))
    return out


def text_band(p: np.ndarray, which: str, frac=0.6):
    """Row band (y0, y1) of the lowest ('last') or highest ('first') text line."""
    bg, pk = np.percentile(p, 30), np.percentile(p, 99.5)
    prof = (p > bg + frac * (pk - bg)).sum(axis=1)
    on = prof >= max(3, 0.02 * p.shape[1])
    bands, y = [], 0
    while y < len(on):
        if on[y]:
            s = y
            while y < len(on) and on[y]:
                y += 1
            if y - s >= 4:
                bands.append((s, y))
        else:
            y += 1
    if not bands:
        return None
    if which == "tallest":
        return max(bands, key=lambda b: b[1] - b[0])
    return bands[-1] if which == "last" else bands[0]


def cut_box(p: np.ndarray, box, pad=2) -> np.ndarray:
    x0, x1, y0, y1 = box
    return p[max(0, y0 - pad):y1 + pad, max(0, x0 - pad):x1 + pad]


def best_offset(p: np.ndarray, t: np.ndarray, window=None):
    """(score, y, x) of the best zero-mean NCC position of t in p (the
    result.rs best_ncc score). Window sums come from integral images, the
    cross term from an FFT, so memory stays at a few image sizes.
    `window` = (y0, y1, x0, x1) limits the positions searched."""
    th, tw = t.shape
    ph, pw = p.shape
    if th > ph or tw > pw:
        return (-2.0, 0, 0)
    p = p.astype(np.float64)
    tz = t.astype(np.float64) - t.mean()
    tn = np.sqrt((tz * tz).sum())
    if tn == 0:
        return (-2.0, 0, 0)
    fy, fx = ph + th, pw + tw
    num = np.fft.irfft2(np.fft.rfft2(p, (fy, fx)) * np.conj(np.fft.rfft2(tz, (fy, fx))), (fy, fx))
    num = num[:ph - th + 1, :pw - tw + 1]
    s1 = np.pad(p, ((1, 0), (1, 0))).cumsum(0).cumsum(1)
    s2 = np.pad(p * p, ((1, 0), (1, 0))).cumsum(0).cumsum(1)
    rect = lambda s: s[th:, tw:] - s[:-th, tw:] - s[th:, :-tw] + s[:-th, :-tw]
    sm, sq = rect(s1), rect(s2)
    var = sq - sm * sm / (th * tw)
    sc = np.where(var > 1e-9, num / (np.sqrt(np.maximum(var, 1e-12)) * tn), -2.0)
    if window is not None:
        y0, y1, x0, x1 = window
        m = np.full_like(sc, -2.0)
        m[max(0, y0):y1, max(0, x0):x1] = sc[max(0, y0):y1, max(0, x0):x1]
        sc = m
    y, x = np.unravel_index(int(np.argmax(sc)), sc.shape)
    return float(sc[y, x]), int(y), int(x)


def word_patch(p: np.ndarray, layout: str, words: int = 1):
    """The word (or first `words` words) of a result / phrase region."""
    if layout == "rank_screen":
        band = text_band(p, "last", 0.7)
        if band is None:
            return None
        gs = ink_groups(p, 0.7, 0.35, rows=band)
    elif layout == "accolade":
        # the coloured word is dimmer than the white map name beside it, and
        # voice/party overlays can sit above or below it: take the first tall,
        # wide group
        gs = ink_groups(p, 0.3, 0.30)
        if not gs:
            return None
        mh = max(g[3] - g[2] for g in gs)
        tall = [g for g in gs if g[3] - g[2] >= 0.75 * mh]
        wide = [g for g in tall if g[1] - g[0] >= 1.5 * (g[3] - g[2])]
        gs = wide[:1] or sorted(tall, key=lambda g: g[0] - g[1])[:1]
        # rows: the tallest ink band inside the word's columns (drops an
        # overlay line that touches the word from above or below)
        x0, x1 = gs[0][0], gs[0][1]
        band = text_band(p[:, x0:x1], "tallest", 0.3)
        if band is None:
            return None
        sub = ink_groups(p, 0.3, 0.30, rows=band)
        sub = [g for g in sub if g[1] > x0 and g[0] < x1]
        if not sub:
            return None
        gs = [(min(g[0] for g in sub), max(g[1] for g in sub), band[0], band[1])]
    else:  # phrase: the tallest text line, its first `words` words
        return phrase_patch(p, words)
    if len(gs) < words:
        return None
    sel = gs[:words]
    box = (sel[0][0], sel[-1][1], min(g[2] for g in sel), max(g[3] for g in sel))
    return cut_box(p, box)


def line_patch(p: np.ndarray, frac: float = 0.5):
    """The widest bright group of the tallest text line (one big word)."""
    band = text_band(p, "tallest", frac)
    if band is None:
        return None
    gs = ink_groups(p, frac, 0.6, rows=band)
    if not gs:
        return None
    g = max(gs, key=lambda g: g[1] - g[0])
    return cut_box(p, (g[0], g[1], band[0], band[1]))


def phrase_patch(p: np.ndarray, words: int, skew: float = 0.16, frac: float = 0.55):
    """First `words` words of an italic title line. Word gaps are found with
    the slant taken out (x' = x + skew * y, y down from the line top); pixels right of the
    gap after the last kept word are set to the background, so the counter
    after the phrase never enters the mask."""
    band = text_band(p, "tallest", frac)
    if band is None:
        return None
    y0, y1 = band
    bg, pk = np.percentile(p, 30), np.percentile(p, 99.5)
    ink = p[y0:y1] > bg + frac * (pk - bg)
    hgt = y1 - y0
    shift = np.round(skew * np.arange(hgt)).astype(int)  # lower rows move right
    wd = p.shape[1] + int(shift.max()) + 1
    sh = np.zeros((hgt, wd), bool)
    for y in range(hgt):
        sh[y, shift[y]:shift[y] + p.shape[1]] = ink[y]
    col = sh.any(axis=0)
    xs = np.nonzero(col)[0]
    if len(xs) == 0:
        return None
    groups, x = [], xs[0]
    while x < wd:
        if col[x]:
            st = x
            while x < wd and col[x]:
                x += 1
            if groups and st - groups[-1][1] <= 0.22 * hgt:
                groups[-1][1] = x
            else:
                groups.append([st, x])
        else:
            x += 1
    # drop specks and a mouse cursor: a word spans most of the line height
    tall = []
    for g0, g1 in groups:
        rows = np.nonzero(sh[:, g0:g1].any(axis=1))[0]
        if rows[-1] - rows[0] + 1 >= 0.6 * hgt:
            tall.append([g0, g1])
    groups = tall
    if not groups:
        return None
    # short words of an italic line can touch ("A PREFERRED"): with fewer
    # groups than words the whole line is the phrase (the dim counter is
    # below the ink threshold); with more, cut after the last word
    words = min(words, len(groups))
    cut = groups[words - 1][1] + (groups[words][0] - groups[words - 1][1]) // 2 if len(groups) > words else wd
    sub = p[y0:y1].copy()
    for y in range(hgt):
        lim = cut - shift[y]  # original x of the slanted cut on this row
        if lim < sub.shape[1]:
            sub[y, max(lim, 0):] = bg
    x0 = max(0, groups[0][0] - int(shift.max()))
    x1 = min(p.shape[1], groups[words - 1][1])
    keep = (sub[:, x0:x1] > bg + frac * (pk - bg)).any(axis=0)
    cols = np.nonzero(keep)[0]
    if len(cols) == 0:
        return None
    x0, x1 = x0 + cols[0], x0 + cols[-1] + 1
    pad = 2
    full = np.full((hgt + 2 * pad, x1 - x0 + 2 * pad), bg, np.float32)
    ya, yb = max(0, y0 - pad), min(p.shape[0], y1 + pad)
    blk = p[ya:yb, max(0, x0 - pad):x1 + pad].copy()
    blk[y0 - ya:y0 - ya + hgt, (x0 - max(0, x0 - pad)):(x0 - max(0, x0 - pad)) + (x1 - x0)] = sub[:, x0:x1]
    # mask the right pad too where the slanted cut passes through it
    full[:blk.shape[0], :blk.shape[1]] = blk
    return full


def merge_patches(patches: list[np.ndarray]) -> np.ndarray:
    """Pixelwise median of patches aligned by NCC. Patches whose size is off
    the median (a word cut together with an overlay) are left out, so
    nothing but the word survives the median."""
    mw = float(np.median([q.shape[1] for q in patches]))
    mh = float(np.median([q.shape[0] for q in patches]))
    keep = [q for q in patches if abs(q.shape[1] - mw) <= 0.03 * mw + 1 and abs(q.shape[0] - mh) <= 0.10 * mh + 1]
    patches = keep or patches
    ref = patches[0]
    th, tw = ref.shape
    stack = [ref]
    for q in patches[1:]:
        padq = np.pad(q, 4, mode="edge")
        if padq.shape[0] < th or padq.shape[1] < tw:
            padq = np.pad(padq, ((0, max(0, th - padq.shape[0])), (0, max(0, tw - padq.shape[1]))), mode="edge")
        _, y, x = best_offset(padq, ref)
        stack.append(padq[y:y + th, x:x + tw])
    return np.median(np.stack(stack), axis=0)


@functools.lru_cache(maxsize=256)
def _probe(font_path, text, xscale, track, skew):
    return render_text(font_path, text, 100, xscale, track, skew)


@functools.lru_cache(maxsize=512)
def _render_word(font_path, text, height, xscale, track, skew, pad):
    return _render_word_uncached(font_path, text, height, xscale, track, skew, pad)


def render_word(font_path, text, height, xscale=1.0, track=0.0, skew=0.0, pad=2):
    return _render_word(str(font_path), text, float(height), float(xscale), float(track), float(skew), pad).copy()


def _render_word_uncached(font_path, text, height, xscale=1.0, track=0.0, skew=0.0, pad=2):
    """Rendered word mask at the work scale: ink `height` px tall (cap height
    for a word in capitals, the `!` included), `pad` px of black around it."""
    probe, _, _ = _probe(str(font_path), text, xscale, track, skew)
    y0, y1 = ink_rows(probe, 0.1)
    k = height / max(1, y1 - y0)
    g, _, _ = render_text(font_path, text, max(4, round(100 * k)), xscale, track, skew)
    y0, y1 = ink_rows(g, 0.1)
    g = g[y0:y1]
    g = gray_resize(g, max(1, round(g.shape[1] * height / g.shape[0])), int(round(height)))
    return np.pad(g, pad)


def cmd_words(a):
    """Spec TSV lines (one per line, tab separated):
        result<TAB>image<TAB>accolade|rank_screen<TAB>victory|defeat|draw
        phrase<TAB>image<TAB>phrase-id<TAB>PHRASE TEXT
        line<TAB>image<TAB>file.png<TAB>roi=x0:y0:x1:y1;plane=min;layout=ID;word=W
        render<TAB>font<TAB>file.png<TAB>TEXT<TAB>height=H;xscale=X;track=T;skew=K;keep_rows=N;layout=ID;word=W;roi=..;fit=NOTE
        layout<TAB>id<TAB>x0,y0,x1,y1<TAB>max|min
    `result`, `phrase` and `line` lines add one frame each (`line`: the
    biggest word of the tallest text line in `roi`); the frames of one template
    are merged by a pixelwise median after alignment. `render` lines draw a
    word in a game font (the font file stays on the build machine; only the
    mask ships) and are marked rendered, not validated. `layout` lines go to
    layouts.json (extra or replaced result.rs regions). words.json lists
    every template with its provenance."""
    out = prepare_out(Path(a.out))
    groups, skipped, seen = {}, 0, set()
    phrases, meta, layouts, rendered = {}, {}, [], {}
    for line in Path(a.spec).read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        kind, path, ident, word = (line.split("\t") + ["", ""])[:4]
        if kind == "layout":
            layouts.append({"id": path, "roi": [float(v) for v in ident.split(",")], "plane": word})
            continue
        if kind == "line":
            opts = dict(kv.split("=", 1) for kv in word.split(";") if kv)
            roi = tuple(float(v) for v in opts["roi"].split(":"))
            rgb = np.asarray(Image.open(path).convert("RGB"))
            digest = sha256(rgb.tobytes())
            if (digest, ident) in seen:
                continue
            seen.add((digest, ident))
            p = line_patch(roi_plane(rgb, roi, opts.get("plane", "min")))
            meta.setdefault(ident, {"kind": "cut", "rendered": False, "validated": True,
                                    "layout": opts.get("layout"), "word": opts.get("word"),
                                    "roi": list(roi), "plane": opts.get("plane", "min")})
            if p is None or p.size == 0:
                skipped += 1
                continue
            groups.setdefault(ident, []).append(p)
            continue
        if kind == "render":
            raw = line.split("\t")[4]
            opts = dict(kv.split("=", 1) for kv in raw.split(";" if ";" in raw else ",") if kv)
            t = render_word(Path(path), word, float(opts["height"]), float(opts.get("xscale", 1)),
                            float(opts.get("track", 0)), float(opts.get("skew", 0)))
            if "keep_rows" in opts:  # a word that fades out downwards: keep its top rows
                t = t[: int(opts["keep_rows"])]
            rendered[ident] = t
            from PIL import ImageFont
            fam = " ".join(ImageFont.truetype(path, 10).getname())
            meta[ident] = {"kind": "rendered", "rendered": True, "validated": False, "text": word,
                           **({"roi": [float(v) for v in opts["roi"].split(":")], "plane": opts.get("plane", "max")}
                              if "roi" in opts else {}),
                           "font": fam, "layout": opts.get("layout"), "word": opts.get("word"),
                           "height": float(opts["height"]), "xscale": float(opts.get("xscale", 1)),
                           "track": float(opts.get("track", 0)), "skew": float(opts.get("skew", 0)),
                           "fit": opts.get("fit", "assumed")}
            continue
        rgb = np.asarray(Image.open(path).convert("RGB"))
        digest = sha256(rgb.tobytes())
        if (digest, ident, word) in seen:
            continue
        seen.add((digest, ident, word))
        if kind == "result":
            roi, plane = RESULT_LAYOUTS[ident]
            p = word_patch(roi_plane(rgb, roi, plane), ident)
            key = f"{ident}_{word}.png"
            meta.setdefault(key, {"kind": "cut", "rendered": False, "validated": True,
                                  "layout": ident, "word": word})
        else:
            p = word_patch(roi_plane(rgb, PHRASE_ROI, "min"), "phrase", len(word.split()))
            key = f"phrase_{ident}.png"
            phrases[ident] = {"text": word, "roi": list(PHRASE_ROI), "plane": "min", "work": WORK}
            meta.setdefault(key, {"kind": "cut", "rendered": False, "validated": False,
                                  "phrase": ident, "text": word})
        if p is None or p.size == 0:
            skipped += 1
            continue
        groups.setdefault(key, []).append(p)
    files, counts = {}, {}
    for key, ps in sorted(groups.items()):
        t = merge_patches(ps)
        files[key] = png_bytes(Image.fromarray(np.round(t.clip(0, 1) * 255).astype(np.uint8), "L"))
        counts[key] = len(ps)
        meta[key]["frames"] = len(ps)
    for key, t in rendered.items():
        files[key] = png_bytes(Image.fromarray(np.round(t.clip(0, 1) * 255).astype(np.uint8), "L"))
    for ident in phrases:
        phrases[ident]["frames"] = counts.get(f"phrase_{ident}.png", 0)
    if phrases:
        files["phrases.json"] = json.dumps(phrases, indent=1).encode()
    if layouts:
        files["layouts.json"] = json.dumps(layouts, indent=1).encode()
    files["words.json"] = json.dumps({k: meta[k] for k in sorted(meta) if k in files}, indent=1,
                                     ensure_ascii=False).encode()
    source = {"from": "member result / phase captures + game-font renders",
              "frames_per_template": counts, "skipped": skipped,
              "rendered": sorted(rendered), "rendered_validated": False}
    write_pack(out, a.name, a.version, files, source)
    print(json.dumps({"templates": counts, "rendered": sorted(rendered), "skipped": skipped}))


# ------------------------------------------------------------------ main

def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    h = sub.add_parser("heroes", help="heroes-v2 from DataTool slot 0x40C9")
    h.add_argument("--icons", required=True, help="DataTool extract-hero-icons HeroIcons dir")
    h.add_argument("--heroes-json", required=True, help="DataTool list-heroes --json output")
    h.add_argument("--special", help="special/ dir to copy (placeholder, skull, empty)")
    h.add_argument("--keys-from", help="template dir whose *.png stems are the expected hero keys")
    h.add_argument("--build", type=int, required=True, help="game build the extract came from")
    h.add_argument("--slot", default=PORTRAIT_SLOT, help="list-heroes image slot id (default: the scoreboard portrait)")
    h.add_argument("--size", type=int, default=74)
    h.add_argument("--name", default="heroes-v2.tar")
    h.add_argument("--version")
    h.add_argument("--out", required=True)
    m = sub.add_parser("maps", help="maps-v1 banner glyph atlas")
    m.add_argument("--labels", required=True, help="TSV: image<TAB>banner spelling")
    m.add_argument("--banner-rs", help="banner.rs for the MAPS table (default: this checkout)")
    m.add_argument("--fill-font", help="font file to render glyphs the captures lack (build machine only)")
    m.add_argument("--name", default="maps-v1.tar")
    m.add_argument("--version", default="1")
    m.add_argument("--out", required=True)
    w = sub.add_parser("words", help="words-v1 result words + phrase masks")
    w.add_argument("--spec", required=True, help="TSV, see the words docstring")
    w.add_argument("--name", default="words-v1.tar")
    w.add_argument("--version", default="1")
    w.add_argument("--out", required=True)
    a = ap.parse_args(argv)
    {"heroes": cmd_heroes, "maps": cmd_maps, "words": cmd_words}[a.cmd](a)


if __name__ == "__main__":
    main()
