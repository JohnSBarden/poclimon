# term-harness

Headless reproduction and measurement of how PoCLImon renders in a terminal.
Runs the real binary in a pseudo-terminal that answers the graphics
capability query like a real terminal, drives a scripted session (feed, play,
sleep, release, add, swap, rapid selection, resize, idle soak), and records
every byte the app writes.

Requires Python 3 with `pyte` and `Pillow` (`pip install pyte pillow`).

```bash
cargo build --release
cd tools/term-harness

# Record a session for each graphics protocol (forces the protocol via POCLIMON_PROTOCOL)
for p in kitty sixel iterm2 halfblocks; do
  python3 run.py ../../target/release/poclimon $p out/$p.bin
done

python3 phases.py out/*.bin              # throughput per phase, image sends per phase
python3 kitty_check.py out/kitty.bin     # Kitty images placed but never uploaded (should be 0)
python3 screens.py out/kitty.bin shots/kitty 10 30 60   # screenshots at t=10s,30s,60s
python3 stale.py out/kitty.bin           # intended frame vs what the terminal shows (should be 0)
```

`stale.py` needs the frame dump, which `run.py` turns on automatically via
`POCLIMON_FRAME_DUMP` (see `src/devtools.rs`). It works for the text-grid
protocols (Kitty placeholders and halfblocks); Sixel/iTerm2 pixels can't be
inspected by a text emulator.

Each run also writes `<out>.stats.json` (peak RSS, CPU seconds) and
`<out>.reads.json` (timestamped output offsets — gaps > 0.25s are render-loop
stalls).

Set `NOANSWER=1` to simulate a terminal that never answers the capability
query.
