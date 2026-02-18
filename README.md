# 🎮 PoCLImon

**A terminal-based virtual pet — Tamagotchi in your terminal!**

> 📸 *Screenshot coming soon*

## Features

- 🐾 Animated pixel-art creatures rendered directly in your terminal
- 🎨 Sprite animations from the PMDCollab SpriteCollab repository
- 🔄 Multiple creatures displayed simultaneously (up to 6)
- 🍖 Feed, sleep, and interact with your creatures
- ➕ Add, remove, and swap creatures at runtime — no config editing required
- ⚙️ TOML-based configuration for customizing your starting roster
- 🖼️ Sixel/Kitty/iTerm2 image protocol support via ratatui-image
- ⚡ Performance-optimised: sprites are pre-scaled at load time and only
  redrawn when the displayed frame actually changes

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

## Configuration

PoCLImon uses a TOML config file. Default location: `~/.poclimon/config.toml`

```toml
# PoCLImon Configuration

[display]
# Scale multiplier for sprites (default 6)
scale = 6

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

## License

This project is for personal/educational use. Pokémon is a trademark of Nintendo/Game Freak/The Pokémon Company. Sprites are used under the PMDCollab CC BY-NC license.
