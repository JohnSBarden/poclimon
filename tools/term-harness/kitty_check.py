import re, sys
from screens_diac import R
TOK = re.compile(r'\x1b_G(?P<apc>[^\x1b]*)\x1b\\|\x1b\[(?P<csi>[0-9;?]*)(?P<fin>[@-~])|\x1b.|(?P<ph>\U0010EEEE)(?P<dia>[̀-￿]*)', re.S)
def sgr_fg(params, fg):
    p = [int(x) if x else 0 for x in params.split(';')] if params else [0]
    i = 0
    while i < len(p):
        v = p[i]
        if v == 0 or v == 39: fg = None
        elif v == 38 and i + 1 < len(p) and p[i+1] == 2: fg = tuple(p[i+2:i+5]); i += 4
        elif v == 38 and i + 1 < len(p) and p[i+1] == 5: fg = ('idx', p[i+2]); i += 2
        elif v == 48 and i + 1 < len(p) and p[i+1] == 2: i += 4
        elif v == 48 and i + 1 < len(p) and p[i+1] == 5: i += 2
        elif 30 <= v <= 37 or 90 <= v <= 97: fg = ('idx', v)
        i += 1
    return fg
for path in sys.argv[1:]:
    s = open(path, 'rb').read().decode('utf-8', 'replace')
    fg = None; tx = set(); placed = {}; never_first = {}
    for m in TOK.finditer(s):
        if m.group('apc') is not None:
            a = m.group('apc'); mm = re.search(r'\bi=(\d+),a=T', a)
            if mm: tx.add(int(mm.group(1)))
        elif m.group('fin') == 'm':
            fg = sgr_fg(m.group('csi'), fg)
        elif m.group('ph'):
            dia = [R[c] for c in m.group('dia') if c in R]
            if len(dia) >= 2 and isinstance(fg, tuple) and len(fg) == 3 and fg[0] != 'idx':
                extra = dia[2] if len(dia) >= 3 else 0
                i = (extra << 24) | (fg[0] << 16) | (fg[1] << 8) | fg[2]
                placed[i] = placed.get(i, 0) + 1
                if i not in tx: never_first.setdefault(i, 0); never_first[i] += 1
    never = [i for i in placed if i not in tx]
    print(f"{path}: transmitted={len(tx)} placed={len(placed)} never-uploaded={len(never)} "
          f"(row placements {sum(placed[i] for i in never)}) placed-before-first-upload={len(never_first)}")
