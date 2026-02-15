//! PoCLImon — A terminal-based Pokémon virtual pet.
//!
//! Run with `cargo run` or `cargo run -- --config path/to/config.json`.

mod app;
mod config;
mod sprite;
mod ui;

use anyhow::Result;
use app::App;
use clap::Parser;
use config::GameConfig;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use ratatui_image::picker::Picker;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(version, about = "PoCLImon - A terminal-based Pokémon virtual pet")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Cli::parse();
    let config = match args.config {
        Some(path) => GameConfig::load(path)?,
        None => GameConfig::default(),
    };

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16)));

    let mut app = App::new(config)?;

    // Load initial sprite
    app.ensure_sprite_loaded(&mut picker);

    // Main loop
    let res = run_app(&mut terminal, &mut app, &mut picker);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("Error: {e}");
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    picker: &mut Picker,
) -> Result<()> {
    while app.running {
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    // Quit
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        app.running = false;
                    }
                    // Navigate roster
                    KeyCode::Left => {
                        app.prev_slot();
                        app.ensure_sprite_loaded(picker);
                    }
                    KeyCode::Right => {
                        app.next_slot();
                        app.ensure_sprite_loaded(picker);
                    }
                    // Slot jump (1-6)
                    KeyCode::Char(c @ '1'..='9') => {
                        let slot = (c as usize) - ('1' as usize);
                        if app.goto_slot(slot) {
                            app.ensure_sprite_loaded(picker);
                        }
                    }
                    // Feed
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        app.current_pet_mut().feed();
                        let name = app.current_pet().entry.display_name().to_string();
                        app.status_message = Some(format!("{name} was fed! 🍔"));
                    }
                    // Pet
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        app.current_pet_mut().pet();
                        let name = app.current_pet().entry.display_name().to_string();
                        app.status_message = Some(format!("{name} was petted! ❤️"));
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
