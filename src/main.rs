mod anim_data;
mod animation;
mod app;
mod cli;
mod config;
mod creature;
mod creatures;
mod devtools;
mod kitty_upload;
mod notification;
mod sprite;
mod sprite_loading;
mod sprite_sheet;
mod ui;

use app::App;
use clap::Parser;
use cli::Cli;
use config::GameConfig;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute, queue,
    terminal::{
        BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};
use ratatui_image::picker::{Picker, cap_parser::QueryStdioOptions};
use std::io::{self, BufWriter, Stdout, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Frames are assembled in a 64 KiB buffer and written in one go; with the
/// raw `Stdout` (a line-buffered writer) a frame went out in many small
/// writes, letting the terminal paint half-finished frames.
type Term = Terminal<CrosstermBackend<BufWriter<Stdout>>>;

/// Game tick. Physics, animation and XP advance exactly once per tick,
/// independent of how many input events arrive.
const TICK: Duration = Duration::from_millis(50);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    let (config, save_path): (GameConfig, Option<PathBuf>) = if let Some(name) = &args.creature {
        // Quick override — single creature, no persistence.
        let cfg = match GameConfig::from_creature_name(name) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Warning: {e} — using default");
                GameConfig::default()
            }
        };
        (cfg, None)
    } else if let Some(path) = args.config {
        let cfg = GameConfig::load(&path)?;
        (cfg, Some(path))
    } else {
        let path = config::default_config_path();
        let cfg = GameConfig::load_default().unwrap_or_default();
        (cfg, Some(path))
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(BufWriter::with_capacity(1 << 16, stdout));
    let mut terminal = Terminal::new(backend)?;

    // Kitty zlib compression: sprites are flat-colour pixel art scaled up with
    // nearest-neighbour, which deflates extremely well. Only used if the
    // terminal answers the probe.
    let query_options = QueryStdioOptions {
        kitty_compression: true,
        ..Default::default()
    };
    let mut picker = Picker::from_query_stdio_with_options(query_options)
        .unwrap_or_else(|_| Picker::halfblocks());
    // Dev/testing override: force a graphics protocol regardless of detection.
    if let Ok(p) = std::env::var("POCLIMON_PROTOCOL") {
        use ratatui_image::picker::ProtocolType;
        let forced = match p.to_ascii_lowercase().as_str() {
            "kitty" => Some(ProtocolType::Kitty),
            "sixel" => Some(ProtocolType::Sixel),
            "iterm2" => Some(ProtocolType::Iterm2),
            "halfblocks" => Some(ProtocolType::Halfblocks),
            _ => None,
        };
        if let Some(t) = forced {
            picker.set_protocol_type(t);
        }
    }

    let mut app = App::new(config, save_path);
    app.is_tmux = picker.tmux_detected();
    app.encoder = Some(sprite_loading::SpriteEncoder::new(&picker));

    app.start_background_loads();

    let res = run_app(&mut terminal, &mut app, &mut picker);

    // Free the terminal-side image memory Kitty/Ghostty would otherwise keep.
    let ids = app.all_kitty_ids();
    app.retire_kitty_images(&ids);
    write_term_writes(&mut app.term_writes);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("Error: {e}");
    }

    Ok(())
}

/// Write queued Kitty uploads/deletes straight to the terminal.
///
/// Called from inside the draw closure, i.e. after the frame's widgets are
/// built but before ratatui writes the diff, so every image is in the terminal
/// before the placeholders that show it.
fn write_term_writes(writes: &mut Vec<String>) {
    if writes.is_empty() {
        return;
    }
    let mut out = io::stdout().lock();
    for w in writes.drain(..) {
        let _ = out.write_all(w.as_bytes());
    }
    let _ = out.flush();
}

/// Draw one frame inside a synchronized update (DEC mode 2026), so terminals
/// that support it present the whole frame at once instead of mid-write.
/// Terminals without support ignore the sequences.
fn draw_frame(terminal: &mut Term, app: &mut App, picker: &mut Picker) -> io::Result<()> {
    queue!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
    let completed = terminal.draw(|f| {
        ui::ui(f, app, picker, env!("CARGO_PKG_VERSION"));
        write_term_writes(&mut app.term_writes);
    })?;
    let dumped = devtools::frame_dump_enabled().then(|| completed.buffer.clone());
    execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;
    if let Some(buf) = dumped {
        devtools::frame_dump(&buf);
    }
    Ok(())
}

fn run_app(
    terminal: &mut Term,
    app: &mut App,
    picker: &mut Picker,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame_duration = TICK;

    // Splash: ~2 seconds (40 ticks × 50ms); any keypress skips it.
    'splash: for _ in 0..40u32 {
        app.update_all_displays();
        terminal.draw(ui::render_splash)?;
        if event::poll(frame_duration)?
            && let Event::Key(KeyEvent {
                code,
                kind: KeyEventKind::Press,
                ..
            }) = event::read()?
        {
            if matches!(code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc) {
                app.running = false;
                break 'splash;
            }
            break 'splash;
        }
    }

    let mut next_tick = Instant::now();
    while app.running {
        let now = Instant::now();
        if now >= next_tick {
            app.update_all_displays();
            draw_frame(terminal, app, picker)?;
            next_tick += TICK;
            if next_tick < now {
                // Fell behind (e.g. the terminal was slow to drain): resync
                // rather than running a burst of catch-up ticks.
                next_tick = now + TICK;
            }
        }

        let timeout = next_tick.saturating_duration_since(Instant::now());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(KeyEvent {
                    code,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    handle_key(app, code);
                    // Show the effect of input right away, without advancing the game.
                    draw_frame(terminal, app, picker)?;
                }
                Event::Resize(..) => draw_frame(terminal, app, picker)?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode) {
    match code {
        // ── Prompt intercept — handles all keys when a prompt is open ──
        _ if app.prompt_mode != app::PromptMode::None => match code {
            KeyCode::Esc => {
                app.prompt_mode = app::PromptMode::None;
                app.prompt_buffer.clear();
            }
            KeyCode::Enter => {
                let buf = app.prompt_buffer.trim().to_string();
                let mode = app.prompt_mode;
                app.prompt_mode = app::PromptMode::None;
                app.prompt_buffer.clear();
                if let Ok(id) = buf.parse::<u32>() {
                    match mode {
                        app::PromptMode::Add => app.add_creature_by_dex(id),
                        app::PromptMode::Swap => app.swap_selected_to_dex(id),
                        app::PromptMode::None => {}
                    }
                } else {
                    app.notify(
                        notification::NotifLevel::Warn,
                        "Invalid Pokédex number — enter digits only",
                    );
                }
            }
            KeyCode::Backspace => {
                app.prompt_buffer.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() && app.prompt_buffer.len() < 4 => {
                app.prompt_buffer.push(c);
            }
            _ => {}
        },
        // ── Normal game controls ───────────────────────────────────────
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            app.running = false;
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            app.set_selected_state(animation::AnimationState::Eating);
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            app.set_selected_state(animation::AnimationState::Sleeping);
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            app.set_selected_state(animation::AnimationState::Idle);
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            app.set_selected_state(animation::AnimationState::Playing);
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            if app.has_background_load() {
                app.notify(
                    notification::NotifLevel::Warn,
                    "Please wait for the current load to finish",
                );
            } else if app.slots.len() < config::MAX_ACTIVE_CREATURES {
                app.prompt_mode = app::PromptMode::Add;
                app.prompt_buffer.clear();
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.remove_selected();
        }
        KeyCode::Tab => {
            if app.has_background_load() {
                app.notify(
                    notification::NotifLevel::Warn,
                    "A creature load is already in progress",
                );
            } else {
                app.prompt_mode = app::PromptMode::Swap;
                app.prompt_buffer.clear();
            }
        }
        KeyCode::Right => app.select_next(),
        KeyCode::Left => app.select_prev(),
        KeyCode::Char('1') => app.select_slot(0),
        KeyCode::Char('2') => app.select_slot(1),
        KeyCode::Char('3') => app.select_slot(2),
        KeyCode::Char('4') => app.select_slot(3),
        KeyCode::Char('5') => app.select_slot(4),
        KeyCode::Char('6') => app.select_slot(5),
        _ => {}
    }
}
