#!/usr/bin/env python3
"""Per-phase throughput + graphics-sequence accounting for a recorded session."""
import json, re, sys
from collections import Counter

KITTY = re.compile(rb"\x1b_G([^;\x1b]*)(?:;[^\x1b]*)?\x1b\\")
SIXEL = re.compile(rb"\x1bP[0-9;]*q.*?\x1b\\", re.S)
ITERM = re.compile(rb"\x1b\]1337;File=.*?(?:\x07|\x1b\\)", re.S)

for path in sys.argv[1:]:
    data = open(path, "rb").read()
    marks = json.load(open(path + ".times.json"))
    print(f"\n=== {path}  total {len(data)/1e6:.2f} MB")
    for (t, off, label), (t2, off2, _) in zip(marks, marks[1:]):
        dt = max(t2 - t, 1e-3)
        chunk = data[off:off2]
        k = KITTY.findall(chunk)
        transmits = sum(1 for c in k if b"a=T" in c or b"a=t" in c)
        six = SIXEL.findall(chunk); it = ITERM.findall(chunk)
        gfx = sum(map(len, six)) + sum(map(len, it)) + sum(len(m.group(0)) for m in KITTY.finditer(chunk))
        print(f"{t:6.1f}s {label[:34]:34s} {len(chunk)/dt/1e3:8.0f} KB/s  gfx {gfx/max(len(chunk),1)*100:4.0f}%"
              f"  kitty_tx={transmits:4d} sixel={len(six):4d} iterm={len(it):4d}")
