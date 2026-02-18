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
//
// Expected working set at scale=3, ≤8 frames, 5 creatures:
//   120×168×4 ≈ 80 KB/frame × 8 frames × 3 anims × 5 creatures ≈ 9.6 MB peak.

/// Maximum frames to cache per animation.  If the sprite sheet has more,
/// we sample evenly-spaced frames so the animation still looks smooth.
const MAX_CACHED_FRAMES: usize = 8;

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
    /// The (state, frame_index) of the last rendered frame.
    /// We only rebuild `current_frame` when this changes.
    last_render_key: Option<(AnimationState, usize)>,
    /// The current image protocol handed to ratatui-image.
    current_frame: Option<StatefulProtocol>,
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
            last_render_key: None,
            current_frame: None,
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
        }
    }

    /// Load sprites for all creatures currently in the roster.
    fn load_all_sprites(&mut self, picker: &mut Picker) -> Result<()> {
        for i in 0..self.slots.len() {
            load_slot_sprites(&mut self.slots[i], self.config.scale)?;
        }
        // Render first frames for all slots.
        self.update_all_displays(picker);
        Ok(())
    }

    /// Tick all animators and rebuild `StatefulProtocol` only when the
    /// displayed frame has actually changed.  This is the hot path —
    /// no image resizing happens here.
    fn update_all_displays(&mut self, picker: &mut Picker) {
        for slot in &mut self.slots {
            slot.animator.tick();

            let state = slot.animator.state();
            let Some(frame_idx) = slot.animator.current_frame_index() else {
                continue;
            };

            let render_key = (state, frame_idx);
            if slot.last_render_key == Some(render_key) {
                // Frame unchanged — no need to recreate the protocol.
                continue;
            }

            let frames = match state {
                AnimationState::Idle => &slot.cached_idle,
                AnimationState::Eating => &slot.cached_eat,
                AnimationState::Sleeping => &slot.cached_sleep,
            };

            if let Some(frame) = frames.get(frame_idx) {
                slot.current_frame =
                    Some(picker.new_resize_protocol(frame.clone()));
                slot.last_render_key = Some(render_key);
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

    /// Add the next available creature (not already in roster) to the end.
    ///
    /// Cycles through `creatures::ROSTER` in order, skipping IDs already
    /// present.  Does nothing when all creatures are already in the roster
    /// or the roster is already at the display limit (6 slots).
    fn add_creature(&mut self, picker: &mut Picker) {
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
        if load_slot_sprites(&mut slot, self.config.scale).is_ok() {
            // Trigger immediate render for the first frame.
            let state = slot.animator.state();
            if let Some(frame_idx) = slot.animator.current_frame_index() {
                let frames = match state {
                    AnimationState::Idle => &slot.cached_idle,
                    AnimationState::Eating => &slot.cached_eat,
                    AnimationState::Sleeping => &slot.cached_sleep,
                };
                if let Some(frame) = frames.get(frame_idx) {
                    slot.current_frame =
                        Some(picker.new_resize_protocol(frame.clone()));
                    slot.last_render_key = Some((state, frame_idx));
                }
            }
            self.slots.push(slot);
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
    fn cycle_selected_creature(&mut self, picker: &mut Picker) {
        let Some(slot) = self.slots.get(self.selected) else {
            return;
        };

        let current_id = slot.creature_id;
        let roster = creatures::ROSTER;

        // Find the current position in ROSTER (fall back to 0 if not found).
        let current_pos = roster
            .iter()
            .position(|c| c.id == current_id)
            .unwrap_or(0);

        let next_pos = (current_pos + 1) % roster.len();
        let next = &roster[next_pos];

        let mut new_slot = CreatureSlot::new(next.id, next.name.to_string());
        if load_slot_sprites(&mut new_slot, self.config.scale).is_ok() {
            // Trigger immediate render.
            let state = new_slot.animator.state();
            if let Some(frame_idx) = new_slot.animator.current_frame_index() {
                let frames = match state {
                    AnimationState::Idle => &new_slot.cached_idle,
                    AnimationState::Eating => &new_slot.cached_eat,
                    AnimationState::Sleeping => &new_slot.cached_sleep,
                };
                if let Some(frame) = frames.get(frame_idx) {
                    new_slot.current_frame =
                        Some(picker.new_resize_protocol(frame.clone()));
                    new_slot.last_render_key = Some((state, frame_idx));
                }
            }
            self.slots[self.selected] = new_slot;
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
fn load_slot_sprites(slot: &mut CreatureSlot, scale: u32) -> Result<()> {
    let (anim_data_path, sheets) = sprite::download_all_sprites(slot.creature_id)?;

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

    // Invalidate any stale render key so the first frame is always drawn.
    slot.last_render_key = None;
    slot.current_frame = None;

    Ok(())
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

// ── Application entry point ────────────────────────────────────────────────────

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
                    app.add_creature(picker);
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    app.remove_selected();
                }
                KeyCode::Tab => {
                    app.cycle_selected_creature(picker);
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

fn ui(f: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Title bar
        Constraint::Min(10),   // Pen (shared creature canvas)
        Constraint::Length(3), // Status bar
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
    render_pen(f, chunks[1], app);

    let status = Paragraph::new(Line::from(vec![
        Span::raw(format!("{}: ", selected_name)),
        Span::styled(state_label, Style::default().fg(status_color)),
    ]))
    .block(Block::default().borders(Borders::ALL));
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
fn render_pen(f: &mut Frame<'_>, area: Rect, app: &mut App) {
    let count = app.slots.len();
    if count == 0 {
        return;
    }

    // Single outer border — no inner dividers.
    let block = Block::default()
        .borders(Borders::ALL)
        .title("🌿 Pen");
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

        // Render sprite (or "Loading…" fallback).
        if let Some(ref mut protocol) = slot.current_frame {
            let image_widget = StatefulImage::default();
            f.render_stateful_widget(image_widget, img_area, protocol);
        } else {
            let fallback = Paragraph::new("Loading…")
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(fallback, img_area);
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

/// Compute the `Rect` for creature `index` of `total` within `pen_inner`.
///
/// Divides the pen width into `total` equal regions; the last region absorbs
/// any remainder pixels from integer division.
pub(crate) fn compute_creature_region(pen_inner: Rect, index: usize, total: usize) -> Rect {
    if total == 0 {
        return pen_inner;
    }
    // Guard: if there are more creatures than pixels, each gets the full area.
    if total > pen_inner.width as usize {
        return pen_inner;
    }

    let region_w = pen_inner.width / total as u16;
    let x = pen_inner.x + index as u16 * region_w;

    // Last creature absorbs remaining width after integer division.
    let w = if index + 1 == total {
        pen_inner.width.saturating_sub(index as u16 * region_w)
    } else {
        region_w
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
        let config = GameConfig {
            scale: 1,
            roster,
        };
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

    #[test]
    fn test_pen_region_single_creature() {
        let area = Rect::new(0, 0, 100, 50);
        let r = compute_creature_region(area, 0, 1);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, 100);
        assert_eq!(r.height, 50);
    }

    #[test]
    fn test_pen_region_two_creatures_even_split() {
        let area = Rect::new(0, 0, 100, 50);
        let r0 = compute_creature_region(area, 0, 2);
        let r1 = compute_creature_region(area, 1, 2);
        assert_eq!(r0.x, 0);
        assert_eq!(r0.width, 50);
        assert_eq!(r1.x, 50);
        assert_eq!(r1.width, 50); // 100 − 1×50 = 50
        assert_eq!(r0.height, 50);
        assert_eq!(r1.height, 50);
    }

    #[test]
    fn test_pen_region_three_creatures_remainder_in_last() {
        // 100 / 3 = 33 remainder 1 → last slot gets 34.
        let area = Rect::new(0, 0, 100, 50);
        let r0 = compute_creature_region(area, 0, 3);
        let r1 = compute_creature_region(area, 1, 3);
        let r2 = compute_creature_region(area, 2, 3);
        assert_eq!(r0.x, 0);
        assert_eq!(r0.width, 33);
        assert_eq!(r1.x, 33);
        assert_eq!(r1.width, 33);
        assert_eq!(r2.x, 66);
        assert_eq!(r2.width, 34); // 100 − 2×33 = 34
    }

    #[test]
    fn test_pen_region_six_creatures() {
        // 120 / 6 = 20 exactly — no remainder.
        let area = Rect::new(2, 4, 120, 40);
        for i in 0..6usize {
            let r = compute_creature_region(area, i, 6);
            assert_eq!(r.x, 2 + i as u16 * 20);
            assert_eq!(r.width, 20);
            assert_eq!(r.y, 4);
            assert_eq!(r.height, 40);
        }
    }

    #[test]
    fn test_pen_region_non_zero_origin() {
        let area = Rect::new(10, 5, 60, 30);
        let r0 = compute_creature_region(area, 0, 2);
        let r1 = compute_creature_region(area, 1, 2);
        assert_eq!(r0.x, 10);
        assert_eq!(r1.x, 40); // 10 + 30
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

    // ── cap_frames ────────────────────────────────────────────────────────

    fn blank_frames(n: usize) -> Vec<image::DynamicImage> {
        (0..n)
            .map(|_| {
                image::DynamicImage::ImageRgba8(image::RgbaImage::new(4, 4))
            })
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
}
