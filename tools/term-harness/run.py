#!/usr/bin/env python3
"""Drive poclimon in a pty with a scripted session and record raw output.

usage: run.py <binary> <protocol> <out.bin> [--cols N --rows N]
Writes <out.bin> (raw bytes) and <out.bin>.times.json ([t, offset] marks per action).
"""
import fcntl, json, os, pty, select, shutil, signal, struct, sys, termios, time, argparse

ap = argparse.ArgumentParser()
ap.add_argument("binary"); ap.add_argument("protocol"); ap.add_argument("out")
ap.add_argument("--cols", type=int, default=160); ap.add_argument("--rows", type=int, default=45)
ap.add_argument("--script", default="full")
ap.add_argument("--home", default=None)
ap.add_argument("--no-dump", action="store_true", help="disable the frame dump (for CPU/perf numbers)")
a = ap.parse_args()

home = a.home or os.path.join(os.path.dirname(os.path.abspath(a.out)), "home")
os.makedirs(os.path.join(home, ".config"), exist_ok=True)
with open(os.path.join(home, ".config", "poclimon.toml"), "w") as f:
    f.write('[display]\nscale = 3\n\n[roster]\ncreatures = ["pikachu", "eevee", "bulbasaur", "charmander", "squirtle", "jolteon"]\n')

def setsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

pid, fd = pty.fork()
if pid == 0:
    env = dict(os.environ, HOME=home, POCLIMON_FRAME_DUMP="" if a.no_dump else os.path.abspath(a.out) + ".frames.jsonl", TERM="xterm-256color", POCLIMON_PROTOCOL=a.protocol,
               COLORTERM="truecolor")
    os.execve(a.binary, [a.binary], env)

setsize(fd, a.rows, a.cols)
os.kill(pid, signal.SIGWINCH)
out = bytearray(); marks = []; t0 = time.time()
stats = {"peak_rss_kb": 0, "cpu_s": 0.0}
readlog = []; last_stack = [0.0]
import subprocess
def maybe_stack():
    # If no output for >1.2s, capture thread stacks once per stall.
    if readlog and time.time()-t0 - readlog[-1][0] > 1.2 and time.time()-t0 - last_stack[0] > 3:
        last_stack[0] = time.time()-t0
        r = subprocess.run(["gdb","-p",str(pid),"-batch","-ex","thread apply all bt 12"],capture_output=True,text=True,timeout=30)
        stacks.append((round(time.time()-t0,2), r.stdout[-6000:]))
stacks = []
answered = [0]
def answer_queries():
    if os.environ.get("NOANSWER"):
        return
    # Behave like a real terminal: answer the capability query ratatui-image sends.
    if answered[0] < 0:
        return
    i = out.find(b"\x1b[5n", max(0, len(out) - 70000))
    if i < 0:
        return
    answered[0] = -1
    resp = b""
    if a.protocol == "kitty":
        resp += b"\x1b_Gi=31;OK\x1b\\"
        if b"i=32" in out[max(0, len(out) - 70000):]:
            resp += b"\x1b_Gi=32;OK\x1b\\"
    resp += b"\x1b[6;20;10t"                       # cell size 10x20 px
    resp += b"\x1b[?62;4;22c" if a.protocol == "sixel" else b"\x1b[?62;22c"
    resp += b"\x1b[0n"
    os.write(fd, resp)
def sample():
    try:
        st = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
        stats["cpu_s"] = (int(st[11]) + int(st[12])) / os.sysconf("SC_CLK_TCK")
        for line in open(f"/proc/{pid}/status"):
            if line.startswith("VmRSS"):
                stats["peak_rss_kb"] = max(stats["peak_rss_kb"], int(line.split()[1]))
    except Exception:
        pass

def pump(secs):
    end = time.time() + secs
    while time.time() < end:
        sample(); maybe_stack()
        r, _, _ = select.select([fd], [], [], 0.02)
        if r:
            try:
                data = os.read(fd, 1 << 16)
            except OSError:
                return False
            if not data:
                return False
            out.extend(data)
            answer_queries()
            readlog.append((round(time.time()-t0,3), len(out)))
    return True

def key(k, label=None, wait=0.6):
    marks.append([round(time.time() - t0, 3), len(out), label or repr(k)])
    os.write(fd, k.encode() if isinstance(k, str) else k)
    pump(wait)

RIGHT = "\x1b[C"
SCRIPTS = {
    "full": [
        ("wait", 8.0, "startup+splash+sprite loads"),
        ("key", "e", "feed #1"), ("wait", 3.0, None),
        ("key", RIGHT, "select #2"), ("key", "p", "play #2"), ("wait", 4.0, None),
        ("key", RIGHT, "select #3"), ("key", "s", "sleep #3"), ("wait", 3.0, None),
        ("key", "3", None), ("key", "p", "play #3"), ("key", "4", None), ("key", "p", "play #4"),
        ("wait", 4.0, "3 playing"),
        ("key", "r", "release #4"), ("wait", 3.0, None),
        ("key", "a", "add prompt"), ("key", "1", None), ("key", "4", None), ("key", "4", None),
        ("key", "\r", "add #144"), ("wait", 6.0, None),
        ("key", "1", None), ("key", "\t", "swap prompt"), ("key", "1", None), ("key", "3", None),
        ("key", "6", None), ("key", "\r", "swap #1 -> 136"), ("wait", 7.0, None),
        ("rapid", 20, "20 rapid selection changes"), ("wait", 2.0, None),
        ("resize", (38, 120), "resize to 120x38"), ("wait", 4.0, None),
        ("resize", (45, 160), "resize back"), ("wait", 4.0, None),
        ("wait", 20.0, "idle soak 20s"),
    ],
    "soak": [("wait", 8.0, "startup"), ("key", "e", "feed"), ("key", RIGHT, None), ("key", "p", "play"),
             ("wait", 60.0, "soak 60s")],
}
for step in SCRIPTS[a.script]:
    kind, arg, label = step
    if kind == "wait":
        marks.append([round(time.time() - t0, 3), len(out), label or "wait"]); pump(arg)
    elif kind == "key":
        key(arg, label)
    elif kind == "rapid":
        marks.append([round(time.time() - t0, 3), len(out), label])
        for i in range(arg):
            os.write(fd, RIGHT.encode()); pump(0.05)
    elif kind == "resize":
        marks.append([round(time.time() - t0, 3), len(out), label])
        setsize(fd, *arg); os.kill(pid, signal.SIGWINCH); pump(0.1)

sample(); marks.append([round(time.time() - t0, 3), len(out), "quit"])
os.write(fd, b"q"); pump(2.0)
try:
    wpid, status = os.waitpid(pid, os.WNOHANG)
    if wpid == 0:
        os.kill(pid, signal.SIGKILL); os.waitpid(pid, 0); marks.append([0, len(out), "KILLED (did not exit)"])
except ChildProcessError:
    pass
open(a.out, "wb").write(out)
json.dump(marks, open(a.out + ".times.json", "w"))
json.dump(stats, open(a.out + ".stats.json", "w"))
json.dump(readlog, open(a.out + ".reads.json", "w"))
json.dump(stacks, open(a.out + ".stacks.json", "w"))
print(stats)
print(f"{a.protocol}: {len(out)/1e6:.2f} MB in {time.time()-t0:.1f}s")
