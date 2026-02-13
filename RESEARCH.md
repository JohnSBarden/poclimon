# PoCLImon Research Report

*Generated: 2026-02-13*

## Table of Contents

1. [Current State of the Project](#1-current-state-of-the-project)
2. [Terminal UI Frameworks for Rust](#2-terminal-ui-frameworks-for-rust)
3. [Sprite Rendering in the Terminal](#3-sprite-rendering-in-the-terminal)
4. [Relevant Rust Crates for Image/Sprite Rendering](#4-relevant-rust-crates-for-imagesprite-rendering)
5. [Similar Projects & Inspiration](#5-similar-projects--inspiration)
6. [Pokémon Sprite Sources](#6-pokémon-sprite-sources)
7. [Architecture Recommendations](#7-architecture-recommendations)
8. [Recommended Tech Stack](#8-recommended-tech-stack)
9. [MVP Roadmap](#9-mvp-roadmap)
10. [Open Questions](#10-open-questions)

---

## 1. Current State of the Project

### Files & Structure

```
poclimon/
├── Cargo.toml
├── Cargo.lock
├── poclimon_config.json
├── .gitignore
└── src/
    ├── main.rs        # CLI entry point (clap)
    ├── animal.rs       # Animal struct + ASCII art sprites
    ├── render.rs       # Crossterm-based renderer
    ├── game.rs         # Game loop (event handling, update, render)
    └── config/
        └── mod.rs      # JSON config (serde)
```

### What's There

- **Game loop**: Working event loop at 10 FPS with quit/pause/reset controls
- **3 animal types**: Cat, dog, bird — each with 4 states (idle, sleeping, playing, walking)
- **ASCII art sprites**: Unicode block characters (▄▀█▐▌) with 2-frame animations per state
- **Crossterm renderer**: Raw mode, alternate screen, colored sprite drawing
- **JSON config**: Configurable animals with name, kind, position
- **CLI**: Clap-based with optional config path

### Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| crossterm | 0.27.0 | Terminal I/O, raw mode, events |
| serde/serde_json | 1.0 | Config serialization |
| anyhow | 1.0 | Error handling |
| thiserror | 1.0 | Custom error types |
| rand | 0.8 | Random state changes |
| lazy_static | 1.4 | Static sprite data |
| clap | 4.0 | CLI argument parsing |

### What Works

- Basic game loop runs
- ASCII art animals render with color
- Animals cycle through animation frames
- Random state transitions
- Keyboard input (q/p/r)

### What's Missing / Needs Work

- **No Pokémon sprites** — currently generic cat/dog/bird ASCII art
- **No image rendering** — only hardcoded Unicode text sprites
- **No Ratatui** — uses raw crossterm directly (harder to build complex UI)
- **No interaction** — can't pet, feed, or interact with animals
- **No persistence** — animal state resets each run
- **Edition 2024** — bleeding edge Rust edition, may cause compatibility issues
- **Outdated crossterm** — 0.27 is old; current is 0.28+
- **No README** — repo has no documentation

### Assessment

Good foundation for learning Rust. The architecture (game loop, config, render separation) is solid. The main gap is the jump from ASCII art to actual sprite/image rendering, which is the core challenge of this project.

---

## 2. Terminal UI Frameworks for Rust

### Ratatui

- **Crate**: `ratatui` v0.30.0 | **Downloads**: 17.3M total, 5.2M recent
- **Repo**: github.com/ratatui/ratatui
- **Status**: Very actively maintained, successor to tui-rs

| Pros | Cons |
|------|------|
| Massive community, excellent docs | Widget-based model adds abstraction layer |
| Rich widget ecosystem (tables, charts, etc.) | Slightly more boilerplate than raw crossterm |
| `ratatui-image` crate exists for image rendering | Learning curve for layout system |
| Excellent for building complex UIs (stats, menus) | |
| Built on crossterm (or termion/termwiz backends) | |
| Great examples and cookbook | |
| Active Discord community | |

**Best for poclimon?** **YES** — the ecosystem is unbeatable. `ratatui-image` solves the sprite rendering problem directly. The widget system will make it easy to add UI around the pet (stats bars, menus, etc.).

### Crossterm

- **Crate**: `crossterm` v0.28+ | Cross-platform terminal manipulation
- **Status**: Actively maintained, used as Ratatui's default backend

| Pros | Cons |
|------|------|
| Low-level control | No widget system — build everything yourself |
| Cross-platform (Windows/Mac/Linux) | More code for UI layout |
| Already used in poclimon | No image rendering built-in |
| Good for understanding terminal internals | |

**Best for poclimon?** Use as **backend** (via Ratatui), not directly. The current codebase uses it directly, but migrating to Ratatui will save tons of work.

### Cursive

- **Crate**: `cursive` | ncurses-style TUI
- **Status**: Maintained but less active than Ratatui

| Pros | Cons |
|------|------|
| Dialog/form-oriented UI | Not designed for games or animations |
| Good for text-heavy apps | No image rendering support |
| | Smaller ecosystem than Ratatui |

**Best for poclimon?** No — wrong paradigm for a game with sprites.

### tui-rs (Legacy)

- **Status**: Archived/unmaintained — Ratatui is its successor
- **Verdict**: Don't use. Ratatui is the direct continuation.

### Summary

**Use Ratatui + Crossterm backend.** It's the clear winner for this project.

---

## 3. Sprite Rendering in the Terminal

### Rendering Protocols

#### Unicode Half-Block Characters (▀▄█)

Each terminal cell is split into top/bottom halves using `▀` (upper half) or `▄` (lower half), with foreground and background colors set independently. This gives **2 vertical pixels per cell**.

| Pros | Cons |
|------|------|
| Works in ALL terminals | Low resolution (~2x pixel density) |
| No special protocol needed | Colors limited to what terminal supports |
| Simple to implement | Sprites look blocky at small sizes |
| Fast rendering | |
| Portable everywhere | |

**Resolution**: A 96x96 sprite would need ~48x96 cells — too large. Works best for sprites ≤32x32 pixels (16x32 cells).

#### Sixel Graphics

Binary image protocol from the DEC VT340 era. Sends actual pixel data to the terminal.

| Pros | Cons |
|------|------|
| True pixel-level rendering | Not supported in many terminals |
| Good color support (256+ colors) | Flickering on redraw in some terminals |
| Widely supported among "fancy" terminals | No Windows Terminal support |
| | Slower than halfblocks for small images |

**Supported terminals**: xterm (with `-ti vt340`), mlterm, foot, WezTerm, Contour, mintty, some others.
**NOT supported**: Windows Terminal, Alacritty, GNOME Terminal (vte-based), macOS Terminal.app

#### Kitty Graphics Protocol

Modern protocol by the Kitty terminal. Transmits PNG/RGB data, terminal composites it.

| Pros | Cons |
|------|------|
| Best quality — true image rendering | Only Kitty and WezTerm support it |
| Supports transparency/alpha | Very limited terminal support |
| Efficient (can cache images) | |
| Animation-friendly (replace in-place) | |

**Supported terminals**: Kitty, WezTerm, Ghostty (partial), Konsole (partial)

#### iTerm2 Inline Images Protocol

Used by iTerm2 on macOS. Base64-encodes images in escape sequences.

| Pros | Cons |
|------|------|
| Easy to implement | macOS-only (iTerm2, WezTerm) |
| Good quality | Limited terminal support |

### Protocol Comparison for PoCLImon

| Feature | Half-block | Sixel | Kitty | iTerm2 |
|---------|-----------|-------|-------|--------|
| Universality | ★★★★★ | ★★☆☆☆ | ★☆☆☆☆ | ★☆☆☆☆ |
| Image quality | ★★☆☆☆ | ★★★★☆ | ★★★★★ | ★★★★☆ |
| Animation ease | ★★★★☆ | ★★★☆☆ | ★★★★★ | ★★☆☆☆ |
| Speed | ★★★★★ | ★★★☆☆ | ★★★★☆ | ★★★☆☆ |
| Pokémon sprite suitability | ★★★☆☆ | ★★★★☆ | ★★★★★ | ★★★★☆ |

### Recommendation

**Use `ratatui-image` which supports ALL of these protocols** and auto-detects the best one available. Write once, render everywhere. Halfblock is the universal fallback; sixel/kitty/iTerm2 are used when available.

---

## 4. Relevant Rust Crates for Image/Sprite Rendering

### ratatui-image (★★★★★ — PRIMARY RECOMMENDATION)

- **Version**: 10.0.5 | **Downloads**: 258K total, 63K recent
- **Repo**: github.com/benjajaja/ratatui-image
- **Protocols**: Sixel, Kitty, iTerm2, Unicode halfblocks
- **Integration**: Native Ratatui widget — just add to your layout

**Key features**:
- `StatefulImage` widget for Ratatui
- Auto-detects best protocol for current terminal
- Resize/crop support
- Works with `image` crate's `DynamicImage`
- Async image loading support
- Halfblock fallback for unsupported terminals

**How to use**:
```rust
use ratatui_image::{picker::Picker, StatefulImage, protocol::StatefulProtocol};

// On startup: detect terminal capabilities
let mut picker = Picker::from_termios()?;

// Load an image
let dyn_img = image::open("pikachu.png")?;
let image_state = picker.new_resize_protocol(dyn_img);

// In render:
let image_widget = StatefulImage::new(None);
frame.render_stateful_widget(image_widget, area, &mut image_state);
```

### viuer (★★★☆☆ — ALTERNATIVE)

- **Version**: 0.11.0 | **Downloads**: 814K total, 55K recent
- **Protocols**: Sixel, Kitty, iTerm2, halfblocks
- **Integration**: Standalone (prints directly to stdout)

| Pros | Cons |
|------|------|
| Simple API — just `viuer::print()` | Not a Ratatui widget — can't compose in layouts |
| More mature (2020) | Prints directly, conflicts with TUI frameworks |
| Good for simple "display image in terminal" | Not suitable for game rendering |

**Verdict**: Great for CLI tools, but **not suitable for poclimon** since it bypasses Ratatui's rendering pipeline.

### image (★★★★★ — REQUIRED)

- The standard Rust image processing crate
- Load PNG, GIF, etc.
- Resize, crop, manipulate pixels
- Required by both ratatui-image and viuer
- Use this to load and pre-process Pokémon sprites

### Other Relevant Crates

| Crate | Purpose | Useful? |
|-------|---------|---------|
| `image` | Image loading/processing | Yes — dependency |
| `gif` | GIF decoding (for animated sprites) | Maybe — for animation frames |
| `tokio` | Async runtime | Yes — for async event handling |
| `reqwest` | HTTP client | Yes — to fetch sprites from PokeAPI |
| `directories` | XDG/platform dirs | Yes — for sprite cache location |

---

## 5. Similar Projects & Inspiration

### vscode-pets (Primary Inspiration)

- **Extension**: VS Code marketplace, by tonybaloney
- **Architecture**: 
  - Uses a VS Code Webview panel (essentially an embedded browser)
  - Pets are rendered as animated GIF/sprite sheets on an HTML5 canvas
  - TypeScript-based with sprite animation loop
  - Pets walk, sit, run, chase, play with ball
  - Each pet type has a sprite sheet with frames for each state/direction
  - Collision detection with panel edges
  - Pet states: sit, walk, run, lie, wallHang, climb, etc.
- **What to steal**:
  - State machine approach for pet behavior
  - Sprite sheet organization (states × directions × frames)
  - Interaction model (click to chase, throw ball)
  - The "fun factor" — pets should feel alive

### Terminal-Based Pet/Creature Projects

- **`pokemon-colorscripts`** — Prints colored Pokémon sprites in terminal using Unicode. Great reference for how Pokémon look in terminal art.
- **`pokeshell`** — Shell script that displays Pokémon in terminal.
- **`terminal-pet`** — Various implementations of desktop pets in terminal.
- **`nyancat`** — Classic terminal animation, good reference for frame-based terminal animation.

### Rust TUI Games

- **`minesweeper-rs`** — Ratatui-based minesweeper
- **`snake-tui`** — Snake game in Ratatui
- **`game-of-life-tui`** — Conway's Game of Life in Ratatui
- These demonstrate the game loop + Ratatui rendering pattern

### Key Takeaways from Similar Projects

1. **Sprite sheets work well** — organize by state, direction, frame number
2. **State machines are essential** — pets need clear behavioral states
3. **Small sprites (32x32 or 64x64) look best** in terminals
4. **Pre-converted halfblock art** can be cached for fast rendering
5. **pokemon-colorscripts proves Pokémon sprites look good in terminal Unicode**

---

## 6. Pokémon Sprite Sources

### PokeAPI (pokeapi.co)

- **Free REST API** with sprite URLs for all Pokémon
- **Endpoint**: `https://pokeapi.co/api/v2/pokemon/{id_or_name}`
- **Sprite URLs**: Multiple variants per Pokémon:
  - `sprites.front_default` — 96x96 PNG
  - `sprites.back_default` — 96x96 PNG  
  - `sprites.front_shiny` — shiny variant
  - `sprites.versions.generation-v.black-white.animated` — **animated GIFs** (Gen 5 style, pixel art, PERFECT for this project)
  - `sprites.other.showdown` — Showdown-style animated GIFs
- **Hosted on**: GitHub (raw.githubusercontent.com)
- **License**: Fan use / educational (not for commercial)

### Best Sprite Choice for PoCLImon

**Gen 5 (Black & White) animated sprites** are ideal:
- Pixel art style (designed for low resolution)
- Animated GIFs with idle animations
- ~80x80 pixels — good size for terminal rendering
- All 649 Gen 1-5 Pokémon available
- The pixel art aesthetic matches terminal rendering perfectly

**Showdown sprites** are also excellent:
- Cover all generations
- Animated
- Consistent style

### Sprite Pipeline

```
PokeAPI → Download PNG/GIF → Extract frames (if animated)
→ Resize to target size → Convert to terminal format
→ Cache locally → Render via ratatui-image
```

---

## 7. Architecture Recommendations

### Proposed Architecture

```
poclimon/
├── src/
│   ├── main.rs              # Entry point, CLI
│   ├── app.rs               # App state (replaces game.rs)
│   ├── ui/
│   │   ├── mod.rs            # UI rendering (Ratatui widgets)
│   │   ├── pet_view.rs       # Pet sprite widget
│   │   └── status_bar.rs     # Stats, help text
│   ├── pet/
│   │   ├── mod.rs            # Pet struct, state machine
│   │   ├── behavior.rs       # AI/random behavior
│   │   └── stats.rs          # Hunger, happiness, energy
│   ├── sprites/
│   │   ├── mod.rs            # Sprite loading & caching
│   │   ├── loader.rs         # Download from PokeAPI
│   │   └── cache.rs          # Local sprite cache
│   └── config.rs             # Configuration
├── sprites/                   # Cached sprite files
├── Cargo.toml
└── README.md
```

### Key Design Decisions

1. **Ratatui for UI** — widget-based layout, compose pet view + stats + menu
2. **ratatui-image for sprites** — auto-detect protocol, halfblock fallback
3. **Async with tokio** — non-blocking sprite downloads, event handling
4. **State machine for pet behavior** — clear transitions, extensible
5. **Local sprite cache** — download once from PokeAPI, cache in `~/.poclimon/sprites/`
6. **GIF frame extraction** — use `image` or `gif` crate to extract animation frames, cycle through them

---

## 8. Recommended Tech Stack

### Core Dependencies

```toml
[dependencies]
ratatui = "0.30"
crossterm = { version = "0.28", features = ["event-stream"] }
ratatui-image = "10.0"
image = "0.25"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
clap = { version = "4", features = ["derive"] }
directories = "5"
```

### Migration Path from Current Code

1. **Add Ratatui** — replace raw crossterm rendering with Ratatui widgets
2. **Add ratatui-image** — replace ASCII art with actual sprite images
3. **Download a test sprite** — pikachu.png from PokeAPI, render it
4. **Iterate** — add more features once sprites work

---

## 9. MVP Roadmap

### Phase 1: Get a Pokémon Sprite Rendering (1-2 days)

- [ ] Add ratatui + ratatui-image to Cargo.toml
- [ ] Migrate renderer from raw crossterm to Ratatui
- [ ] Download a Pikachu sprite manually, put in `sprites/`
- [ ] Render it using ratatui-image
- [ ] Verify it works with halfblock fallback

### Phase 2: Sprite Loading & Animation (2-3 days)

- [ ] Implement PokeAPI sprite downloader
- [ ] Extract GIF frames for animation
- [ ] Cycle through frames on a timer
- [ ] Add local sprite caching
- [ ] CLI: choose Pokémon by name/ID

### Phase 3: Pet Behavior (2-3 days)

- [ ] Pet state machine (idle, walking, sleeping, playing)
- [ ] Random movement within terminal bounds
- [ ] Direction-aware sprites (face left/right)
- [ ] Pet stats (happiness, hunger, energy)

### Phase 4: Interaction (2-3 days)

- [ ] Keyboard commands to interact (feed, pet, play)
- [ ] Visual feedback for interactions
- [ ] Stats display alongside pet
- [ ] State persistence (save/load)

### Phase 5: Polish (ongoing)

- [ ] Multiple Pokémon on screen
- [ ] Sound effects (terminal bell?)
- [ ] Evolution mechanics
- [ ] Battle system?
- [ ] README + screenshots

---

## 10. Open Questions

1. **Rust edition**: Currently `edition = "2024"` — this is very new. Consider downgrading to `2021` if crate compatibility is an issue.
2. **Sprite size**: What's the ideal pixel size for terminal sprites? 32x32 or 64x64 seem ideal — need to test.
3. **Animation approach**: Timer-based frame cycling vs. async animation loop?
4. **Terminal compatibility**: What terminals will John primarily use? This affects which protocols to prioritize.
5. **Copyright**: Pokémon sprites are Nintendo/Game Freak IP. Fine for personal/learning projects, but not for distribution. Worth noting.
6. **Windows support**: Does John use Windows? Sixel/Kitty don't work on Windows Terminal, but halfblocks do.

---

*This research report was compiled for the PoCLImon project. The recommended approach is: Ratatui + ratatui-image + PokeAPI sprites, with halfblock fallback for universal terminal support.*
