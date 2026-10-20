#!/usr/bin/env python3
"""Generate assets/lumen-icons.ttf and src/icon.rs from the geometry below.

All icons are original Lumen artwork drawn on a 16x16 grid (y down) from
strokes and filled shapes. Run with fontTools installed:

    python3 -m venv /tmp/v && /tmp/v/bin/pip install fonttools
    /tmp/v/bin/python tools/gen_icons.py
"""
import math, pathlib
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

ROOT = pathlib.Path(__file__).resolve().parent.parent
S = 1.3  # default stroke width
UNIT = 64
BASE = 14  # grid y that sits on the baseline


class G:
    def __init__(self):
        self.polys = []  # (points, is_cut)

    def poly(self, pts, cut=False):
        self.polys.append(([(float(x), float(y)) for x, y in pts], cut))

    def seg(self, a, b, w=S, cut=False):
        (x1, y1), (x2, y2) = a, b
        dx, dy = x2 - x1, y2 - y1
        n = math.hypot(dx, dy) or 1
        nx, ny = -dy / n * w / 2, dx / n * w / 2
        self.poly([(x1 + nx, y1 + ny), (x2 + nx, y2 + ny), (x2 - nx, y2 - ny), (x1 - nx, y1 - ny)], cut)

    def circ(self, c, r, cut=False, n=28):
        self.poly([(c[0] + r * math.cos(2 * math.pi * i / n), c[1] + r * math.sin(2 * math.pi * i / n)) for i in range(n)], cut)

    def ring(self, c, r, w=S):
        self.circ(c, r + w / 2)
        self.circ(c, r - w / 2, cut=True)

    def line(self, pts, w=S, closed=False):
        pts = list(pts)
        pairs = list(zip(pts, pts[1:] + ([pts[0]] if closed else [])))
        for a, b in pairs:
            self.seg(a, b, w)
        joints = pts if closed else pts[1:-1]
        for p in joints:
            self.circ(p, w / 2, n=12)

    def arc(self, c, r, a0, a1, n=14):
        return [(c[0] + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)),
                 c[1] + r * math.sin(math.radians(a0 + (a1 - a0) * i / n))) for i in range(n + 1)]

    def rect(self, x, y, w, h, cut=False):
        self.poly([(x, y), (x + w, y), (x + w, y + h), (x, y + h)], cut)


def area(pts):
    return sum(x1 * y2 - x2 * y1 for (x1, y1), (x2, y2) in zip(pts, pts[1:] + pts[:1])) / 2


def mirror(pts):
    return [(16 - x, y) for x, y in pts]


icons = {}


def icon(name, cfg=None):
    def deco(fn):
        g = G()
        fn(g)
        icons[name] = (g, cfg)
        return fn
    return deco


FILE_OUTLINE = [(3.5, 1.5), (9.5, 1.5), (12.5, 4.5), (12.5, 14.5), (3.5, 14.5)]


def file_base(g):
    g.line(FILE_OUTLINE, closed=True)
    g.line([(9.5, 1.5), (9.5, 4.5), (12.5, 4.5)])


@icon("FILE")
def _(g): file_base(g)


@icon("FILE_PLUS")
def _(g):
    file_base(g)
    g.seg((8, 7.8), (8, 12.2)); g.seg((5.8, 10), (10.2, 10))


@icon("FILE_CODE")
def _(g):
    file_base(g)
    g.line([(6.6, 8), (5, 10), (6.6, 12)]); g.line([(9.4, 8), (11, 10), (9.4, 12)])


@icon("FILE_ARROW_UP")
def _(g):
    file_base(g)
    g.seg((8, 12.5), (8, 8)); g.line([(6, 9.8), (8, 7.8), (10, 9.8)])


@icon("FILES")
def _(g):
    g.line([(2.5, 5.5), (2.5, 14.5), (9.5, 14.5), (9.5, 5.5)], closed=True)
    g.line([(5.5, 5.5), (5.5, 1.5), (10.5, 1.5), (13.5, 4.5), (13.5, 11.5), (9.5, 11.5)])
    g.line([(10.5, 1.5), (10.5, 4.5), (13.5, 4.5)])


@icon("COPY")
def _(g):
    g.line([(5.5, 5.5), (13.5, 5.5), (13.5, 14.5), (5.5, 14.5)], closed=True)
    g.line([(5.5, 11.5), (2.5, 11.5), (2.5, 1.5), (10.5, 1.5), (10.5, 5.5)])


@icon("FLOPPY_DISK")
def _(g):
    g.line([(2, 2), (12, 2), (14, 4), (14, 14), (2, 14)], closed=True)
    g.line([(5, 2), (5, 6), (11, 6), (11, 2)])
    g.line([(4.5, 14), (4.5, 9.5), (11.5, 9.5), (11.5, 14)])


@icon("FOLDER")
def _(g):
    g.line([(1.5, 3), (6, 3), (7.5, 5), (14.5, 5), (14.5, 13), (1.5, 13)], closed=True)


@icon("FOLDER_OPEN")
def _(g):
    g.line([(1.5, 12.5), (1.5, 3), (6, 3), (7.5, 4.8), (12.5, 4.8), (12.5, 7)])
    g.line([(1.5, 12.5), (3.5, 7), (15, 7), (13, 12.5)], closed=True)


@icon("NEW_FOLDER")
def _(g):
    g.line([(7, 13), (1.5, 13), (1.5, 3), (6, 3), (7.5, 5), (14.5, 5), (14.5, 7.5)])
    g.seg((12, 9), (12, 14)); g.seg((9.5, 11.5), (14.5, 11.5))


@icon("ELLIPSIS")
def _(g):
    for x in (3.5, 8, 12.5):
        g.circ((x, 8), 1.3)


@icon("CARET_DOWN")
def _(g): g.line([(4, 6), (8, 10), (12, 6)], 1.5)


@icon("CARET_RIGHT")
def _(g): g.line([(6, 4), (10, 8), (6, 12)], 1.5)


@icon("CHECK")
def _(g): g.line([(2.5, 8.5), (6.3, 12), (13.5, 4)], 1.6)


@icon("X")
def _(g):
    g.seg((3.5, 3.5), (12.5, 12.5), 1.5); g.seg((12.5, 3.5), (3.5, 12.5), 1.5)


@icon("PLUS")
def _(g):
    g.seg((8, 2.5), (8, 13.5), 1.5); g.seg((2.5, 8), (13.5, 8), 1.5)


@icon("REFRESH")
def _(g):
    pts = g.arc((8, 8), 5, 40, 340)
    g.line(pts)
    ex, ey = pts[-1]
    g.poly([(ex - 2.6, ey - 0.4), (ex + 2.6, ey - 0.4), (ex, ey - 3.8)])


@icon("DISCARD")
def _(g):
    g.line([(5.5, 3.5), (2.5, 6.5), (5.5, 9.5)])
    g.line([(2.5, 6.5)] + g.arc((9.5, 9.5), 3, -90, 90, 10) + [(6, 12.5)])


def repo(g, up):
    g.line([(3, 1.5), (13, 1.5), (13, 14.5), (3, 14.5)], closed=True)
    if up:
        g.seg((8, 12), (8, 5)); g.line([(5.6, 7.2), (8, 4.8), (10.4, 7.2)])
    else:
        g.seg((8, 4), (8, 11)); g.line([(5.6, 8.8), (8, 11.2), (10.4, 8.8)])


@icon("REPO_PULL")
def _(g): repo(g, False)


@icon("REPO_PUSH")
def _(g): repo(g, True)


@icon("TERMINAL")
def _(g):
    g.line([(1.5, 2.5), (14.5, 2.5), (14.5, 13.5), (1.5, 13.5)], closed=True)
    g.line([(4.5, 5.5), (7, 8), (4.5, 10.5)]); g.seg((8.5, 10.5), (11.5, 10.5))


@icon("HISTORY")
def _(g):
    g.ring((8, 8), 5.8)
    g.line([(8, 4.6), (8, 8), (10.6, 9.6)])


@icon("OUTPUT")
def _(g):
    g.line([(1.5, 2.5), (14.5, 2.5), (14.5, 13.5), (1.5, 13.5)], closed=True)
    g.seg((4.5, 5.5), (11.5, 5.5), 1.1); g.seg((4.5, 8), (11.5, 8), 1.1); g.seg((4.5, 10.5), (8.5, 10.5), 1.1)


@icon("AGENT")
def _(g):
    pts = []
    for i in range(8):
        r = 7 if i % 2 == 0 else 2
        a = math.radians(i * 45 - 90)
        pts.append((8 + r * math.cos(a), 8 + r * math.sin(a)))
    g.poly(pts)


@icon("SEND")
def _(g):
    g.poly([(1.5, 2), (14.5, 8), (1.5, 14), (3.8, 8)])
    g.seg((3.8, 8), (9, 8), 1.0, cut=True)


@icon("STOP")
def _(g): g.rect(3.5, 3.5, 9, 9)


@icon("CLEAR_ALL")
def _(g):
    for y in (3.5, 8, 12.5):
        g.seg((7.5, y), (14.5, y), 1.3)
    g.seg((1.5, 6), (5.5, 10), 1.3); g.seg((5.5, 6), (1.5, 10), 1.3)


@icon("REPLACE_ALL")
def _(g):
    g.seg((2.5, 5), (12.5, 5)); g.line([(10.2, 2.6), (12.8, 5), (10.2, 7.4)])
    g.seg((3.5, 11), (13.5, 11)); g.line([(5.8, 8.6), (3.2, 11), (5.8, 13.4)])


@icon("COLLAPSE_ALL")
def _(g):
    g.line([(2.5, 5.5), (10.5, 5.5), (10.5, 13.5), (2.5, 13.5)], closed=True)
    g.line([(5.5, 5.5), (5.5, 2.5), (13.5, 2.5), (13.5, 10.5), (10.5, 10.5)])
    g.seg((4.5, 9.5), (8.5, 9.5))


@icon("GEAR")
def _(g):
    pts = []
    R, r = 7, 5.2
    for i in range(8):
        a = i * 45
        for da, rad in ((-12, r), (-8, R), (8, R), (12, r)):
            t = math.radians(a + da)
            pts.append((8 + rad * math.cos(t), 8 + rad * math.sin(t)))
    g.poly(pts)
    g.circ((8, 8), 2.4, cut=True)


@icon("GIT_BRANCH")
def _(g):
    g.ring((4.5, 3.5), 1.7, 1.2); g.ring((4.5, 12.5), 1.7, 1.2); g.ring((11.5, 5.5), 1.7, 1.2)
    g.seg((4.5, 5.2), (4.5, 10.8), 1.2)
    g.line([(11.5, 7.2), (11.5, 8.5), (9.5, 10), (4.5, 10.5)], 1.2)


@icon("MAGNIFYING_GLASS")
def _(g):
    g.ring((6.5, 6.5), 4.3, 1.4); g.seg((9.8, 9.8), (14, 14), 1.7)


@icon("PLAY")
def _(g): g.poly([(4.5, 2.5), (13, 8), (4.5, 13.5)])


def panel_frame(g):
    g.line([(2, 3), (14, 3), (14, 13), (2, 13)], closed=True)


@icon("SIDEBAR_LEFT")
def _(g):
    panel_frame(g)
    g.rect(2.5, 3.5, 3.5, 9)
    g.seg((6.5, 3), (6.5, 13))


@icon("SIDEBAR_RIGHT")
def _(g):
    panel_frame(g)
    g.rect(10, 3.5, 3.5, 9)
    g.seg((9.5, 3), (9.5, 13))


@icon("PANEL_BOTTOM")
def _(g):
    panel_frame(g)
    g.rect(2.5, 9, 11, 3.5)
    g.seg((2, 8.5), (14, 8.5))


@icon("ERROR")
def _(g):
    g.circ((8, 8), 6.5)
    g.seg((5.6, 5.6), (10.4, 10.4), 1.3, cut=True); g.seg((10.4, 5.6), (5.6, 10.4), 1.3, cut=True)


@icon("WARNING")
def _(g):
    g.poly([(8, 1.8), (15, 14), (1, 14)])
    g.seg((8, 6), (8, 10), 1.4, cut=True); g.circ((8, 12), 0.9, cut=True)


@icon("DEBUG")
def _(g):
    g.circ((8, 9.2), 3.9); g.circ((8, 4.3), 2.0)
    for sy, ex, ey in ((7, 1.8, 5.2), (9.5, 1.4, 9.5), (12, 2, 13.8)):
        g.seg((4.2, sy), (ex, ey), 1.1); g.seg((11.8, sy), (16 - ex, ey), 1.1)
    g.seg((8, 6.8), (8, 13), 0.9, cut=True)


@icon("BRACKETS_CURLY", cfg='#[cfg(not(any(target_os = "macos", target_os = "windows")))]')
def _(g):
    left = [(6.5, 2.5), (5.3, 2.5), (5.3, 6.8), (3.5, 8), (5.3, 9.2), (5.3, 13.5), (6.5, 13.5)]
    g.line(left, 1.2); g.line(mirror(left), 1.2)


def build():
    names = list(icons)
    codes = {n: 0xE000 + i for i, n in enumerate(names)}
    order = [".notdef"] + [n.lower() for n in names]
    glyphs = {".notdef": TTGlyphPen(None).glyph()}
    for n, (g, _) in icons.items():
        pen = TTGlyphPen(None)
        for pts, cut in g.polys:
            fp = [(round(x * UNIT), round((BASE - y) * UNIT)) for x, y in pts]
            want_cw = not cut  # filled contours clockwise (y up), cuts counter-clockwise
            if (area(fp) < 0) != want_cw:
                fp.reverse()
            pen.moveTo(fp[0])
            for p in fp[1:]:
                pen.lineTo(p)
            pen.closePath()
        glyphs[n.lower()] = pen.glyph()
    fb = FontBuilder(1024, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({c: n.lower() for n, c in codes.items()})
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({g: (1024, 0) for g in order})
    fb.setupHorizontalHeader(ascent=896, descent=-128)
    fb.setupNameTable({"familyName": "Lumen Icons", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=896, sTypoDescender=-128, usWinAscent=896, usWinDescent=128)
    fb.setupPost()
    fb.save(ROOT / "assets/lumen-icons.ttf")

    out = ["//! Lumen's own icon set, drawn in `tools/gen_icons.py` and shipped as\n"
           "//! `assets/lumen-icons.ttf`. Regenerate both with that script; do not edit by hand.\n"]
    for n in names:
        cfg = icons[n][1]
        if cfg:
            out.append(cfg + "\n")
        out.append(f'pub const {n}: &str = "\\u{{{codes[n]:04x}}}";\n')
        if n == "FILE":
            out.append("pub const FILE_TEXT: &str = FILE;\n")
    (ROOT / "src/icon.rs").write_text("".join(out))


build()
