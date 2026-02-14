mod anim_data;
mod animation;
mod config;
mod creatures;
mod sprite;
mod sprite_sheet;

use anim_data::AnimInfo;
use animation::{Animation, AnimationState, Animator};
use anyhow::Result;
use clap::Parser;
use config::GameConfig;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
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
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, StatefulImage};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(version, about = "PoCLImon - A terminal-based creature virtual pet")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Quick override: show only this creature (by name)
    #[arg(short = 'n', long)]
    creature: Option<String>,
}

/// A single creature slot in the multi-sprite display.
struct CreatureSlot {
    creature_id: u32,
    creature_name: String,
    animator: Animator,
    current_frame: Option<StatefulProtocol>,
}

struct App {
    config: GameConfig,
    slots: Vec<CreatureSlot>,
    selected: usize,
    running: bool,
}

impl App {
    fn new(config: GameConfig) -> Self {
        let slots: Vec<CreatureSlot> = config
            .roster
            .iter()
            .map(|(id, name)| CreatureSlot {
                creature_id: *id,
                creature_name: name.clone(),
                animator: Animator::new(),
                current_frame: None,
            })
            .collect();

        Self {
            config,
            slots,
            selected: 0,
            running: true,
        }
    }

    /// Load sprites for all creatures in the roster.
    fn load_all_sprites(&mut self, picker: &mut Picker) -> Result<()> {
        for slot in &mut self.slots {
            eprintln!("Downloading sprites for {}...", slot.creature_name);

            let (anim_data_path, sheets) =
                sprite::download_all_sprites(slot.creature_id)?;

            let xml = std::fs::read_to_string(&anim_data_path)?;
            let anim_infos = anim_data::parse_anim_data(&xml);

            let idle_anim = load_animation("Idle", &sheets, &anim_infos)?;
            let eat_anim = load_animation("Eat", &sheets, &anim_infos)?;
            let sleep_anim = load_animation("Sleep", &sheets, &anim_infos)?;

            slot.animator = Animator::new();
            slot.animator.load_animations(idle_anim, eat_anim, sleep_anim);
        }

        // Render first frames
        self.update_all_displays(picker);
        Ok(())
    }

    /// Update all creature displays.
    fn update_all_displays(&mut self, picker: &mut Picker) {
        let scale = self.config.scale;
        for slot in &mut self.slots {
            slot.animator.tick();
            if let Some(frame) = slot.animator.render_frame() {
                let (w, h) = (frame.width(), frame.height());
                let scaled = image::imageops::resize(
                    frame,
                    w * scale,
                    h * scale,
                    image::imageops::FilterType::Nearest,
                );
                slot.current_frame =
                    Some(picker.new_resize_protocol(image::DynamicImage::ImageRgba8(scaled)));
            }
        }
    }

    fn select_next(&mut self) {
        if !self.slots.is_empty() {
            self.selected = (self.selected + 1) % self.slots.len();
        }
    }

    fn select_prev(&mut self) {
        if !self.slots.is_empty() {
            self.selected = if self.selected == 0 {
                self.slots.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    fn select_slot(&mut self, index: usize) {
        if index < self.slots.len() {
            self.selected = index;
        }
    }

    fn set_selected_state(&mut self, state: AnimationState) {
        if let Some(slot) = self.slots.get_mut(self.selected) {
            slot.animator.set_state(state);
        }
    }
}

/// Load an Animation from downloaded sprite sheets and parsed AnimData.
fn load_animation(
    anim_name: &str,
    sheets: &[(String, PathBuf)],
    anim_infos: &HashMap<String, AnimInfo>,
) -> Result<Animation> {
    let sheet_path = sheets
        .iter()
        .find(|(name, _)| name == anim_name)
        .map(|(_, path)| path);

    let anim_info = anim_infos.get(anim_name);

    match (sheet_path, anim_info) {
        (Some(path), Some(info)) => {
            let sheet = image::ImageReader::open(path)?.decode()?;
            let frames = sprite_sheet::extract_frames(&sheet, info);

            if frames.is_empty() {
                let fallback = sprite::fallback::create_fallback_frame()?;
                Ok(Animation::new(vec![fallback], &[20]))
            } else {
                Ok(Animation::new(frames, &info.durations))
            }
        }
        _ => {
            let fallback = sprite::fallback::create_fallback_frame()?;
            Ok(Animation::new(vec![fallback], &[20]))
        }
    }
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let config = if let Some(name) = &args.creature {
        // Quick override — single creature
        match GameConfig::from_creature_name(name) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Warning: {} — using default", e);
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

    if let Err(e) = app.load_all_sprites(&mut picker) {
        eprintln!("Failed to load sprites: {}", e);
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
    let frame_duration = Duration::from_millis(50);

    while app.running {
        app.update_all_displays(picker);
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
                        app.set_selected_state(AnimationState::Eating);
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        app.set_selected_state(AnimationState::Sleeping);
                    }
                    KeyCode::Char('i') | KeyCode::Char('I') => {
                        app.set_selected_state(AnimationState::Idle);
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
    }
    Ok(())
}

fn ui(f: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Title bar
        Constraint::Min(10),  // Creature area
        Constraint::Length(3), // Status bar
        Constraint::Length(3), // Help bar
    ])
    .split(f.area());

    // Title
    let selected_name = app
        .slots
        .get(app.selected)
        .map(|s| s.creature_name.clone())
        .unwrap_or_else(|| "???".to_string());
    let title = Paragraph::new(Line::from(vec![
        Span::styled("PoCLImon", Style::default().fg(Color::Yellow)),
        Span::raw(" — "),
        Span::styled(
            format!("{} creatures", app.slots.len()),
            Style::default().fg(Color::LightYellow),
        ),
        Span::styled(
            format!(" [selected: {}]", selected_name),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("⚡ PoCLImon v0.1.0"),
    );
    f.render_widget(title, chunks[0]);

    // Gather status info before mutable borrow
    let state_label = app
        .slots
        .get(app.selected)
        .map(|s| match s.animator.state() {
            AnimationState::Idle => "Idle",
            AnimationState::Eating => "Nomming 🍖",
            AnimationState::Sleeping => "Sleeping 💤",
        })
        .unwrap_or("—");
    let status_color = app
        .slots
        .get(app.selected)
        .map(|s| match s.animator.state() {
            AnimationState::Idle => Color::Green,
            AnimationState::Eating => Color::Yellow,
            AnimationState::Sleeping => Color::Blue,
        })
        .unwrap_or(Color::White);

    // Creature area — layout depends on count
    render_creature_grid(f, chunks[1], app);
    let status = Paragraph::new(Line::from(vec![
        Span::raw(format!("{}: ", &selected_name)),
        Span::styled(state_label, Style::default().fg(status_color)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(status, chunks[2]);

    // Help bar
    let help = Paragraph::new("[E]at  [S]leep  [I]dle  [←/→]Select  [1-6]Slot  [Q]uit")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[3]);
}

/// Render the creature grid based on how many creatures are active.
fn render_creature_grid(f: &mut Frame<'_>, area: Rect, app: &mut App) {
    let count = app.slots.len();
    if count == 0 {
        return;
    }

    match count {
        1 => {
            render_creature_slot(f, area, &mut app.slots[0], app.selected == 0);
        }
        2 => {
            let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            for (i, col) in cols.iter().enumerate() {
                if let Some(slot) = app.slots.get_mut(i) {
                    render_creature_slot(f, *col, slot, app.selected == i);
                }
            }
        }
        3 => {
            let cols = Layout::horizontal([
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ])
            .split(area);
            for (i, col) in cols.iter().enumerate() {
                if let Some(slot) = app.slots.get_mut(i) {
                    render_creature_slot(f, *col, slot, app.selected == i);
                }
            }
        }
        4..=6 => {
            // 2 rows: top row gets ceil(count/2), bottom gets the rest
            let top_count = (count + 1) / 2;
            let bot_count = count - top_count;
            let rows = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            // Top row
            let top_constraints: Vec<Constraint> = (0..top_count)
                .map(|_| Constraint::Ratio(1, top_count as u32))
                .collect();
            let top_cols = Layout::horizontal(top_constraints).split(rows[0]);
            for (i, col) in top_cols.iter().enumerate() {
                if let Some(slot) = app.slots.get_mut(i) {
                    render_creature_slot(f, *col, slot, app.selected == i);
                }
            }

            // Bottom row
            if bot_count > 0 {
                let bot_constraints: Vec<Constraint> = (0..bot_count)
                    .map(|_| Constraint::Ratio(1, bot_count as u32))
                    .collect();
                let bot_cols = Layout::horizontal(bot_constraints).split(rows[1]);
                for (i, col) in bot_cols.iter().enumerate() {
                    let idx = top_count + i;
                    if let Some(slot) = app.slots.get_mut(idx) {
                        render_creature_slot(f, *col, slot, app.selected == idx);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Render a single creature slot.
fn render_creature_slot(
    f: &mut Frame<'_>,
    area: Rect,
    slot: &mut CreatureSlot,
    selected: bool,
) {
    let border_color = if selected {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let state_icon = match slot.animator.state() {
        AnimationState::Idle => "",
        AnimationState::Eating => " 🍖",
        AnimationState::Sleeping => " 💤",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(format!("{}{}", slot.creature_name, state_icon));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(ref mut protocol) = slot.current_frame {
        let img_area = centered_rect(
            inner,
            inner.width.saturating_sub(2),
            inner.height.saturating_sub(1),
        );
        let image_widget = StatefulImage::default();
        f.render_stateful_widget(image_widget, img_area, protocol);
    } else {
        let fallback = Paragraph::new("Loading...")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(fallback, inner);
    }
}

fn centered_rect(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = max_width.min(area.width);
    let height = max_height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
