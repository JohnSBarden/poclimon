# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build

# Run (uses ~/.poclimon/config.toml, creates it with defaults if missing)
cargo run

# Run with single creature override
cargo run -- --creature pikachu

# Run with custom config
cargo run -- --config ./poclimon.toml

# Run with debug logging
POCLIMON_DEBUG_LOG=/tmp/poclimon-debug.log cargo run

# Lint (must pass clean)
cargo clippy --all-targets --all-features -- -D warnings

# Run unit tests (fast, no network)
cargo test

# Run all tests including the opt-in network integration test
cargo test -- --include-ignored
```

## Architecture

PoCLImon is a Ratatui TUI application (Rust). The entire game loop lives in `src/main.rs`; all other files are support modules.

### Module Overview

| File | Role |
|---|---|
| `src/main.rs` | CLI (clap), game loop, Ratatui rendering, `CreatureSlot` state |
| `src/animation.rs` | Timing-only animation: `Animation` stores frame durations, no pixel data |
| `src/anim_data.rs` | Hand-rolled parser for PMDCollab `AnimData.xml` |
| `src/sprite_sheet.rs` | Extracts frames from PMDCollab sprite sheet PNGs; normalizes to Idle dimensions |
| `src/sprite/mod.rs` | Downloads sprites via `curl`, caches to `~/.poclimon/sprites/{0025}/` |
| `src/sprite/fallback.rs` | Fallback when animations are missing |
| `src/creatures.rs` | Static `ROSTER` (11 creatures) and `FULL_DEX` (Gen 1–9 name lookup) |
| `src/config/mod.rs` | Parses `~/.poclimon/config.toml` into validated `GameConfig` |
| `tests/sprite_integration.rs` | Network integration test (`#[ignore]`), validates PMDCollab download |

### Key Design Decisions

**Memory layout**: `Animation` is timing-only — it holds frame durations but zero pixel data. Actual decoded frames (`DynamicImage` → `Protocol`) live exclusively in `CreatureSlot::cached_idle / cached_eat / cached_sleep`. This eliminates the double-storage from v0.0.2.

**Image encoding**: `ratatui-image` `Protocol` objects (not `StatefulProtocol`) are used so frames are encoded once per `Rect` and reused every render tick. The `Picker` auto-detects the terminal's best protocol (Kitty → Sixel → iTerm2 → halfblock).

**Frame cap**: At most `MAX_CACHED_FRAMES = 8` frames are cached per animation. If a PMDCollab sheet has more, frames are sampled evenly so the animation still looks smooth.

**Scale**: Default `scale = 3`. Memory scales quadratically — `scale = 6` uses 4× more RAM. Pre-scaling happens at load time; no runtime resize.

**Sprite layout**: PMDCollab sheets are `N_frames × 8_directions` PNGs. Row 0 = Down (toward viewer). Direction-aware display uses hysteresis to reduce jitter when a creature changes heading.

**Downloads**: `curl --fail` with connect/request timeouts and atomic `.part` → final writes. Files are cached indefinitely in `~/.poclimon/sprites/`; already-present files are skipped.

**Column widths**: Fixed to `MAX_ACTIVE_CREATURES` columns so adding a creature never shifts existing columns (which would invalidate all encoded `Protocol` objects).

### Configuration

- Default: `~/.poclimon/config.toml` (auto-created on first run)
- Example: `poclimon.toml` in the repo root
- Creatures accepted as lowercase names or National Dex IDs (1–898+)
- Max 6 creatures in roster; runtime add/remove/swap via `A`/`R`/`Tab`
