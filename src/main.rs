mod config;
mod sprite;

use anyhow::Result;
use clap::Parser;
use config::GameConfig;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use ratatui_image::{picker::Picker, StatefulImage, protocol::StatefulProtocol};
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

struct App {
    #[allow(dead_code)]
    config: GameConfig,
    pokemon_image: Option<StatefulProtocol>,
    sprite_path: Option<PathBuf>,
    running: bool,
}

impl App {
    fn new(config: GameConfig) -> Self {
        Self {
            config,
            pokemon_image: None,
            sprite_path: None,
            running: true,
        }
    }

    fn load_sprite(&mut self, picker: &mut Picker) -> Result<()> {
        let sprite_dir = dirs_or_default();
        std::fs::create_dir_all(&sprite_dir)?;

        let sprite_path = sprite_dir.join("pikachu.png");

        // If sprite doesn't exist, try to download it
        if !sprite_path.exists() {
            eprintln!("Downloading Pikachu sprite...");
            match sprite::download_sprite(25, &sprite_path) {
                Ok(()) => {}
                Err(e) => {
                    // Use bundled fallback
                    eprintln!("Download failed ({}), using fallback", e);
                    sprite::create_fallback_sprite(&sprite_path)?;
                }
            }
        }

        let dyn_img = image::ImageReader::open(&sprite_path)?.decode()?;
        self.pokemon_image = Some(picker.new_resize_protocol(dyn_img));
        self.sprite_path = Some(sprite_path);
        Ok(())
    }
}

fn dirs_or_default() -> PathBuf {
    dirs_path().unwrap_or_else(|| PathBuf::from(".poclimon/sprites"))
}

fn dirs_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".poclimon").join("sprites"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
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

    // Create picker - query terminal for graphics protocol support
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| {
        // Fallback to halfblocks if query fails
        Picker::from_fontsize((8, 16))
    });

    let mut app = App::new(config);

    // Load the sprite
    if let Err(e) = app.load_sprite(&mut picker) {
        // We'll show the error in the UI
        eprintln!("Failed to load sprite: {}", e);
    }

    // Main loop
    let res = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    while app.running {
        terminal.draw(|f| ui(f, app))?;

        // Handle input with timeout for ~30fps
        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        app.running = false;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn ui(f: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),  // Title bar
        Constraint::Min(10),   // Sprite area
        Constraint::Length(3), // Help bar
    ])
    .split(f.area());

    // Title
    let title = Paragraph::new(Line::from(vec![
        Span::styled("PoCLImon", Style::default().fg(Color::Yellow)),
        Span::raw(" - "),
        Span::styled("Pikachu", Style::default().fg(Color::LightYellow)),
    ]))
    .block(Block::default().borders(Borders::ALL).title("⚡ PoCLImon"));
    f.render_widget(title, chunks[0]);

    // Sprite area
    let sprite_block = Block::default()
        .borders(Borders::ALL)
        .title("Pokémon Sprite");

    let inner = sprite_block.inner(chunks[1]);
    f.render_widget(sprite_block, chunks[1]);

    if let Some(ref mut protocol) = app.pokemon_image {
        // Center the image in the available area
        let img_area = centered_rect(inner, 40, 20);
        let image_widget = StatefulImage::default();
        f.render_stateful_widget(image_widget, img_area, protocol);
    } else {
        let fallback = Paragraph::new("No sprite loaded. Check ~/.poclimon/sprites/")
            .style(Style::default().fg(Color::Red));
        f.render_widget(fallback, inner);
    }

    // Help bar
    let help = Paragraph::new("Press 'q' or ESC to quit")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[2]);
}

fn centered_rect(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = max_width.min(area.width);
    let height = max_height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
