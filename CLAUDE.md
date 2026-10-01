# CLAUDE.md — PoCLImon Developer Context

This file gives Claude Code (and human contributors) the context needed to work on this repo without re-deriving it.

---

## Project Overview

**poclimon** is a Rust TUI idle pet game. Pokémon live in your terminal — they eat, sleep, play, and accumulate XP. Built on [Ratatui](https://ratatui.rs/) and [ratatui-image](https://github.com/benjajaja/ratatui-image), it renders sprites via Kitty graphics, Sixel, iTerm2 inline, or Unicode halfblocks depending on what the terminal supports.

- **Version:** see `Cargo.toml` (bumped automatically by the release workflow)
- **Edition:** Rust 2024 (requires nightly or recent stable that ships 2024 edition support)
- **Binary:** `poclimon` (single binary, no server, no daemon)

---

## Architecture

```
src/
  main.rs          — CLI entry, event loop, key handling and Pokédex-number prompt
  cli.rs           — clap argument definitions
  lib.rs           — library root (exposes modules to integration tests)
  app.rs           — App state, background sprite loading, pen physics/collision tick
  config/mod.rs    — TOML config loading, validation, roster + XP persistence
  creatures.rs     — Pokédex table (1025 entries) and padded-ID helpers
  creature.rs      — Per-creature state machine (idle/eat/sleep/play), XP/level, movement
  animation.rs     — Timing-only frame animation player
  anim_data.rs     — PMDCollab AnimData.xml parser
  sprite_sheet.rs  — Sprite sheet frame extraction and normalization
  sprite_loading.rs— Loads/scales animations into per-direction frame sets
  sprite/mod.rs    — Sprite disk cache + HTTPS downloader (ureq, shared agent)
  sprite/fallback.rs — Generated fallback sprites when downloads fail
  notification.rs  — In-TUI notification messages
  kitty_upload.rs  — Out-of-band Kitty image uploads/deletes + placeholder fixup (see Rendering rules)
  devtools.rs      — POCLIMON_FRAME_DUMP instrumentation used by tools/term-harness
  ui/
    panels.rs      — Layout, title/status/help panels, prompt popup; runs the Kitty fixup last
    pen.rs         — Pen, sprites, nameplates, toy
    splash.rs      — Startup splash screen
    theme.rs       — Game Boy DMG palette
tools/term-harness/ — Headless terminal harness: records sessions per protocol, checks rendering
build.rs           — Pre-renders the title art from assets/poclimon-title.png at compile time
```

**Data flow:**
1. Config loaded from `~/.config/poclimon.toml` (or `--config`)
2. Creatures initialized from config slots; sprites loaded in background threads via `mpsc` channel
3. Main loop: crossterm events → state updates → ratatui render
4. On quit, creature state (XP, level, slot_id) persisted back to TOML config

---

## Rendering rules (read before touching ui/ or sprite encoding)

These are the invariants behind the 2026-09 rendering fixes. Breaking one
brings back a visible bug.

1. **Stay on ratatui ≥ 0.30 / ratatui-image ≥ 11.** Older ratatui-image
   stored a whole image escape sequence in one cell; ratatui 0.29 measured
   that cell as thousands of columns wide and re-sent every image after it
   on every frame (≈120 full image re-sends/s on iTerm2/Sixel), and old
   Kitty rows overwrote cells ratatui thought held other content. 0.30's
   `CellDiffOption::ForcedWidth` fixes the width problem.
2. **Kitty uploads go out-of-band.** ratatui-image attaches an image's
   upload to its first cell the first time it renders, and marks it sent. If
   anything later in the frame covers that cell the upload is lost forever
   (blank animation frames). `SpriteEncoder` takes the uploads out at encode
   time (`kitty_upload::take_upload`); `render_pen` queues each one just
   before its frame is first shown, and `draw_frame` writes them ahead of
   the diff.
3. **Delete Kitty images you drop.** Replacing/removing a slot queues
   `a=d,d=I` deletes for its uploaded ids. Terminals keep images until
   deleted and evict in-use ones once their quota fills.
4. **Clear before overlaying sprites.** A styled `Block` only restyles
   cells; draw `Clear` first (nameplates, flash, popups), or sprite cells
   survive underneath in the overlay's colours.
5. **Run `fix_orphan_placeholders` after everything is drawn.** Kitty infers
   a placeholder's column from its left neighbour; anything drawn across an
   image row breaks that. `ui()` calls it last.
6. **Never encode on the render thread.** Frames are encoded by the loader
   threads (`SpriteEncoder`); `render_pen` only re-encodes as a fallback.
   Encoding all frames of six creatures took 1–4 s and froze the UI.
7. **Fixed tick.** `run_app` advances the game once per 50 ms tick; input
   only triggers a redraw. (Ticking per loop iteration made creatures speed
   up while keys were pressed.)
8. Frames are written through a 64 KiB `BufWriter` inside a synchronized
   update (DEC 2026) so terminals present whole frames.

**Known limitation:** ratatui-image's startup capability query reads stdin
on a helper thread. If a terminal never answers the query, that thread
lingers and swallows the first keypress. Real terminals, SSH and tmux all
answer; only affects unusual setups.

---

## Config Format

Current format (v0.4.0+, with roster persistence):

```toml
[display]
scale = 3  # sprite scale multiplier — memory scales quadratically

[[slot]]
id = 25
slot_id = 8675309
name = "Pikachu"
level = 3
xp = 42

[[slot]]
id = 133
slot_id = 8675310
name = "Eevee"
level = 1
xp = 0
```

Legacy format (pre-v0.4.0, still accepted on load):

```toml
[display]
scale = 3

[roster]
creatures = ["pikachu", "eevee"]
```

Config is auto-migrated: legacy format is read, converted to slot format, and written back on quit.

---

## Development Workflow

```bash
# Build and run
cargo run

# Run with a single creature (no config needed)
cargo run -- --creature pikachu

# Run with custom config
cargo run -- --config ./my-test-roster.toml

# Lint (CI enforces warnings-as-errors)
cargo clippy --all-targets -- -D warnings

# Format check
cargo fmt --check

# Tests
cargo test

# Debug logging (writes to a file)
POCLIMON_DEBUG_LOG=/tmp/poclimon.log cargo run

# Force a graphics protocol (kitty | sixel | iterm2 | halfblocks)
POCLIMON_PROTOCOL=kitty cargo run

# Rendering regression check (see tools/term-harness/README.md)
cd tools/term-harness && python3 run.py ../../target/release/poclimon kitty out/kitty.bin \
  && python3 stale.py out/kitty.bin && python3 kitty_check.py out/kitty.bin
```

---

## CI/CD

Three workflows in `.github/workflows/`:

| Workflow | Trigger | Purpose |
|---|---|---|
| `ci.yml` | Push/PR | fmt, clippy, tests |
| `release.yml` | Manual dispatch | Bumps version, tags, triggers build |
| `build-release.yml` | `v*` tags | Builds multi-platform binaries, creates GitHub Release |

**Secrets required:**
- `RELEASE_PAT` — PAT for pushing version commits
- `CARGO_REGISTRY_TOKEN` — crates.io publish token

CI runs on `ubuntu-latest` only. Release builds produce Linux (musl), Windows, and macOS (arm64 + x86_64) binaries.

---

## Security Posture (assessed 2026-04-06)

**Overall: No vulnerabilities identified.**

| Area | Status | Notes |
|---|---|---|
| Dependencies | Clean | All well-maintained; no known CVEs |
| Input validation | Strong | Digit-only prompts, length-limited, database-validated creature IDs |
| File paths | Safe | All PathBuf joins; no user input in path construction |
| Network | Safe | `ureq` with hardcoded URLs and u32-derived IDs; no user input flows into URL; 16 MB per-download cap |
| Secrets | Clean | No secrets in repo; GitHub Actions secrets used for release automation |
| Unsafe code | None | Zero `unsafe` blocks |
| Thread safety | Good | OnceLock + Mutex for shared state; mpsc channels for sprite loading |

**Open notes:**
- `edition = "2024"` in Cargo.toml — verify this compiles on your Rust toolchain. Rust 2024 edition stabilized in Rust 1.85 (Feb 2025).
- `ureq` 3 with the `rustls` feature — small synchronous client (no tokio/hyper), pure-Rust TLS, bundled Mozilla root CAs via webpki-roots. Chosen over `reqwest::blocking`, which added ~1.2 MB to the binary.
- `image` and `ratatui-image` both use `default-features = false, features = ["png"]` — only PNG decoder compiled in, removing ~13 unused format decoders.
- Sprite disk cache (`~/.config/poclimon/sprites/`) has no eviction or size cap. Each creature caches ≈5 PNGs + 1 XML; cache grows permanently but only for creatures actually loaded.
- Background sprite loading spawns one thread per creature load (unbounded). Fine for current 6-creature max; consider a thread pool if that limit increases.
- Release binary: ~5.0 MB on x86_64 Linux (PNG-only `image`, ureq, thin LTO + strip). Memory at scale=3 with 6 creatures: up to ~35 MB raw frames (Arc-shared fallbacks reduce this in practice).

---

## Pending Admin Actions

1. **Verify Rust edition 2024 in CI** — ensure `ci.yml` pins a toolchain that ships 2024 edition (`>=1.85`).
2. **Add SECURITY.md** — document how to report vulnerabilities (even for a hobby project, good practice).
3. ~~**In-process HTTP for sprite downloads**~~ — done; `curl` subprocess replaced with `ureq`.
4. **crates.io publish** — package is configured for publishing; confirm `CARGO_REGISTRY_TOKEN` secret is current before next release.

---

## Credits

- Sprites: [PMDCollab SpriteCollab](https://sprites.pmdcollab.org/) — CC BY-NC
- Pokémon is a trademark of Nintendo / Game Freak / The Pokémon Company. Unofficial and non-commercial.
