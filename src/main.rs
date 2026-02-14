mod animation;
mod config;
mod creatures;
mod sprite;

use animation::{AnimationState, Animator};
use anyhow::Result;
use clap::Parser;
use config::GameConfig;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use image::DynamicImage;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, StatefulImage};
use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(version, about = "PoCLImon - A terminal-based creature virtual pet")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Creature name to start with (e.g., pikachu, bulbasaur, charmander, squirtle, eevee)
    #[arg(short = 'n', long)]
    creature: Option<String>,

    /// Creature ID to start with
    #[arg(long)]
    creature_id: Option<u32>,
}

struct App {
    config: GameConfig,
    base_image: Option<DynamicImage>,
    current_frame: Option<StatefulProtocol>,
    running: bool,
    animator: Animator,
    /// Index into the creature roster for cycling.
    roster_index: usize,
}

impl App {
    fn new(config: GameConfig) -> Self {
        // Find matching roster index
        let roster_index = creatures::ROSTER
            .iter()
            .position(|c| c.id == config.creature_id)
            .unwrap_or(0);

        Self {
            config,
            base_image: None,
            current_frame: None,
            running: true,
            animator: Animator::new(),
            roster_index,
        }
    }

    fn display_name(&self) -> &str {
        self.config
            .alias
            .as_deref()
            .unwrap_or(&self.config.creature_name)
    }

    fn load_sprite(&mut self, picker: &mut Picker) -> Result<()> {
        let sprite_dir = dirs_or_default();
        std::fs::create_dir_all(&sprite_dir)?;

        let sprite_name = self.config.creature_name.to_lowercase();
        let sprite_path = sprite_dir.join(format!("{}.png", sprite_name));

        if !sprite_path.exists() {
            eprintln!("Downloading sprite for {}...", self.config.creature_name);
            match sprite::download_sprite(self.config.creature_id, &sprite_path) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Download failed ({}), using fallback", e);
                    sprite::fallback::create_fallback_sprite(&sprite_path)?;
                }
            }
        }

        let dyn_img = image::ImageReader::open(&sprite_path)?.decode()?;
        self.current_frame = Some(picker.new_resize_protocol(dyn_img.clone()));
        self.animator.detect_eyes(&dyn_img);
        self.base_image = Some(dyn_img);
        Ok(())
    }

    /// Switch to a different creature from the roster.
    fn switch_creature(&mut self, index: usize, picker: &mut Picker) {
        if index >= creatures::ROSTER.len() {
            return;
        }
        self.roster_index = index;
        let creature = &creatures::ROSTER[index];
        self.config.creature_id = creature.id;
        self.config.creature_name = creature.name.to_string();
        self.config.alias = None;
        self.animator = Animator::new();

        if let Err(e) = self.load_sprite(picker) {
            eprintln!("Failed to load sprite: {}", e);
        }
    }

    fn next_creature(&mut self, picker: &mut Picker) {
        let next = (self.roster_index + 1) % creatures::ROSTER.len();
        self.switch_creature(next, picker);
    }

    fn prev_creature(&mut self, picker: &mut Picker) {
        let prev = if self.roster_index == 0 {
            creatures::ROSTER.len() - 1
        } else {
            self.roster_index - 1
        };
        self.switch_creature(prev, picker);
    }

    fn update_animation(&mut self, picker: &mut Picker) {
        self.animator.tick();

        if let Some(ref base) = self.base_image {
            let frame = self.animator.render_frame(base);
            self.current_frame = Some(picker.new_resize_protocol(frame));
        }
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

    let config = if let Some(path) = args.config {
        GameConfig::load(path)?
    } else if let Some(name) = &args.creature {
        if let Some(creature) = creatures::find_by_name(name) {
            GameConfig {
                creature_id: creature.id,
                creature_name: creature.name.to_string(),
                alias: None,
            }
        } else {
            eprintln!("Unknown creature '{}', using default", name);
            GameConfig::default()
        }
    } else if let Some(id) = args.creature_id {
        if let Some(creature) = creatures::find_by_id(id) {
            GameConfig {
                creature_id: creature.id,
                creature_name: creature.name.to_string(),
                alias: None,
            }
        } else {
            GameConfig {
                creature_id: id,
                creature_name: format!("Creature #{}", id),
                alias: None,
            }
        }
    } else {
        GameConfig::default()
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| {
        Picker::from_fontsize((8, 16))
    });

    let mut app = App::new(config);

    if let Err(e) = app.load_sprite(&mut picker) {
        eprintln!("Failed to load sprite: {}", e);
    }

    let res = run_app(&mut terminal, &mut app, &mut picker);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    picker: &mut Picker,
) -> Result<()> {
    while app.running {
        let frame_duration = Duration::from_millis(app.animator.frame_rate_ms());
        app.update_animation(picker);
        terminal.draw(|f| ui(f, app))?;

        if event::poll(frame_duration)? {
            if let Event::Key(KeyEvent {
                code,
                kind: KeyEventKind::Press,
                ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        app.running = false;
                    }
                    KeyCode::Char('e') | KeyCode::Char('E') => {
                        app.animator.set_state(AnimationState::Eating);
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        app.animator.set_state(AnimationState::Sleeping);
                    }
                    KeyCode::Char('i') | KeyCode::Char('I') => {
                        app.animator.set_state(AnimationState::Idle);
                    }
                    KeyCode::Right | KeyCode::Char('n') | KeyCode::Char('N') => {
                        app.next_creature(picker);
                    }
                    KeyCode::Left | KeyCode::Char('p') | KeyCode::Char('P') => {
                        app.prev_creature(picker);
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
        Constraint::Length(3), // Title bar
        Constraint::Min(10),  // Sprite area
        Constraint::Length(3), // Status bar
        Constraint::Length(3), // Help bar
    ])
    .split(f.area());

    // Title
    let display_name = app.display_name();
    let roster_info = format!(
        " [{}/{}]",
        app.roster_index + 1,
        creatures::ROSTER.len()
    );
    let title = Paragraph::new(Line::from(vec![
        Span::styled("PoCLImon", Style::default().fg(Color::Yellow)),
        Span::raw(" - "),
        Span::styled(display_name, Style::default().fg(Color::LightYellow)),
        Span::styled(&roster_info, Style::default().fg(Color::DarkGray)),
    ]))
    .block(Block::default().borders(Borders::ALL).title("⚡ PoCLImon v0.0.1"));
    f.render_widget(title, chunks[0]);

    // Sprite area
    let state_label = match app.animator.state() {
        AnimationState::Idle => "Idle",
        AnimationState::Eating => "Nomming 🍖",
        AnimationState::Sleeping => "Sleeping 💤",
    };
    let sprite_block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Creature Sprite — {}", state_label));

    let inner = sprite_block.inner(chunks[1]);
    f.render_widget(sprite_block, chunks[1]);

    if let Some(ref mut protocol) = app.current_frame {
        let img_area = centered_rect(inner, 40, 20);
        let image_widget = StatefulImage::default();
        f.render_stateful_widget(image_widget, img_area, protocol);
    } else {
        let fallback = Paragraph::new("No sprite loaded. Check ~/.poclimon/sprites/")
            .style(Style::default().fg(Color::Red));
        f.render_widget(fallback, inner);
    }

    // Status bar
    let status_color = match app.animator.state() {
        AnimationState::Idle => Color::Green,
        AnimationState::Eating => Color::Yellow,
        AnimationState::Sleeping => Color::Blue,
    };
    let status = Paragraph::new(Line::from(vec![
        Span::raw("State: "),
        Span::styled(state_label, Style::default().fg(status_color)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(status, chunks[2]);

    // Help bar
    let help = Paragraph::new("[E]at  [S]leep  [I]dle  [←/P]rev  [→/N]ext  [Q]uit")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[3]);
}

fn centered_rect(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = max_width.min(area.width);
    let height = max_height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
