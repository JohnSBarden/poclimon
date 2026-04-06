# CLAUDE.md — PoCLImon Developer Context

This file gives Claude Code (and human contributors) the context needed to work on this repo without re-deriving it.

---

## Project Overview

**poclimon** is a Rust TUI idle pet game. Pokémon live in your terminal — they eat, sleep, play, and accumulate XP. Built on [Ratatui](https://ratatui.rs/) and [ratatui-image](https://github.com/benjajaja/ratatui-image), it renders sprites via Kitty graphics, Sixel, iTerm2 inline, or Unicode halfblocks depending on what the terminal supports.

- **Version:** 0.4.1
- **Edition:** Rust 2024 (requires nightly or recent stable that ships 2024 edition support)
- **Binary:** `poclimon` (single binary, no server, no daemon)

---

## Architecture

```
src/
  main.rs          — CLI entry (clap), event loop bootstrap, interactive prompts
  app.rs           — App state, background sprite loading, tick/render loop
  config/
    mod.rs         — TOML config loading, validation, roster management
  creatures/
    mod.rs         — Pokédex database (898+ entries), padded ID helpers
  creature.rs      — Per-creature state machine (idle/eat/sleep/play), XP/level logic
  animation.rs     — Frame-based animation player
  sprite/
    mod.rs         — Sprite cache, curl-based downloader, protocol detection
  physics.rs       — Elastic collision physics for multi-creature pen
  ui.rs            — Ratatui layout and rendering
  splash.rs        — Startup splash screen
build.rs           — Compile-time sprite embedding for the 11 starters
```

**Data flow:**
1. Config loaded from `~/.config/poclimon.toml` (or `--config`)
2. Creatures initialized from config slots; sprites loaded in background threads via `mpsc` channel
3. Main loop: crossterm events → state updates → ratatui render
4. On quit, creature state (XP, level, slot_id) persisted back to TOML config

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
| Network | Safe | `reqwest` (blocking) with hardcoded URLs and u32-derived IDs; no user input flows into URL |
| Secrets | Clean | No secrets in repo; GitHub Actions secrets used for release automation |
| Unsafe code | None | Zero `unsafe` blocks |
| Thread safety | Good | OnceLock + Mutex for shared state; mpsc channels for sprite loading |

**Open notes:**
- `edition = "2024"` in Cargo.toml — verify this compiles on your Rust toolchain. Rust 2024 edition stabilized in Rust 1.85 (Feb 2025).
- `reqwest` (blocking) with `rustls-tls-webpki-roots` — pure-Rust TLS, no OpenSSL dependency, bundled Mozilla root CAs.
- `image` and `ratatui-image` both use `default-features = false, features = ["png"]` — only PNG decoder compiled in, removing ~13 unused format decoders.
- Sprite disk cache (`~/.config/poclimon/sprites/`) has no eviction or size cap. Each creature caches ≈5 PNGs + 1 XML; cache grows permanently but only for creatures actually loaded.
- Background sprite loading spawns one thread per creature load (unbounded). Fine for current 6-creature max; consider a thread pool if that limit increases.
- Release binary: ~6 MB (after feature trimming + LTO + strip). Memory at scale=3 with 6 creatures: up to ~35 MB raw frames (Arc-shared fallbacks reduce this in practice).

---

## Pending Admin Actions

1. **Verify Rust edition 2024 in CI** — ensure `ci.yml` pins a toolchain that ships 2024 edition (`>=1.85`).
2. **Add SECURITY.md** — document how to report vulnerabilities (even for a hobby project, good practice).
3. ~~**`reqwest` for sprite downloads**~~ — done in v0.4.1; `curl` subprocess replaced.
4. **crates.io publish** — package is configured for publishing; confirm `CARGO_REGISTRY_TOKEN` secret is current before next release.

---

## Credits

- Sprites: [PMDCollab SpriteCollab](https://sprites.pmdcollab.org/) — CC BY-NC
- Pokémon is a trademark of Nintendo / Game Freak / The Pokémon Company. Unofficial and non-commercial.
