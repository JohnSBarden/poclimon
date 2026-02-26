# 🎮 PoCLImon

**A terminal-based virtual pet — Tamagotchi in your terminal!**

> **Latest release: v0.1.0 (February 26, 2026)** — improved collision behavior, free pen movement, sprite size fixes, animation fallback, and directional sprites (see [Changelog](#changelog))

> 📸 *Screenshot coming soon*

## Features

- 🐾 Animated pixel-art creatures rendered directly in your terminal
- 🎨 Sprite animations from the PMDCollab SpriteCollab repository
- 🌿 Shared-pen view — all creatures share one open canvas, no dividing walls
- 🔄 Multiple creatures displayed simultaneously (up to 6)
- 🧭 Direction-aware sprite animations with movement-based facing
- 🏷️ Compact bordered nameplates centered under each sprite
- 🍖 Feed, sleep, and interact with your creatures
- ➕ Add, remove, and swap creatures at runtime — no config editing required
- ⚙️ TOML-based configuration for customizing your starting roster
- 🖼️ Sixel/Kitty/iTerm2 image protocol support via ratatui-image
- ⚡ Performance-optimised: sprites pre-scaled at load time, only redrawn when
  the displayed frame changes; frames stored once, not twice

## Creature Roster

PoCLImon ships with 11 creatures, all sourced from the PMDCollab sprite
repository. Press `A` in-app to add them to your roster on demand.

| Name       | ID  | Group                    |
|------------|-----|--------------------------|
| Bulbasaur  | 1   | Gen 1 Starter            |
| Charmander | 4   | Gen 1 Starter            |
| Squirtle   | 7   | Gen 1 Starter            |
| Pikachu    | 25  | Electric Mouse           |
| Eevee      | 133 | Eevee Family             |
| Vaporeon   | 134 | Eevee Evolution (Water)  |
| Jolteon    | 135 | Eevee Evolution (Electric)|
| Flareon    | 136 | Eevee Evolution (Fire)   |
| Articuno   | 144 | Legendary Bird (Ice)     |
| Zapdos     | 145 | Legendary Bird (Electric)|
| Moltres    | 146 | Legendary Bird (Fire)    |

## Installation

```bash
# Clone and build from source
git clone https://github.com/JohnSBarden/poclimon.git
cd poclimon
cargo install --path .
```

## Usage

```bash
# Run with default config (~/.poclimon/config.toml)
poclimon

# Quick override — show a single creature
poclimon --creature pikachu

# Use a custom config file
poclimon --config ./my-config.toml
```

### CLI Arguments

| Argument      | Description                                      |
|---------------|--------------------------------------------------|
| `--creature`  | Quick override: show only this creature by name  |
| `--config`    | Path to a custom TOML config file                |

## Controls

### Animation States

| Key         | Action                                   |
|-------------|------------------------------------------|
| `E`         | Feed the selected creature (loops)       |
| `S`         | Put the selected creature to sleep (loops)|
| `I`         | Set the selected creature to idle (loops)|

### Roster Management

| Key         | Action                                                    |
|-------------|-----------------------------------------------------------|
| `A`         | Add the next available creature to the roster             |
| `R`         | Remove the selected creature (requires 2+ creatures)      |
| `Tab`       | Swap the selected slot to the next creature               |

### Navigation

| Key         | Action                          |
|-------------|---------------------------------|
| `←` / `→`  | Cycle selected creature         |
| `1`–`6`    | Select creature by slot number  |
| `Q` / `Esc` | Quit                           |

## Runtime Debugging

Set `POCLIMON_DEBUG_LOG` to capture movement/collision/render diagnostics while
running the TUI:

```bash
POCLIMON_DEBUG_LOG=/tmp/poclimon-debug.log cargo run
```

This is useful when tuning collision/facing behavior.

## Configuration

PoCLImon uses a TOML config file. Default location: `~/.poclimon/config.toml`

```toml
# PoCLImon Configuration

[display]
# Scale multiplier for sprites.
# Default: 3 (since v0.0.3; was 6 in v0.0.2).
# Lower values use less memory: scale=3 uses ~4× less RAM than scale=6.
scale = 3

[roster]
# Starting creatures (max 6 at startup; use A/R/Tab to change at runtime)
# Use Pokemon names (lowercase) or IDs
creatures = ["pikachu", "eevee", "bulbasaur"]
```

The `roster.creatures` array sets the starting roster. You can then use
`A`, `R`, and `Tab` to modify it at runtime without editing any files.

## Credits

- Sprites from [PMDCollab SpriteCollab](https://sprites.pmdcollab.org/) — community-contributed Pokémon Mystery Dungeon sprite sheets
- Licensed under **CC BY-NC** (Creative Commons Attribution-NonCommercial)

## Changelog

### v0.1.0 (2026-02-26)
- Improved collision behavior.
- Carries forward the v0.0.4 movement/render improvements.

### v0.0.4 (2026-02-25)
- Free pen movement.
- Sprite size fix.
- Animation fallback.

### v0.0.3
- **Shared pen view** — replaced the bordered-box grid with a single open
  canvas. All creatures share one area; the selected creature is highlighted
  with a selected marker in yellow. No internal dividers.
- Directional sprite facing follows movement heading with hysteresis to reduce
  jitter.
- Nameplates switched to compact bordered plates centered under sprites.
- **Memory optimisation** — scale default changed from 6 → 3 (4× less RAM
  per frame). Frame cache capped at 8 frames per animation. `Animation` is
  now timing-only; pixel data lives exclusively in the slot cache (no more
  double-storage).
- **New unit tests** — pen layout (`compute_creature_region`) and frame
  capping (`cap_frames`) fully tested.
- Added runtime debug logging via `POCLIMON_DEBUG_LOG` for collision/render
  triage.

### v0.0.2
- Multi-creature grid layout (1–6 slots, bordered boxes)
- Sprite download + caching from PMDCollab SpriteCollab

---

## License

This project is for personal/educational use. Pokémon is a trademark of Nintendo/Game Freak/The Pokémon Company. Sprites are used under the PMDCollab CC BY-NC license.
