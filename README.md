# 🎮 PoCLImon

**A terminal-based virtual pet — Tamagotchi in your terminal!**

> 📸 *Screenshot coming soon*

## Features

- 🐾 Animated pixel-art creatures rendered directly in your terminal
- 🎨 Sprite animations from the PMDCollab SpriteCollab repository
- 🔄 Multiple creatures displayed simultaneously (up to 6)
- 🍖 Feed, sleep, and interact with your creatures
- ⚙️ TOML-based configuration for customizing your roster
- 🖼️ Sixel/Kitty/iTerm2 image protocol support via ratatui-image

## Creature Roster

| Name       | ID  |
|------------|-----|
| Bulbasaur  | 1   |
| Charmander | 4   |
| Squirtle   | 7   |
| Pikachu    | 25  |
| Eevee      | 133 |

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

| Key         | Action                          |
|-------------|---------------------------------|
| `E`         | Feed the selected creature      |
| `S`         | Put the selected creature to sleep |
| `I`         | Set the selected creature to idle |
| `←` / `→`  | Cycle selected creature         |
| `1`–`6`    | Select creature by slot number  |
| `Q` / `Esc` | Quit                           |

## Configuration

PoCLImon uses a TOML config file. Default location: `~/.poclimon/config.toml`

```toml
# PoCLImon Configuration

[display]
# Scale multiplier for sprites (default 6)
scale = 6

[roster]
# Active creatures to display (max 6)
# Use Pokemon names (lowercase) or IDs
creatures = ["pikachu", "eevee", "bulbasaur"]
```

The `roster.creatures` array determines which creatures are displayed simultaneously.
You can use lowercase names or numeric IDs (as strings). Maximum 6 creatures.

## Credits

- Sprites from [PMDCollab SpriteCollab](https://sprites.pmdcollab.org/) — community-contributed Pokémon Mystery Dungeon sprite sheets
- Licensed under **CC BY-NC** (Creative Commons Attribution-NonCommercial)

## License

This project is for personal/educational use. Pokémon is a trademark of Nintendo/Game Freak/The Pokémon Company. Sprites are used under the PMDCollab CC BY-NC license.
