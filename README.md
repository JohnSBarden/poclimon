# PoCLImon

A terminal-based creature virtual pet built with Rust, [Ratatui](https://ratatui.rs/), and [ratatui-image](https://github.com/benjajaja/ratatui-image).

## Quick Start

```bash
# Build
cargo build

# Run (defaults to Pikachu)
cargo run

# Start with a specific creature
cargo run -- --creature eevee
cargo run -- --creature-id 4

# Run with a custom config
cargo run -- --config path/to/config.json
```

## Creatures (v0.0.1)

| Name       | ID  |
|------------|-----|
| Bulbasaur  | 1   |
| Charmander | 4   |
| Squirtle   | 7   |
| Pikachu    | 25  |
| Eevee      | 133 |

Use `←`/`→` or `P`/`N` to cycle through the roster in-app.

## Config

```json
{
  "creature_id": 25,
  "creature_name": "Pikachu",
  "alias": "Sparky"
}
```

- `creature_id` — Sprite ID (matches PokeAPI)
- `creature_name` — Display name
- `alias` — Optional internal codename

## Controls

| Key           | Action                    |
|---------------|---------------------------|
| `E`           | Eat (chomping + crumbs)   |
| `S`           | Sleep (dark + zZzZ)       |
| `I`           | Return to idle            |
| `→` / `N`     | Next creature             |
| `←` / `P`     | Previous creature         |
| `Q` / `ESC`   | Quit                      |

## Animations

**Idle** — cycles through 3 variants:
- **Breathe** — gentle squash/stretch (4s)
- **Bounce** — small hop (3s)
- **Sway** — horizontal lean anchored at feet (3s)

**Eating** — ravenous chomping at 30fps with crumb particles flying off

**Sleeping** — deep dark palette, programmatic eye closure, floating "zZzZ", slow breathing + head nod

## Terminal Support

Sprites render best in terminals with graphics protocol support:
- **Kitty**, **WezTerm**, **iTerm2** — Full color rendering
- **Basic terminals** — Unicode halfblock fallback

## Sprites

Downloaded from PokeAPI on first run and cached in `~/.poclimon/sprites/`. Falls back to a generated sprite if download fails.

## Requirements

- Rust 1.85+ (edition 2024)
- `curl` (for sprite downloads)
