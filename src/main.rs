mod anim_data;
mod animation;
mod app;
mod cli;
mod config;
mod creature;
mod creatures;
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
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use ratatui_image::picker::Picker;
use std::io;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    let config = if let Some(name) = &args.creature {
        // Quick override — single creature
        match GameConfig::from_creature_name(name) {
            Ok(c) => c,
            Err(e) => {
                // TUI not yet active; eprintln! is safe here.
                eprintln!("Warning: {e} — using default");
                GameConfig::default()
            }
        }
    } else if let Some(path) = args.config {
        GameConfig::load(path)?
    } else {
        GameConfig::load_default().unwrap_or_default()
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16)));

    let mut app = App::new(config);

    // Start sprite loads in background threads — game loop renders immediately
    // while sprites arrive. Slots show "Loading…" until their worker completes.
    app.start_background_loads();

    let res = run_app(&mut terminal, &mut app, &mut picker);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        // TUI has been torn down; eprintln! is safe here.
        eprintln!("Error: {e}");
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    picker: &mut Picker,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame_duration = std::time::Duration::from_millis(50);

    while app.running {
        app.update_all_displays();
        terminal.draw(|f| ui::ui(f, app, picker, env!("CARGO_PKG_VERSION")))?;

        if event::poll(frame_duration)?
            && let Event::Key(KeyEvent {
                code,
                kind: KeyEventKind::Press,
                ..
            }) = event::read()?
        {
            match code {
                // ── Prompt intercept — handles all keys when a prompt is open ──
                _ if app.prompt_mode != app::PromptMode::None => {
                    match code {
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
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            if app.prompt_buffer.len() < 4 {
                                app.prompt_buffer.push(c);
                            }
                        }
                        _ => {} // ignore other keys while prompt is active
                    }
                }
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
    }
    Ok(())
}
