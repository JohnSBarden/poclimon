# 🎮 PoCLImon

A terminal-based Pokémon virtual pet built with Rust and [Ratatui](https://ratatui.rs).

![Screenshot placeholder](docs/screenshot.png)

## Features

- **Sprite rendering** — Downloads and displays official Pokémon artwork directly in your terminal (Sixel/Kitty/iTerm2/halfblock protocols)
- **Roster system** — Cycle through up to 6 Pokémon with arrow keys or number keys
- **Pet interactions** — Feed and pet your Pokémon to manage their stats
- **On-demand sprite caching** — Sprites are downloaded once and cached in `~/.poclimon/sprites/`

## Build & Run

```bash
cargo build --release
cargo run

# With a custom config:
cargo run -- --config poclimon_config.json
```

## Controls

| Key | Action |
|-----|--------|
| `←` / `→` | Cycle through Pokémon roster |
| `1`–`6` | Jump to a specific slot |
| `F` | Feed your Pokémon (reduces hunger) |
| `P` | Pet your Pokémon (increases happiness) |
| `Q` / `Esc` | Quit |

## Configuration

Edit `poclimon_config.json` to customize your roster:

```json
{
  "roster": [
    {"id": 25, "name": "Pikachu", "nickname": "Sparky"},
    {"id": 4, "name": "Charmander", "nickname": ""}
  ]
}
```

The `id` field is the National Pokédex number used to fetch sprites from [PokeAPI](https://github.com/PokeAPI/sprites).

## Dependencies

- [ratatui](https://crates.io/crates/ratatui) — Terminal UI framework
- [ratatui-image](https://crates.io/crates/ratatui-image) — Image rendering in terminal
- [crossterm](https://crates.io/crates/crossterm) — Cross-platform terminal handling
- [clap](https://crates.io/crates/clap) — CLI argument parsing
- [serde](https://crates.io/crates/serde) / [serde_json](https://crates.io/crates/serde_json) — Config serialization
- [image](https://crates.io/crates/image) — Image decoding
- [anyhow](https://crates.io/crates/anyhow) / [thiserror](https://crates.io/crates/thiserror) — Error handling

## Requirements

- A terminal with image protocol support (Kitty, iTerm2, WezTerm, foot) for best results; falls back to Unicode halfblocks
- `curl` on PATH (used for sprite downloads)

## License

MIT
