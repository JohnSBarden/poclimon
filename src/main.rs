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
use ratatui_image::{picker::Picker, protocol::Protocol, Image, Resize};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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

// ── Memory budget ──────────────────────────────────────────────────────────────
//
// v0.0.2 stored frames TWICE: once in `Animator.{idle,eat,sleep}_anim.frames`
// and once in `CreatureSlot.cached_*`.  At scale=6 each frame was 240×336×4 ≈
// 323 KB, so 3 anims × ~6 frames × 2 copies ≈ 11.6 MB per creature × 5 = 58 MB.
//
// v0.0.3 fixes:
//   1. Scale default 6 → 3:  frames shrink by 6²/3² = 4×.
//   2. Frame cap at 8 per animation: limits long PMDCollab animations.
//   3. `Animation` is now timing-only — no pixel data. Frames live exclusively
//      in `CreatureSlot::cached_*`.  Double-storage eliminated.
//   4. Protocol pre-encoding: switch from StatefulProtocol (stores full
//      DynamicImage in the protocol) to Protocol (stores only encoded halfblock
//      data). All frames are encoded once for a given Rect and reused every
//      frame, eliminating alloc/free churn.
//
// Expected working set at scale=3, ≤8 frames, 5 creatures:
//   ~40 KB/encoded-frame × 8 frames × 3 anims × 5 creatures ≈ 4.8 MB total.
//   Zero DynamicImage copies held in protocol objects.

/// Maximum frames to cache per animation.  If the sprite sheet has more,
/// we sample evenly-spaced frames so the animation still looks smooth.
const MAX_CACHED_FRAMES: usize = 8;

// ──────────────────────────────────────────────────────────────────────────────

/// Maximum number of simultaneous creature slots in the pen.
/// Used to compute fixed column widths so adding a creature never
/// shifts existing columns (which would invalidate all encoded Protocols).
const MAX_SLOTS: usize = 6;

// ── Notification system ────────────────────────────────────────────────────────

/// Maximum number of notifications to keep in the deque at once.
const MAX_NOTIFICATIONS: usize = 5;

/// How long (seconds) before a notification expires from the display.
const NOTIF_TTL_SECS: u64 = 8;

/// Severity level for an in-TUI notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotifLevel {
    // Info is currently used in tests and reserved for future informational
    // messages (e.g., successful sprite downloads).
    #[allow(dead_code)]
    Info,
    Warn,
    Error,
}

/// A single in-TUI notification message.
struct Notification {
    message: String,
    level: NotifLevel,
    created_at: Instant,
}

// ──────────────────────────────────────────────────────────────────────────────

/// A single creature slot in the shared-pen display.
///
/// Pixel data lives here; the animator only knows timing/state.
struct CreatureSlot {
    creature_id: u32,
    creature_name: String,
    animator: Animator,
    /// Pre-scaled, normalized frames for the Idle animation.
    cached_idle: Vec<image::DynamicImage>,
    /// Pre-scaled, normalized frames for the Eat animation.
    cached_eat: Vec<image::DynamicImage>,
    /// Pre-scaled, normalized frames for the Sleep animation.
    cached_sleep: Vec<image::DynamicImage>,
    /// Pre-encoded Protocol objects, indexed by [state_index][frame_index].
    /// state 0 = Idle, 1 = Eat, 2 = Sleep.
    /// `None` entries mean encoding failed for that frame (fallback shown).
    /// Rebuilt whenever `encoded_rect` changes (terminal resize or first render).
    encoded_frames: [Vec<Option<Protocol>>; 3],
    /// The `Rect` these protocols were encoded for. `None` means not yet encoded.
    encoded_rect: Option<Rect>,
}

impl CreatureSlot {
    fn new(creature_id: u32, creature_name: String) -> Self {
        Self {
            creature_id,
            creature_name,
            animator: Animator::new(),
            cached_idle: Vec::new(),
            cached_eat: Vec::new(),
            cached_sleep: Vec::new(),
            encoded_frames: [Vec::new(), Vec::new(), Vec::new()],
            encoded_rect: None,
        }
    }
}

struct App {
    config: GameConfig,
    slots: Vec<CreatureSlot>,
    selected: usize,
    running: bool,
    /// Index into `creatures::ROSTER` used by the `A` key to track
    /// the next creature to add (so it cycles rather than repeating).
    next_add_index: usize,
    /// In-TUI notification messages (replaces eprintln! during TUI operation).
    notifications: VecDeque<Notification>,
}

impl App {
    fn new(config: GameConfig) -> Self {
        let slots: Vec<CreatureSlot> = config
            .roster
            .iter()
            .map(|(id, name)| CreatureSlot::new(*id, name.clone()))
            .collect();

        Self {
            config,
            slots,
            selected: 0,
            running: true,
            next_add_index: 0,
            notifications: VecDeque::new(),
        }
    }

    /// Post a notification to the in-TUI message log.
    ///
    /// Displayed in the status+notifications panel. If the deque is at
    /// capacity, the oldest entry is dropped to make room.
    fn notify(&mut self, level: NotifLevel, message: impl Into<String>) {
        if self.notifications.len() >= MAX_NOTIFICATIONS {
            self.notifications.pop_front();
        }
        self.notifications.push_back(Notification {
            message: message.into(),
            level,
            created_at: Instant::now(),
        });
    }

    /// Expire notifications older than `ttl`.
    ///
    /// Separated from `update_all_displays` so tests can pass a custom TTL.
    fn expire_notifications(&mut self, ttl: Duration) {
        self.notifications.retain(|n| n.created_at.elapsed() < ttl);
    }

    /// Load sprites for all creatures currently in the roster.
    ///
    /// Errors and sprite warnings are posted as notifications rather than
    /// written to stderr (which would corrupt the TUI canvas).
    fn load_all_sprites(&mut self) {
        for i in 0..self.slots.len() {
            match load_slot_sprites(&mut self.slots[i], self.config.scale) {
                Ok(warnings) => {
                    for w in warnings {
                        self.notify(NotifLevel::Warn, w);
                    }
                }
                Err(e) => {
                    let name = self.slots[i].creature_name.clone();
                    self.notify(
                        NotifLevel::Error,
                        format!("Failed to load {}: {}", name, e),
                    );
                }
            }
        }
    }

    /// Tick all animators and expire stale notifications.
    ///
    /// Protocol encoding is deferred to `render_pen` where the actual
    /// `Rect` is known — avoids wasted allocations before the first draw.
    fn update_all_displays(&mut self) {
        for slot in &mut self.slots {
            slot.animator.tick();
        }
        self.expire_notifications(Duration::from_secs(NOTIF_TTL_SECS));
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

    /// Add the next available creature (not already in roster) to the end.
    ///
    /// Cycles through `creatures::ROSTER` in order, skipping IDs already
    /// present.  Does nothing when all creatures are already in the roster
    /// or the roster is already at the display limit (6 slots).
    fn add_creature(&mut self) {
        // Cap at 6 for the pen renderer.
        if self.slots.len() >= 6 {
            return;
        }

        let current_ids: std::collections::HashSet<u32> =
            self.slots.iter().map(|s| s.creature_id).collect();

        // Find the next creature not already in the roster, starting from
        // `next_add_index` and wrapping around ROSTER once.
        let roster = creatures::ROSTER;
        let start = self.next_add_index % roster.len();
        let candidate = (start..roster.len())
            .chain(0..start)
            .find(|&i| !current_ids.contains(&roster[i].id));

        let Some(idx) = candidate else {
            // All ROSTER creatures are already on screen.
            return;
        };

        let creature = &roster[idx];
        self.next_add_index = (idx + 1) % roster.len();

        let mut slot = CreatureSlot::new(creature.id, creature.name.to_string());
        match load_slot_sprites(&mut slot, self.config.scale) {
            Ok(warnings) => {
                for w in warnings {
                    self.notify(NotifLevel::Warn, w);
                }
                // Protocol will be built on first render pass in render_pen.
                self.slots.push(slot);
            }
            Err(e) => {
                self.notify(
                    NotifLevel::Error,
                    format!("Failed to add {}: {}", creature.name, e),
                );
            }
        }
    }

    /// Remove the currently selected slot from the roster.
    ///
    /// Silently does nothing if the roster would drop below 1 creature.
    fn remove_selected(&mut self) {
        if self.slots.len() <= 1 {
            return;
        }
        self.slots.remove(self.selected);
        // Keep `selected` in bounds.
        if self.selected >= self.slots.len() {
            self.selected = self.slots.len() - 1;
        }
    }

    /// Cycle the creature in the selected slot through all `creatures::ROSTER`
    /// entries, wrapping around. This may download and cache new sprites.
    fn cycle_selected_creature(&mut self) {
        let Some(slot) = self.slots.get(self.selected) else {
            return;
        };

        let current_id = slot.creature_id;
        let roster = creatures::ROSTER;

        let current_pos = roster
            .iter()
            .position(|c| c.id == current_id)
            .unwrap_or(0);

        let next_pos = (current_pos + 1) % roster.len();
        let next = &roster[next_pos];

        let mut new_slot = CreatureSlot::new(next.id, next.name.to_string());
        match load_slot_sprites(&mut new_slot, self.config.scale) {
            Ok(warnings) => {
                for w in warnings {
                    self.notify(NotifLevel::Warn, w);
                }
                // Protocol will be built on first render pass in render_pen.
                self.slots[self.selected] = new_slot;
            }
            Err(e) => {
                self.notify(
                    NotifLevel::Error,
                    format!("Failed to swap to {}: {}", next.name, e),
                );
            }
        }
    }
}

// ── Sprite loading ─────────────────────────────────────────────────────────────

/// Cap a frame list to at most `MAX_CACHED_FRAMES`, selecting evenly-spaced
/// frames so the animation remains representative.
///
/// Also truncates `durations` to match `frames` in case they differ (defensive).
fn cap_frames(
    frames: Vec<image::DynamicImage>,
    durations: Vec<u32>,
) -> (Vec<image::DynamicImage>, Vec<u32>) {
    // Align lengths defensively.
    let n = frames.len().min(durations.len());
    let mut frames = frames;
    let mut durations = durations;
    frames.truncate(n);
    durations.truncate(n);

    if n <= MAX_CACHED_FRAMES {
        return (frames, durations);
    }

    // Pick MAX_CACHED_FRAMES evenly-spaced indices.
    let cap = MAX_CACHED_FRAMES;
    let indices: Vec<usize> = (0..cap).map(|i| i * n / cap).collect();
    let capped_frames: Vec<image::DynamicImage> =
        indices.iter().map(|&i| frames[i].clone()).collect();
    let capped_durations: Vec<u32> = indices.iter().map(|&i| durations[i]).collect();
    (capped_frames, capped_durations)
}

/// Download, parse, and cache all animation frames for a single slot.
///
/// Frames are pre-scaled by `scale` and normalized to the Idle animation's
/// canonical dimensions so the render loop never has to resize.
/// Frames live only in `slot.cached_*`; the `Animator` holds timing only.
///
/// Returns a Vec of non-fatal warning strings (e.g., a missing animation
/// sheet that was replaced with a fallback). These are shown as in-TUI
/// notifications rather than written to stderr.
fn load_slot_sprites(slot: &mut CreatureSlot, scale: u32) -> Result<Vec<String>> {
    let (anim_data_path, sheets, warnings) = sprite::download_all_sprites(slot.creature_id)?;

    let xml = std::fs::read_to_string(&anim_data_path)?;
    let anim_infos = anim_data::parse_anim_data(&xml);

    // Load Idle first to establish the canonical frame size.
    let (idle_frames, idle_timing, idle_w, idle_h) =
        load_and_scale_animation("Idle", &sheets, &anim_infos, scale, None)?;

    let (eat_frames, eat_timing, _, _) =
        load_and_scale_animation("Eat", &sheets, &anim_infos, scale, Some((idle_w, idle_h)))?;

    let (sleep_frames, sleep_timing, _, _) =
        load_and_scale_animation("Sleep", &sheets, &anim_infos, scale, Some((idle_w, idle_h)))?;

    // Store frames exclusively in the slot cache.
    slot.cached_idle = idle_frames;
    slot.cached_eat = eat_frames;
    slot.cached_sleep = sleep_frames;

    // Give the animator timing-only Animation objects (no pixel data).
    slot.animator = Animator::new();
    slot.animator.load_animations(idle_timing, eat_timing, sleep_timing);

    // Invalidate encoded frames so the first render re-encodes for the actual Rect.
    slot.encoded_rect = None;
    slot.encoded_frames = [Vec::new(), Vec::new(), Vec::new()];

    Ok(warnings)
}

/// Load an animation, pre-scale its frames by `scale`, cap to
/// `MAX_CACHED_FRAMES`, then normalize to `canonical_size` (if provided).
///
/// Returns `(frames, timing_animation, frame_width, frame_height)`.
/// The returned `Animation` is timing-only — no pixel data.
fn load_and_scale_animation(
    anim_name: &str,
    sheets: &[(String, PathBuf)],
    anim_infos: &HashMap<String, AnimInfo>,
    scale: u32,
    canonical_size: Option<(u32, u32)>,
) -> Result<(Vec<image::DynamicImage>, Animation, u32, u32)> {
    let sheet_path = sheets
        .iter()
        .find(|(name, _)| name == anim_name)
        .map(|(_, path)| path);

    let anim_info = anim_infos.get(anim_name);

    let (raw_frames, raw_durations) = match (sheet_path, anim_info) {
        (Some(path), Some(info)) => {
            let sheet = image::ImageReader::open(path)?.decode()?;
            let frames = sprite_sheet::extract_frames(&sheet, info);
            if frames.is_empty() {
                let fallback = sprite::fallback::create_fallback_frame()?;
                (vec![fallback], vec![20u32])
            } else {
                let durations = info.durations.clone();
                (frames, durations)
            }
        }
        _ => {
            let fallback = sprite::fallback::create_fallback_frame()?;
            (vec![fallback], vec![20u32])
        }
    };

    // Step 1: scale by the display scale factor (Nearest-neighbor, RGBA8).
    let scaled: Vec<image::DynamicImage> = raw_frames
        .into_iter()
        .map(|f| {
            let (w, h) = (f.width() * scale, f.height() * scale);
            image::DynamicImage::ImageRgba8(image::imageops::resize(
                &f,
                w,
                h,
                image::imageops::FilterType::Nearest,
            ))
        })
        .collect();

    // Step 2: cap to MAX_CACHED_FRAMES (evenly-spaced sampling if needed).
    let (capped_frames, capped_durations) = cap_frames(scaled, raw_durations);

    // Record dimensions after scaling (before optional normalization).
    let scaled_w = capped_frames.first().map(|f| f.width()).unwrap_or(0);
    let scaled_h = capped_frames.first().map(|f| f.height()).unwrap_or(0);

    // Step 3: normalize to the canonical size if provided.
    let final_frames = match canonical_size {
        Some((cw, ch)) => sprite_sheet::normalize_frames(capped_frames, cw, ch),
        None => capped_frames,
    };

    let out_w = final_frames.first().map(|f| f.width()).unwrap_or(scaled_w);
    let out_h = final_frames.first().map(|f| f.height()).unwrap_or(scaled_h);

    // Build a timing-only Animation aligned to the final frame count.
    let timing = Animation::new(final_frames.len(), &capped_durations);

    Ok((final_frames, timing, out_w, out_h))
}

// ── Protocol encoding ──────────────────────────────────────────────────────────

/// Pre-encode all animation frames for a slot into non-stateful `Protocol`
/// objects sized for `area`.
///
/// Called lazily from `render_pen` whenever the render `Rect` changes
/// (terminal resize) or on the first render.  After this call, rendering
/// a frame is a cheap table lookup — no DynamicImage copies, no alloc/free
/// churn.
///
/// Memory: each `Protocol::Halfblocks` stores only `Vec<HalfBlock>` + a
/// `Rect`, no source image.  8 frames × 3 states × 6 slots ≈ 5.8 MB total.
fn encode_all_frames(slot: &mut CreatureSlot, picker: &Picker, area: Rect) {
    // Collect each state's encoded frames sequentially to avoid simultaneous
    // shared+mutable borrows of `slot`.
    let idle_encoded: Vec<Option<Protocol>> = slot
        .cached_idle
        .iter()
        .map(|img| picker.new_protocol(img.clone(), area, Resize::Fit(None)).ok())
        .collect();

    let eat_encoded: Vec<Option<Protocol>> = slot
        .cached_eat
        .iter()
        .map(|img| picker.new_protocol(img.clone(), area, Resize::Fit(None)).ok())
        .collect();

    let sleep_encoded: Vec<Option<Protocol>> = slot
        .cached_sleep
        .iter()
        .map(|img| picker.new_protocol(img.clone(), area, Resize::Fit(None)).ok())
        .collect();

    slot.encoded_frames = [idle_encoded, eat_encoded, sleep_encoded];
    slot.encoded_rect = Some(area);
}

// ── Application entry point ────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = Cli::parse();

    let config = if let Some(name) = &args.creature {
        // Quick override — single creature
        match GameConfig::from_creature_name(name) {
            Ok(c) => c,
            Err(e) => {
                // TUI not yet active; eprintln! is safe here.
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

    // TUI is now active — sprite load failures become in-TUI notifications
    // rather than eprintln! (which would corrupt the alternate screen).
    app.load_all_sprites();

    let res = run_app(&mut terminal, &mut app, &mut picker);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        // TUI has been torn down; eprintln! is safe here.
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
        app.update_all_displays();
        terminal.draw(|f| ui(f, app, picker))?;

        if event::poll(frame_duration)?
            && let Event::Key(KeyEvent {
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
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    app.add_creature();
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    app.remove_selected();
                }
                KeyCode::Tab => {
                    app.cycle_selected_creature();
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

// ── UI layout ──────────────────────────────────────────────────────────────────

fn ui(f: &mut Frame<'_>, app: &mut App, picker: &mut Picker) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Title bar
        Constraint::Min(10),   // Pen (shared creature canvas)
        Constraint::Length(5), // Status + notifications (3 inner rows)
        Constraint::Length(3), // Help bar
    ])
    .split(f.area());

    // Collect data before mutable borrows.
    let selected_name: String = app
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
            .title("⚡ PoCLImon v0.0.3"),
    );
    f.render_widget(title, chunks[0]);

    // Gather status info before mutable borrow.
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

    // Shared pen — all creatures on one open canvas.
    render_pen(f, chunks[1], app, picker);

    // Status + notification panel.
    // Line 0: current creature state.
    // Lines 1–2: most recent notifications, newest first.
    let mut status_lines = vec![Line::from(vec![
        Span::raw(format!("{}: ", selected_name)),
        Span::styled(state_label, Style::default().fg(status_color)),
    ])];

    for notif in app.notifications.iter().rev().take(2) {
        let (prefix, color) = match notif.level {
            NotifLevel::Error => ("[Error] ", Color::Red),
            NotifLevel::Warn => ("[Warn]  ", Color::Yellow),
            NotifLevel::Info => ("[Info]  ", Color::DarkGray),
        };
        status_lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(color)),
            Span::styled(notif.message.clone(), Style::default().fg(color)),
        ]));
    }

    let status = Paragraph::new(status_lines).block(Block::default().borders(Borders::ALL));
    f.render_widget(status, chunks[2]);

    // Help bar
    let help = Paragraph::new(
        "[E]at  [S]leep  [I]dle  [←/→]Select  [1-6]Slot  [A]dd  [R]emove  [Tab]Swap  [Q]uit",
    )
    .style(Style::default().fg(Color::DarkGray))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[3]);
}

// ── Shared pen rendering ───────────────────────────────────────────────────────

/// Render all creatures in a single shared pen with no internal borders.
///
/// A single outer border wraps the pen area.  Creatures are spaced evenly
/// along the horizontal axis; a name label below each sprite acts as the
/// selection indicator (Yellow + ▲ for selected, DarkGray for others).
///
/// Protocols are pre-encoded on first render for a given `Rect` and reused
/// each frame (zero alloc/free churn during animation).
fn render_pen(f: &mut Frame<'_>, area: Rect, app: &mut App, picker: &mut Picker) {
    let count = app.slots.len();
    if count == 0 {
        return;
    }

    // Single outer border — no inner dividers.
    let block = Block::default().borders(Borders::ALL).title("🌿 Pen");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let selected = app.selected;

    for i in 0..count {
        let creature_region = compute_creature_region(inner, i, count);

        // Reserve the last row for the name label.
        let img_h = creature_region.height.saturating_sub(1);
        let img_area = Rect::new(
            creature_region.x,
            creature_region.y,
            creature_region.width,
            img_h,
        );

        let label_y = creature_region.y + img_h;
        let label_in_bounds = label_y < inner.y + inner.height;

        let slot = &mut app.slots[i];

        // Lazily encode all frames for this slot when the render Rect changes
        // (first render or terminal resize). This is the ONLY place protocols
        // are created — update_all_displays only ticks animators.
        if slot.encoded_rect != Some(img_area) {
            encode_all_frames(slot, picker, img_area);
        }

        let state = slot.animator.state();
        let frame_idx = slot.animator.current_frame_index().unwrap_or(0);
        let state_idx = match state {
            AnimationState::Idle => 0,
            AnimationState::Eating => 1,
            AnimationState::Sleeping => 2,
        };

        // Render sprite (or "Loading…" fallback).
        match slot.encoded_frames[state_idx]
            .get_mut(frame_idx)
            .and_then(|opt| opt.as_mut())
        {
            Some(protocol) => f.render_widget(Image::new(protocol), img_area),
            None => f.render_widget(
                Paragraph::new("Loading…").style(Style::default().fg(Color::DarkGray)),
                img_area,
            ),
        }

        // Name label — selection indicator without borders.
        if label_in_bounds {
            let state_icon = match slot.animator.state() {
                AnimationState::Idle => "",
                AnimationState::Eating => " 🍖",
                AnimationState::Sleeping => " 💤",
            };
            let is_selected = selected == i;
            let (name_color, prefix) = if is_selected {
                (Color::Yellow, "▲ ")
            } else {
                (Color::DarkGray, "  ")
            };
            let label = format!("{}{}{}", prefix, slot.creature_name, state_icon);
            let label_rect = Rect::new(creature_region.x, label_y, creature_region.width, 1);
            f.render_widget(
                Paragraph::new(label).style(Style::default().fg(name_color)),
                label_rect,
            );
        }
    }
}

/// Compute the `Rect` for creature `index` within `pen_inner`.
///
/// Columns are sized for `MAX_SLOTS` regardless of how many creatures are
/// currently active.  This keeps every slot's Rect stable as creatures are
/// added or removed — avoiding encoded `Protocol` invalidation artifacts.
///
/// Terminal resize is a separate path: callers detect area changes and
/// invalidate protocols explicitly via `encoded_rect`.
pub(crate) fn compute_creature_region(pen_inner: Rect, index: usize, total: usize) -> Rect {
    if total == 0 || MAX_SLOTS == 0 {
        return pen_inner;
    }
    if MAX_SLOTS > pen_inner.width as usize {
        return pen_inner;
    }

    // Fixed column width based on MAX_SLOTS — stable regardless of active count.
    let col_w = pen_inner.width / MAX_SLOTS as u16;
    let x = pen_inner.x + index as u16 * col_w;

    // Last active slot extends to pen edge to consume any remainder pixels.
    let w = if index + 1 == total {
        pen_inner.width.saturating_sub(index as u16 * col_w)
    } else {
        col_w
    };

    Rect::new(x, pen_inner.y, w, pen_inner.height)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `App` with a given roster (by creature IDs) without
    /// downloading any sprites. Slots have no cached frames — that's fine for
    /// logic-only tests.
    fn make_app(ids: &[(u32, &str)]) -> App {
        let roster = ids
            .iter()
            .map(|(id, name)| (*id, name.to_string()))
            .collect();
        let config = GameConfig { scale: 1, roster };
        App::new(config)
    }

    // ── select_next / select_prev ─────────────────────────────────────────

    #[test]
    fn test_select_next_wraps() {
        let mut app = make_app(&[(1, "Bulbasaur"), (4, "Charmander"), (7, "Squirtle")]);
        assert_eq!(app.selected, 0);
        app.select_next();
        assert_eq!(app.selected, 1);
        app.select_next();
        assert_eq!(app.selected, 2);
        app.select_next(); // wraps
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_select_prev_wraps() {
        let mut app = make_app(&[(1, "Bulbasaur"), (4, "Charmander")]);
        app.select_prev(); // 0 → last
        assert_eq!(app.selected, 1);
    }

    // ── remove_selected ───────────────────────────────────────────────────

    #[test]
    fn test_remove_selected_basic() {
        let mut app = make_app(&[(1, "Bulbasaur"), (4, "Charmander"), (7, "Squirtle")]);
        app.selected = 1;
        app.remove_selected();
        assert_eq!(app.slots.len(), 2);
        assert_eq!(app.slots[0].creature_id, 1);
        assert_eq!(app.slots[1].creature_id, 7);
        // selected stays at 1 (now pointing to Squirtle)
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn test_remove_selected_last_item_clamps() {
        // Remove the last slot in a two-creature roster; selected must clamp.
        let mut app = make_app(&[(1, "Bulbasaur"), (4, "Charmander")]);
        app.selected = 1;
        app.remove_selected();
        assert_eq!(app.slots.len(), 1);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_remove_selected_noop_when_only_one() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        app.remove_selected();
        assert_eq!(app.slots.len(), 1);
    }

    #[test]
    fn test_remove_all_but_one_via_repeated_removes() {
        let mut app = make_app(&[
            (1, "Bulbasaur"),
            (4, "Charmander"),
            (7, "Squirtle"),
        ]);
        app.selected = 0;
        app.remove_selected(); // removes Bulbasaur
        assert_eq!(app.slots.len(), 2);
        app.remove_selected(); // removes Charmander
        assert_eq!(app.slots.len(), 1);
        app.remove_selected(); // should be a no-op
        assert_eq!(app.slots.len(), 1);
    }

    // ── set_selected_state ────────────────────────────────────────────────

    #[test]
    fn test_set_selected_state_changes_animator() {
        let mut app = make_app(&[(1, "Bulbasaur"), (4, "Charmander")]);
        app.selected = 0;
        app.set_selected_state(AnimationState::Eating);
        assert_eq!(app.slots[0].animator.state(), AnimationState::Eating);
        // Slot 1 unaffected
        assert_eq!(app.slots[1].animator.state(), AnimationState::Idle);
    }

    // ── select_slot ───────────────────────────────────────────────────────

    #[test]
    fn test_select_slot_in_bounds() {
        let mut app = make_app(&[(1, "Bulbasaur"), (4, "Charmander"), (7, "Squirtle")]);
        app.select_slot(2);
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn test_select_slot_out_of_bounds_ignored() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        app.select_slot(5);
        assert_eq!(app.selected, 0); // unchanged
    }

    // ── compute_creature_region (pen layout) ──────────────────────────────
    //
    // Columns are always sized for MAX_SLOTS (6), keeping positions stable
    // as creatures are added/removed (prevents Protocol encoding invalidation).

    #[test]
    fn test_pen_region_uses_fixed_max_slots_column_width() {
        // 120px wide, MAX_SLOTS=6 → col_w = 20px regardless of active count.
        let area = Rect::new(0, 0, 120, 50);
        // With 1 active creature, slot 0 still starts at col 0 width=120 (last slot absorbs remainder).
        let r = compute_creature_region(area, 0, 1);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, 120); // last slot takes rest: 120 - 0*20 = 120
        assert_eq!(r.height, 50);
    }

    #[test]
    fn test_pen_region_slot_x_stable_across_counts() {
        // Adding a creature must NOT change slot 0's x position.
        let area = Rect::new(0, 0, 120, 50);
        let col_w = 120u16 / MAX_SLOTS as u16; // 20
        let r_when_1 = compute_creature_region(area, 0, 1);
        let r_when_2 = compute_creature_region(area, 0, 2);
        let r_when_6 = compute_creature_region(area, 0, 6);
        // x is always 0 * col_w = 0
        assert_eq!(r_when_1.x, 0);
        assert_eq!(r_when_2.x, 0);
        assert_eq!(r_when_6.x, 0);
        // slot 1 x is always 1 * col_w regardless of total
        let r1_when_2 = compute_creature_region(area, 1, 2);
        let r1_when_6 = compute_creature_region(area, 1, 6);
        assert_eq!(r1_when_2.x, col_w);
        assert_eq!(r1_when_6.x, col_w);
    }

    #[test]
    fn test_pen_region_six_creatures_exact() {
        // 120 / MAX_SLOTS(6) = 20 exactly — no remainder.
        let area = Rect::new(2, 4, 120, 40);
        let col_w = 120u16 / MAX_SLOTS as u16;
        for i in 0..6usize {
            let r = compute_creature_region(area, i, 6);
            assert_eq!(r.x, 2 + i as u16 * col_w);
            assert_eq!(r.y, 4);
            assert_eq!(r.height, 40);
        }
        // Last slot absorbs remainder (none here since 120 % 6 == 0)
        let last = compute_creature_region(area, 5, 6);
        assert_eq!(last.width, col_w);
    }

    #[test]
    fn test_pen_region_last_slot_absorbs_remainder() {
        // 100 / MAX_SLOTS(6) = 16 remainder 4. Last active slot absorbs it.
        let area = Rect::new(0, 0, 100, 30);
        let col_w = 100u16 / MAX_SLOTS as u16; // 16
        let last = compute_creature_region(area, 2, 3); // last of 3 active
        let expected_x = 2 * col_w; // 32
        let expected_w = 100 - expected_x; // 68
        assert_eq!(last.x, expected_x);
        assert_eq!(last.width, expected_w);
    }

    #[test]
    fn test_pen_region_non_zero_origin_stable() {
        let area = Rect::new(10, 5, 120, 30);
        let col_w = 120u16 / MAX_SLOTS as u16; // 20
        let r0 = compute_creature_region(area, 0, 2);
        let r1 = compute_creature_region(area, 1, 2);
        assert_eq!(r0.x, 10);
        assert_eq!(r1.x, 10 + col_w);
        assert_eq!(r0.y, 5);
        assert_eq!(r1.y, 5);
        assert_eq!(r0.height, 30);
    }

    #[test]
    fn test_pen_region_total_zero_returns_full_area() {
        let area = Rect::new(0, 0, 80, 24);
        let r = compute_creature_region(area, 0, 0);
        assert_eq!(r, area);
    }

    #[test]
    fn test_pen_region_narrow_terminal_falls_back() {
        // If pen is narrower than MAX_SLOTS pixels, return full area.
        let area = Rect::new(0, 0, 3, 10); // 3 < MAX_SLOTS(6)
        let r = compute_creature_region(area, 0, 1);
        assert_eq!(r, area);
    }

    // ── cap_frames ────────────────────────────────────────────────────────

    fn blank_frames(n: usize) -> Vec<image::DynamicImage> {
        (0..n)
            .map(|_| image::DynamicImage::ImageRgba8(image::RgbaImage::new(4, 4)))
            .collect()
    }

    #[test]
    fn test_cap_frames_under_limit_passthrough() {
        let frames = blank_frames(5);
        let durations = vec![10u32; 5];
        let (cf, cd) = cap_frames(frames, durations);
        assert_eq!(cf.len(), 5);
        assert_eq!(cd.len(), 5);
    }

    #[test]
    fn test_cap_frames_at_limit_passthrough() {
        let frames = blank_frames(MAX_CACHED_FRAMES);
        let durations = vec![10u32; MAX_CACHED_FRAMES];
        let (cf, cd) = cap_frames(frames, durations);
        assert_eq!(cf.len(), MAX_CACHED_FRAMES);
        assert_eq!(cd.len(), MAX_CACHED_FRAMES);
    }

    #[test]
    fn test_cap_frames_over_limit_capped() {
        let n = 20;
        let frames = blank_frames(n);
        let durations = vec![10u32; n];
        let (cf, cd) = cap_frames(frames, durations);
        assert_eq!(cf.len(), MAX_CACHED_FRAMES);
        assert_eq!(cd.len(), MAX_CACHED_FRAMES);
    }

    #[test]
    fn test_cap_frames_evenly_spaced_indices() {
        // Use distinct duration values (0, 1, 2, …) to identify which frames
        // were selected; this makes the sampling assertion concrete.
        let n = 16usize;
        let frames = blank_frames(n);
        let durations: Vec<u32> = (0..n as u32).collect(); // 0, 1, …, 15
        let (cf, cd) = cap_frames(frames, durations);

        assert_eq!(cf.len(), MAX_CACHED_FRAMES); // cap = 8
        // With n=16, cap=8: index i → i * 16 / 8 = 0, 2, 4, 6, 8, 10, 12, 14
        let expected: Vec<u32> = (0..MAX_CACHED_FRAMES as u32)
            .map(|i| i * n as u32 / MAX_CACHED_FRAMES as u32)
            .collect();
        assert_eq!(cd, expected);
    }

    #[test]
    fn test_cap_frames_mismatched_lengths_aligned() {
        // durations has more entries than frames — cap_frames should align.
        let frames = blank_frames(3);
        let durations = vec![10u32, 20, 30, 40, 50]; // 5 durations, 3 frames
        let (cf, cd) = cap_frames(frames, durations);
        assert_eq!(cf.len(), 3);
        assert_eq!(cd.len(), 3);
        assert_eq!(cd, vec![10, 20, 30]);
    }

    // ── notifications ─────────────────────────────────────────────────────

    #[test]
    fn test_notify_adds_to_deque() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        assert!(app.notifications.is_empty());
        app.notify(NotifLevel::Info, "hello world");
        assert_eq!(app.notifications.len(), 1);
        assert_eq!(app.notifications.front().unwrap().message, "hello world");
        assert_eq!(app.notifications.front().unwrap().level, NotifLevel::Info);
    }

    #[test]
    fn test_notify_all_levels() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        app.notify(NotifLevel::Info, "info");
        app.notify(NotifLevel::Warn, "warn");
        app.notify(NotifLevel::Error, "error");
        assert_eq!(app.notifications.len(), 3);
        let levels: Vec<NotifLevel> = app.notifications.iter().map(|n| n.level).collect();
        assert_eq!(levels, vec![NotifLevel::Info, NotifLevel::Warn, NotifLevel::Error]);
    }

    #[test]
    fn test_notify_max_capacity_drops_oldest() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        // Fill to capacity
        for i in 0..MAX_NOTIFICATIONS {
            app.notify(NotifLevel::Info, format!("msg {}", i));
        }
        assert_eq!(app.notifications.len(), MAX_NOTIFICATIONS);
        assert_eq!(app.notifications.front().unwrap().message, "msg 0");

        // Adding one more should drop "msg 0"
        app.notify(NotifLevel::Warn, format!("msg {}", MAX_NOTIFICATIONS));
        assert_eq!(app.notifications.len(), MAX_NOTIFICATIONS);
        assert_eq!(app.notifications.front().unwrap().message, "msg 1");
        assert_eq!(
            app.notifications.back().unwrap().message,
            format!("msg {}", MAX_NOTIFICATIONS)
        );
    }

    #[test]
    fn test_notifications_expire_with_zero_ttl() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        app.notify(NotifLevel::Error, "oops");
        app.notify(NotifLevel::Warn, "hmm");
        assert_eq!(app.notifications.len(), 2);
        // Duration::ZERO forces all notifications to be "expired" since
        // elapsed() >= 0 and 0 < 0 is false → retain keeps nothing.
        app.expire_notifications(Duration::ZERO);
        assert_eq!(app.notifications.len(), 0);
    }

    #[test]
    fn test_notifications_not_expired_with_long_ttl() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        app.notify(NotifLevel::Info, "stays");
        // Very long TTL — notifications should NOT be expired
        app.expire_notifications(Duration::from_secs(9999));
        assert_eq!(app.notifications.len(), 1);
    }
}
