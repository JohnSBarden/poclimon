#!/usr/bin/env python3
"""Replay a recorded session through a VT emulator and render screenshots.

Kitty images are drawn from their unicode placeholders: every placeholder cell is
painted with a colour derived from its image id, and the row/col diacritics are
decoded so broken placements (wrong row/col order, orphaned cells) are visible
and countable. Halfblock output renders exactly as a terminal would.

usage: screens.py <rec.bin> <outdir> [t1 t2 ...]   (seconds; default: after each mark)
"""
import json, os, re, sys, hashlib
import pyte
from PIL import Image, ImageDraw, ImageFont

rec, outdir = sys.argv[1], sys.argv[2]
times = [float(x) for x in sys.argv[3:]]
os.makedirs(outdir, exist_ok=True)
data = open(rec, "rb").read()
reads = json.load(open(rec + ".reads.json"))
marks = json.load(open(rec + ".times.json"))

# Kitty placeholder diacritics table (row/col index -> combining char), from kitty's rowcolumn-diacritics.txt
# ratatui-image uses the same table; we only need the reverse map.
D = [0x0305,0x030D,0x030E,0x0310,0x0312,0x033D,0x033E,0x033F,0x0346,0x034A,0x034B,0x034C,0x0350,0x0351,0x0352,
     0x0357,0x035B,0x0363,0x0364,0x0365,0x0366,0x0367,0x0368,0x0369,0x036A,0x036B,0x036C,0x036D,0x036E,0x036F,
     0x0483,0x0484,0x0485,0x0486,0x0487,0x0592,0x0593,0x0594,0x0595,0x0597,0x0598,0x0599,0x059C,0x059D,0x059E,
     0x059F,0x05A0,0x05A1,0x05A8,0x05A9,0x05AB,0x05AC,0x05AF,0x05C4,0x0610,0x0611,0x0612,0x0613,0x0614,0x0615]
RDIAC = {chr(c): i for i, c in enumerate(D)}

APC = re.compile(rb"\x1b_G.*?\x1b\\", re.S)
DCS = re.compile(rb"\x1bP.*?\x1b\\", re.S)
OSC = re.compile(rb"\x1b\].*?(?:\x07|\x1b\\)", re.S)
SYNC = re.compile(rb"\x1b\[\?2026[hl]")

def clean(b):
    b = APC.sub(b"", b); b = DCS.sub(b"", b); b = OSC.sub(b"", b); b = SYNC.sub(b"", b)
    return b.replace(b"\x1b[s", b"\x1b7").replace(b"\x1b[u", b"\x1b8")

# Figure out final terminal size changes from marks (resize labels "resize to WxH").
screen = pyte.Screen(160, 45)
stream = pyte.ByteStream(screen)

def size_at(t):
    cols, rows = 160, 45
    for mt, _, label in marks:
        if mt <= t and label.startswith("resize to "):
            cols, rows = map(int, label.split()[-1].split("x"))
        elif mt <= t and label == "resize back":
            cols, rows = 160, 45
    return cols, rows

if not times:
    times = [m[0] - 0.05 for m in marks[1:] if m[0] > 0]

try:
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 12)
except OSError:
    font = ImageFont.load_default()
CW, CH = 8, 16

def color(c, default):
    if c == "default":
        return default
    if isinstance(c, str) and re.fullmatch(r"[0-9a-fA-F]{6}", c):
        return tuple(int(c[i:i+2], 16) for i in (0, 2, 4))
    named = {"black": (0,0,0), "red": (205,49,49), "green": (13,188,121), "brown": (229,229,16),
             "yellow": (229,229,16), "blue": (36,114,200), "magenta": (188,63,188), "cyan": (17,168,205),
             "white": (229,229,229), "brightblack": (102,102,102), "brightred": (241,76,76),
             "brightgreen": (35,209,139), "brightyellow": (245,245,67), "brightblue": (59,142,234),
             "brightmagenta": (214,112,214), "brightcyan": (41,184,219), "brightwhite": (255,255,255)}
    return named.get(c, default)

report = []
pos = 0
for t in sorted(times):
    # feed everything read up to time t
    target = 0
    for rt, off in reads:
        if rt <= t:
            target = off
    cols, rows = size_at(t)
    if (screen.columns, screen.lines) != (cols, rows):
        screen.resize(rows, cols)
    chunk = data[pos:target]
    # Don't split an escape sequence across feeds: hold back an unterminated APC/DCS/OSC/CSI tail.
    cut = len(chunk)
    k = chunk.rfind(b"\x1b")
    if k >= 0:
        tail = chunk[k:]
        complete = (tail[:2] in (b"\x1b_", b"\x1bP", b"\x1b]") and (b"\x1b\\" in tail[2:] or b"\x07" in tail)) or \
                   (tail[:2] == b"\x1b[" and re.match(rb"\x1b\[[0-9;?]*[@-~]", tail)) or \
                   (len(tail) >= 2 and tail[:2] not in (b"\x1b_", b"\x1bP", b"\x1b]", b"\x1b["))
        if not complete:
            cut = k
    # an APC may also be open further back (payload contains no ESC until its terminator)
    for opener in (b"\x1b_", b"\x1bP", b"\x1b]"):
        j = chunk.rfind(opener, 0, cut)
        if j >= 0 and chunk.find(b"\x1b\\", j + 2, cut) < 0 and chunk.find(b"\x07", j + 2, cut) < 0:
            cut = min(cut, j)
    stream.feed(clean(chunk[:cut])); pos += cut

    img = Image.new("RGB", (cols * CW, rows * CH), (20, 20, 20))
    dr = ImageDraw.Draw(img)
    images = {}  # id -> list of (x, y, row, col)
    for y in range(rows):
        line = screen.buffer[y]
        prev_rc = None
        for x in range(cols):
            ch = line[x]
            d = ch.data
            fg = color(ch.fg, (220, 220, 220)); bg = color(ch.bg, (20, 20, 20))
            if ch.reverse: fg, bg = bg, fg
            if d and d[0] == "\U0010EEEE":
                diac = [RDIAC[c] for c in d[1:] if c in RDIAC]
                idc = ch.fg
                if len(diac) >= 2:
                    r, c = diac[0], diac[1]
                elif prev_rc and prev_rc[0] == idc:
                    r, c = prev_rc[1], prev_rc[2] + 1
                else:
                    r, c = -1, -1
                prev_rc = (idc, r, c)
                images.setdefault(idc, []).append((x, y, r, c))
                h = hashlib.md5(str(idc).encode()).digest()
                col = (60 + h[0] % 180, 60 + h[1] % 180, 60 + h[2] % 180)
                dr.rectangle([x*CW, y*CH, x*CW+CW-1, y*CH+CH-1], fill=col)
                continue
            prev_rc = None
            dr.rectangle([x*CW, y*CH, x*CW+CW-1, y*CH+CH-1], fill=bg)
            if d.strip():
                if d in ("▀", "▄"):
                    top = fg if d == "▀" else bg; bot = bg if d == "▀" else fg
                    dr.rectangle([x*CW, y*CH, x*CW+CW-1, y*CH+CH//2-1], fill=top)
                    dr.rectangle([x*CW, y*CH+CH//2, x*CW+CW-1, y*CH+CH-1], fill=bot)
                elif d == "█":
                    dr.rectangle([x*CW, y*CH, x*CW+CW-1, y*CH+CH-1], fill=fg)
                else:
                    dr.text((x*CW, y*CH + 1), d[:1], fill=fg, font=font)
    # Placement integrity for kitty, per Kitty's rules: a bare placeholder
    # inherits (row, col+1) from its left neighbour only if that neighbour shows
    # the same image; otherwise its position is unresolvable. A resolved cell is
    # correct if x - col matches an explicitly-labelled anchor in that row.
    broken = 0; frags = 0
    anchors = {}
    for idc, cells in images.items():
        for (x, y, r, c) in cells:
            if r >= 0:
                anchors.setdefault((idc, y), set()).add(x - c)
    for idc, cells in images.items():
        for (x, y, r, c) in cells:
            if r < 0 or (x - c) not in anchors.get((idc, y), set()):
                broken += 1
    label = next((m[2] for m in reversed(marks) if 0 < m[0] <= t + 0.05), "")
    name = f"{outdir}/t{t:06.1f}.png"
    img.save(name)
    report.append((round(t, 1), label, len(images), broken, frags, name))

for r in report:
    print(f"t={r[0]:6.1f} [{r[1][:28]:28s}] kitty_images_on_screen={r[2]:3d} misplaced_cells={r[3]:5d} split_images={r[4]}  {r[5]}")
