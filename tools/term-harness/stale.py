#!/usr/bin/env python3
"""Compare what the app intended to draw with what the terminal actually shows.

The instrumented build writes its intended frame to <rec>.frames.jsonl and an
OSC 9999 marker into its output right after flushing that frame. We replay the
recording through a VT emulator up to each marker and count cells that differ:
text cells showing the wrong character, and Kitty placeholder cells showing
the wrong image (the image id is carried in the foreground colour).
"""
import json, re, sys
import pyte
from screens_diac import R

PH = "\U0010EEEE"
MARK = re.compile(rb"\x1b\]9999;frame=(\d+)\x07")
APC = re.compile(rb"\x1b_G.*?\x1b\\", re.S)
DCS = re.compile(rb"\x1bP.*?\x1b\\", re.S)
OSC = re.compile(rb"\x1b\].*?(?:\x07|\x1b\\)", re.S)

def clean(b):
    b = APC.sub(b"", b); b = DCS.sub(b"", b); b = OSC.sub(b"", b)
    b = re.sub(rb"\x1b\[\?2026[hl]", b"", b)
    return b.replace(b"\x1b[s", b"\x1b7").replace(b"\x1b[u", b"\x1b8")

def rgb_hex(dbg):
    m = re.match(r"Rgb\((\d+), (\d+), (\d+)\)", dbg)
    return "%02x%02x%02x" % tuple(map(int, m.groups())) if m else None

rec = sys.argv[1]
data = open(rec, "rb").read()
frames = {}
for line in open(rec + ".frames.jsonl"):
    fr = json.loads(line); frames[fr["n"]] = fr

screen = pyte.Screen(160, 45); stream = pyte.ByteStream(screen)
pos = 0; results = []
for m in MARK.finditer(data):
    n = int(m.group(1)); fr = frames.get(n)
    stream.feed(clean(data[pos:m.start()])); pos = m.end()
    if not fr:
        continue
    if (screen.columns, screen.lines) != (fr["w"], fr["h"]):
        continue  # mid-resize; the emulator size isn't tracked here
    txt_bad = ph_bad = txt_n = ph_n = 0
    for x, y, kind, ch, fg in fr["cells"]:
        cell = screen.buffer[y][x]
        if kind == "txt":
            txt_n += 1
            got = cell.data[:1] if cell.data else " "
            want = ch or " "
            if got == PH or (got != want and not (want == " " and got == "")):
                txt_bad += 1
        elif kind == "ph":
            ph_n += 1
            want = rgb_hex(fg)  # None for the old format (colour is inside the escape)
            if not cell.data.startswith(PH) or (want is not None and cell.fg != want):
                ph_bad += 1
    results.append((n, txt_n, txt_bad, ph_n, ph_bad))

if not results:
    print(rec, "no comparable frames"); sys.exit()
frames_with_errors = sum(1 for r in results if r[2] or r[4])
worst = max(results, key=lambda r: r[2] + r[4])
avg_txt = sum(r[2] for r in results) / len(results)
avg_ph = sum(r[4] for r in results) / len(results)
print(f"{rec}: frames checked={len(results)}  frames with wrong cells={frames_with_errors} "
      f"({frames_with_errors*100//len(results)}%)  avg wrong text cells/frame={avg_txt:.1f}  "
      f"avg wrong image cells/frame={avg_ph:.1f}  worst frame #{worst[0]}: {worst[2]} text + {worst[4]} image cells")
