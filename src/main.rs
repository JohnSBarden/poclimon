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
use config::{GameConfig, MAX_ACTIVE_CREATURES};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use ratatui_image::{Image, Resize, picker::Picker, protocol::Protocol};
use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

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
#[cfg(test)]
const MAX_SLOTS: usize = MAX_ACTIVE_CREATURES;

/// Fixed sprite render size in terminal cells. All sprites are this size
/// regardless of pen dimensions. 32×32 gives a clear, consistent look.
const SPRITE_W: u16 = 32;
const SPRITE_H: u16 = 10; // was 16 — tighter area, label sits close to sprite

const LABEL_H: u16 = 3; // compact bordered plate: top border + 1 row + bottom border
const LABEL_OVERLAP: u16 = 0; // keep readable by hugging sprite edge, not overlapping pixels
const COLLISION_MIN_PENETRATION: f32 = 0.75;
const OVERLAP_STACK_THRESHOLD: f32 = 0.60;
const RECALL_TICKS: u8 = 18;
const RECALL_FLASH_SHRINK_DELAY_TICKS: u8 = 10;

/// Optional debug log sink enabled by `POCLIMON_DEBUG_LOG=/path/to/file`.
/// Logs are append-only and intentionally low-level for render/movement triage.
fn debug_log(msg: impl AsRef<str>) {
    static DEBUG_FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let sink = DEBUG_FILE.get_or_init(|| {
        let path = std::env::var("POCLIMON_DEBUG_LOG").ok()?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        Some(Mutex::new(file))
    });
    let Some(file_lock) = sink else {
        return;
    };
    let Ok(mut file) = file_lock.lock() else {
        return;
    };
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = writeln!(file, "{} {}", ts_ms, msg.as_ref());
}

fn sprite_stack_h(sprite_h: u16) -> u16 {
    sprite_h + LABEL_H - LABEL_OVERLAP
}

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

enum SwapWorkerResult {
    Loaded {
        slot: Box<CreatureSlot>,
        warnings: Vec<String>,
    },
    Failed(String),
}

struct SwapTransition {
    slot_index: usize,
    recall_ticks: u8,
    target_name: String,
    worker_rx: Receiver<SwapWorkerResult>,
    worker_result: Option<SwapWorkerResult>,
}

struct AddTransition {
    target_name: String,
    worker_rx: Receiver<SwapWorkerResult>,
    worker_result: Option<SwapWorkerResult>,
}

// ──────────────────────────────────────────────────────────────────────────────

/// A single creature slot in the shared-pen display.
///
/// Pixel data lives here; the animator only knows timing/state.
struct CreatureSlot {
    creature_id: u32,
    creature_name: String,
    animator: Animator,
    /// Pre-scaled, normalized frames for the Idle animation, indexed by direction.
    /// [dir_idx][frame_idx] where dir: 0=Down, 1=Left, 2=Up, 3=Right
    cached_idle: [Vec<image::DynamicImage>; 4],
    /// Pre-scaled, normalized frames for the Eat animation, indexed by direction.
    cached_eat: [Vec<image::DynamicImage>; 4],
    /// Pre-scaled, normalized frames for the Sleep animation, indexed by direction.
    cached_sleep: [Vec<image::DynamicImage>; 4],
    /// Pre-scaled, normalized frames for recall animation, preferring Spin and
    /// falling back to Rotate, then Idle.
    cached_recall: [Vec<image::DynamicImage>; 4],
    /// Pre-encoded Protocol objects, indexed by [state_index][dir_index][frame_index].
    /// state 0 = Idle, 1 = Eat, 2 = Sleep, 3 = Recall.
    /// dir: 0=Down, 1=Left, 2=Up, 3=Right.
    /// `None` entries mean encoding failed for that frame (fallback shown).
    /// Rebuilt whenever `encoded_rect` changes (terminal resize or first render).
    encoded_frames: [[Vec<Option<Protocol>>; 4]; 4],
    /// The size `Rect` (position 0,0) these protocols were encoded for.
    /// `None` means not yet encoded. Position-independent — re-encode only on resize.
    encoded_rect: Option<Rect>,
    /// Current X position in terminal cells, relative to pen_inner.x.
    pub pos_x: f32,
    /// Current Y position in terminal cells, relative to pen_inner.y.
    pub pos_y: f32,
    /// Horizontal velocity in cells per 50ms tick.
    pub vel_x: f32,
    /// Vertical velocity in cells per 50ms tick.
    pub vel_y: f32,
    /// Current direction index: 0=Down, 1=Left, 2=Up, 3=Right.
    pub current_dir: usize,
    /// Ticks remaining holding the current direction before picking a new one.
    /// When it hits 0, pick a new velocity and reset to a random 2-8 second hold.
    pub dir_hold_ticks: u32,
    /// Ticks remaining in a direction-change pause (standing idle before walking).
    /// While > 0, position does NOT update. Reset to 0 when walking resumes.
    pub pause_ticks: u32,
    /// Whether to face Down while paused between direction changes.
    /// Enabled with a small probability to make idle stances feel less rigid.
    pub pause_face_down: bool,
    /// Cooldown (ticks) before accepting another velocity-driven facing change.
    /// Reduces rapid direction jitter when collisions/walls cause tiny flips.
    pub dir_cooldown_ticks: u8,
}

impl CreatureSlot {
    fn new(creature_id: u32, creature_name: String) -> Self {
        Self {
            creature_id,
            creature_name,
            animator: Animator::new(),
            cached_idle: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            cached_eat: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            cached_sleep: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            cached_recall: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            encoded_frames: std::array::from_fn(|_| std::array::from_fn(|_| Vec::new())),
            encoded_rect: None,
            pos_x: 0.0,
            pos_y: 0.0,
            vel_x: 0.0,
            vel_y: 0.0,
            current_dir: 0,
            dir_hold_ticks: 0,
            pause_ticks: 0,
            pause_face_down: false,
            dir_cooldown_ticks: 0,
        }
    }

    /// Update position and movement timers for one 50ms tick.
    ///
    /// `is_moving` should be `true` only when the creature is in the Idle animation
    /// state. Eating/sleeping creatures have timers ticked but position frozen.
    pub fn update_position(
        &mut self,
        pen_w: u16,
        pen_h: u16,
        sprite_w: u16,
        sprite_h: u16,
        is_moving: bool,
    ) {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        // ── Direction-change pause ───────────────────────────────────────────
        // Creature stands still briefly when switching direction.
        if self.dir_cooldown_ticks > 0 {
            self.dir_cooldown_ticks -= 1;
        }
        if self.pause_ticks > 0 {
            self.pause_ticks -= 1;
            if self.pause_ticks == 0 {
                // Resume movement facing the heading we'll actually move in.
                self.current_dir = velocity_to_dir(self.vel_x, self.vel_y);
                self.dir_cooldown_ticks = 3;
                self.pause_face_down = false;
                debug_log(format!(
                    "pause_end id={} dir={} vx={:.3} vy={:.3}",
                    self.creature_id, self.current_dir, self.vel_x, self.vel_y
                ));
            } else if self.pause_face_down {
                self.current_dir = 0;
            }
            return; // Frozen in place — no movement or timer updates this tick.
        }

        // ── Direction hold timer ─────────────────────────────────────────────
        // When the hold timer hits 0, pick a new direction.
        if self.dir_hold_ticks == 0 {
            let new_vx = rng.gen_range(-0.4_f32..=0.4);
            let new_vy = rng.gen_range(-0.4_f32..=0.4);

            // Apply minimum speed so creatures don't stall.
            let new_vx = if new_vx.abs() < 0.12 {
                0.18 * new_vx.signum()
            } else {
                new_vx
            };
            let new_vy = if new_vy.abs() < 0.12 {
                0.18 * new_vy.signum()
            } else {
                new_vy
            };

            // Check whether the new direction is different from the old one.
            let old_dir = velocity_to_dir(self.vel_x, self.vel_y);
            let new_dir = velocity_to_dir(new_vx, new_vy);
            if new_dir != old_dir {
                // Pause for 1–2 seconds (20–40 ticks at 50ms each).
                self.pause_ticks = rng.gen_range(20_u32..40);
                self.pause_face_down = rng.gen_bool(0.30);
                if self.pause_face_down {
                    self.current_dir = 0;
                } else {
                    self.current_dir = old_dir;
                }
                debug_log(format!(
                    "heading_change id={} old_dir={} new_dir={} hold={} pause={} face_down={}",
                    self.creature_id,
                    old_dir,
                    new_dir,
                    self.dir_hold_ticks,
                    self.pause_ticks,
                    self.pause_face_down
                ));
            } else {
                self.pause_face_down = false;
            }

            self.vel_x = new_vx;
            self.vel_y = new_vy;

            // Lock direction for the entire heading — only update here, never from live velocity.
            if self.pause_ticks == 0 {
                self.current_dir = velocity_to_dir(new_vx, new_vy);
                self.dir_cooldown_ticks = 3;
                debug_log(format!(
                    "heading_apply id={} dir={} vx={:.3} vy={:.3}",
                    self.creature_id, self.current_dir, self.vel_x, self.vel_y
                ));
            }

            // Hold this direction for 2–8 seconds (40–160 ticks).
            self.dir_hold_ticks = rng.gen_range(40_u32..160);
        } else {
            self.dir_hold_ticks -= 1;
        }

        // ── Position update (only when in Idle animation) ────────────────────
        if !is_moving {
            return; // Eating/sleeping: timers tick but position is frozen.
        }

        self.pos_x += self.vel_x;
        self.pos_y += self.vel_y;

        // Bounce off pen walls.
        let max_x = (pen_w as f32 - sprite_w as f32).max(0.0);
        let max_y =
            (pen_h as f32 - sprite_h as f32 - LABEL_H as f32 + LABEL_OVERLAP as f32).max(0.0);

        if self.pos_x < 0.0 {
            self.pos_x = 0.0;
            self.vel_x = self.vel_x.abs();
            debug_log(format!(
                "wall_bounce id={} axis=x dir={} vx={:.3} vy={:.3}",
                self.creature_id, self.current_dir, self.vel_x, self.vel_y
            ));
        }
        if self.pos_x > max_x {
            self.pos_x = max_x;
            self.vel_x = -self.vel_x.abs();
            debug_log(format!(
                "wall_bounce id={} axis=x dir={} vx={:.3} vy={:.3}",
                self.creature_id, self.current_dir, self.vel_x, self.vel_y
            ));
        }
        if self.pos_y < 0.0 {
            self.pos_y = 0.0;
            self.vel_y = self.vel_y.abs();
            debug_log(format!(
                "wall_bounce id={} axis=y dir={} vx={:.3} vy={:.3}",
                self.creature_id, self.current_dir, self.vel_x, self.vel_y
            ));
        }
        if self.pos_y > max_y {
            self.pos_y = max_y;
            self.vel_y = -self.vel_y.abs();
            debug_log(format!(
                "wall_bounce id={} axis=y dir={} vx={:.3} vy={:.3}",
                self.creature_id, self.current_dir, self.vel_x, self.vel_y
            ));
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
    swap_transition: Option<SwapTransition>,
    add_transition: Option<AddTransition>,
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
            swap_transition: None,
            add_transition: None,
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
                    self.notify(NotifLevel::Error, format!("Failed to load {name}: {e}"));
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
        self.update_swap_transition();
        self.update_add_transition();
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

    fn transition_slot_index(&self) -> Option<usize> {
        self.swap_transition.as_ref().map(|t| t.slot_index)
    }

    fn has_background_load(&self) -> bool {
        self.swap_transition.is_some() || self.add_transition.is_some()
    }

    /// Add the next available creature (not already in roster) to the end.
    ///
    /// Cycles through `creatures::ROSTER` in order, skipping IDs already
    /// present.  Does nothing when all creatures are already in the roster
    /// or the roster is already at the display limit (6 slots).
    fn add_creature(&mut self) {
        if self.has_background_load() {
            self.notify(NotifLevel::Warn, "Please wait for the current load to finish");
            return;
        }
        // Cap at 6 for the pen renderer.
        if self.slots.len() >= MAX_ACTIVE_CREATURES {
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

        let target_id = creature.id;
        let target_name = creature.name.to_string();
        let worker_target_name = target_name.clone();
        let scale = self.config.scale;
        let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
        std::thread::spawn(move || {
            let mut slot = CreatureSlot::new(target_id, worker_target_name);
            let msg = match load_slot_sprites(&mut slot, scale) {
                Ok(warnings) => SwapWorkerResult::Loaded {
                    slot: Box::new(slot),
                    warnings,
                },
                Err(e) => SwapWorkerResult::Failed(e.to_string()),
            };
            let _ = tx.send(msg);
        });

        self.add_transition = Some(AddTransition {
            target_name,
            worker_rx: rx,
            worker_result: None,
        });
    }

    /// Remove the currently selected slot from the roster.
    ///
    /// Silently does nothing if the roster would drop below 1 creature.
    fn remove_selected(&mut self) {
        if self.has_background_load() {
            self.notify(NotifLevel::Warn, "Please wait for the current load to finish");
            return;
        }
        if self.slots.len() <= 1 {
            return;
        }
        self.slots.remove(self.selected);
        // Keep `selected` in bounds.
        if self.selected >= self.slots.len() {
            self.selected = self.slots.len() - 1;
        }
    }

    /// Poll and advance an in-progress swap transition.
    fn update_swap_transition(&mut self) {
        let mut post_warnings: Vec<String> = Vec::new();
        let mut post_error: Option<String> = None;
        let mut apply_swap: Option<(usize, CreatureSlot)> = None;

        if let Some(transition) = self.swap_transition.as_mut() {
            if transition.worker_result.is_none()
                && let Ok(result) = transition.worker_rx.try_recv()
            {
                transition.worker_result = Some(result);
            }

            if transition.recall_ticks > 0 {
                transition.recall_ticks -= 1;
            }

            if transition.recall_ticks == 0
                && let Some(result) = transition.worker_result.take()
            {
                match result {
                    SwapWorkerResult::Loaded { slot, warnings } => {
                        apply_swap = Some((transition.slot_index, *slot));
                        post_warnings = warnings;
                    }
                    SwapWorkerResult::Failed(err) => {
                        post_error = Some(format!(
                            "Failed to swap to {}: {}",
                            transition.target_name, err
                        ));
                    }
                }
            }
        }

        if let Some((slot_index, slot)) = apply_swap {
            if slot_index < self.slots.len() {
                self.slots[slot_index] = slot;
            }
            self.swap_transition = None;
            for warning in post_warnings {
                self.notify(NotifLevel::Warn, warning);
            }
            return;
        }

        if let Some(err) = post_error {
            self.swap_transition = None;
            self.notify(NotifLevel::Error, err);
        }
    }

    fn update_add_transition(&mut self) {
        let mut post_warnings: Vec<String> = Vec::new();
        let mut post_error: Option<String> = None;
        let mut add_slot: Option<CreatureSlot> = None;

        if let Some(transition) = self.add_transition.as_mut() {
            if transition.worker_result.is_none()
                && let Ok(result) = transition.worker_rx.try_recv()
            {
                transition.worker_result = Some(result);
            }

            if let Some(result) = transition.worker_result.take() {
                match result {
                    SwapWorkerResult::Loaded { slot, warnings } => {
                        add_slot = Some(*slot);
                        post_warnings = warnings;
                    }
                    SwapWorkerResult::Failed(err) => {
                        post_error =
                            Some(format!("Failed to add {}: {}", transition.target_name, err));
                    }
                }
            }
        }

        if let Some(slot) = add_slot {
            self.add_transition = None;
            if self.slots.len() < MAX_ACTIVE_CREATURES {
                self.slots.push(slot);
            } else {
                self.notify(
                    NotifLevel::Warn,
                    "Add completed but roster is already full; result dropped",
                );
            }
            for warning in post_warnings {
                self.notify(NotifLevel::Warn, warning);
            }
            return;
        }

        if let Some(err) = post_error {
            self.add_transition = None;
            self.notify(NotifLevel::Error, err);
        }
    }

    /// Cycle the creature in the selected slot through all `creatures::ROSTER`
    /// entries, wrapping around. Recall animation plays while sprites load in
    /// the background and then the slot swaps without freezing the app.
    fn cycle_selected_creature(&mut self) {
        if self.has_background_load() {
            self.notify(NotifLevel::Warn, "A creature load is already in progress");
            return;
        }

        let Some(slot) = self.slots.get(self.selected) else {
            return;
        };

        let current_id = slot.creature_id;
        let roster = creatures::ROSTER;

        let current_pos = roster.iter().position(|c| c.id == current_id).unwrap_or(0);

        let next_pos = (current_pos + 1) % roster.len();
        let next = &roster[next_pos];
        let selected_index = self.selected;
        let target_id = next.id;
        let target_name = next.name.to_string();
        let worker_target_name = target_name.clone();
        let scale = self.config.scale;

        let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
        std::thread::spawn(move || {
            let mut new_slot = CreatureSlot::new(target_id, worker_target_name);
            let msg = match load_slot_sprites(&mut new_slot, scale) {
                Ok(warnings) => SwapWorkerResult::Loaded {
                    slot: Box::new(new_slot),
                    warnings,
                },
                Err(e) => SwapWorkerResult::Failed(e.to_string()),
            };
            let _ = tx.send(msg);
        });

        self.swap_transition = Some(SwapTransition {
            slot_index: selected_index,
            recall_ticks: RECALL_TICKS,
            target_name,
            worker_rx: rx,
            worker_result: None,
        });
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
///
/// Creatures missing an Eat or Sleep animation (e.g. Articuno, Zapdos, Moltres,
/// Vaporeon) silently fall back to their Idle frames — no yellow "?" placeholder,
/// no size change on state switch, no warning noise.
fn load_slot_sprites(slot: &mut CreatureSlot, scale: u32) -> Result<Vec<String>> {
    let (anim_data_path, sheets, warnings) = sprite::download_all_sprites(slot.creature_id)?;

    let xml = std::fs::read_to_string(&anim_data_path)?;
    let anim_infos = anim_data::parse_anim_data(&xml);

    // PMDCollab direction row indices: 0=Down, 2=Left, 4=Up, 6=Right
    // Our dir_idx mapping:             0=Down, 1=Left, 2=Up, 3=Right
    const DIR_ROWS: [u32; 4] = [0, 2, 4, 6];

    // Load Idle for all 4 directions — use dir 0 (Down) to establish canonical size.
    let (idle_down, idle_timing, idle_w, idle_h, _) =
        load_and_scale_animation("Idle", &sheets, &anim_infos, scale, None, DIR_ROWS[0])?;
    let idle_left = load_and_scale_animation(
        "Idle",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[1],
    )
    .map(|r| r.0)
    .unwrap_or_else(|_| idle_down.clone());
    let idle_up = load_and_scale_animation(
        "Idle",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[2],
    )
    .map(|r| r.0)
    .unwrap_or_else(|_| idle_down.clone());
    let idle_right = load_and_scale_animation(
        "Idle",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[3],
    )
    .map(|r| r.0)
    .unwrap_or_else(|_| idle_down.clone());
    slot.cached_idle = [idle_down.clone(), idle_left, idle_up, idle_right];

    // Try Eat dir 0 first to get fallback status.
    let (eat_down_raw, eat_timing_raw, _, _, eat_fallback) = load_and_scale_animation(
        "Eat",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[0],
    )?;
    let (eat_frames_by_dir, eat_timing) = if eat_fallback {
        // Reuse Idle frames for all 4 directions
        (slot.cached_idle.clone(), idle_timing.clone())
    } else {
        let eat_left = load_and_scale_animation(
            "Eat",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[1],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| eat_down_raw.clone());
        let eat_up = load_and_scale_animation(
            "Eat",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[2],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| eat_down_raw.clone());
        let eat_right = load_and_scale_animation(
            "Eat",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[3],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| eat_down_raw.clone());
        ([eat_down_raw, eat_left, eat_up, eat_right], eat_timing_raw)
    };
    slot.cached_eat = eat_frames_by_dir;

    // Try Sleep dir 0 first to get fallback status.
    let (sleep_down_raw, sleep_timing_raw, _, _, sleep_fallback) = load_and_scale_animation(
        "Sleep",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[0],
    )?;
    let (sleep_frames_by_dir, sleep_timing) = if sleep_fallback {
        // Reuse Idle frames for all 4 directions
        (slot.cached_idle.clone(), idle_timing.clone())
    } else {
        let sleep_left = load_and_scale_animation(
            "Sleep",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[1],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| sleep_down_raw.clone());
        let sleep_up = load_and_scale_animation(
            "Sleep",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[2],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| sleep_down_raw.clone());
        let sleep_right = load_and_scale_animation(
            "Sleep",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[3],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| sleep_down_raw.clone());
        (
            [sleep_down_raw, sleep_left, sleep_up, sleep_right],
            sleep_timing_raw,
        )
    };
    slot.cached_sleep = sleep_frames_by_dir;

    // Recall animation for swap transitions:
    // prefer Spin -> fallback Rotate -> fallback Idle.
    let (recall_frames_by_dir, _recall_name) = {
        let (spin_down, _, _, _, spin_fallback) = load_and_scale_animation(
            "Spin",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[0],
        )?;
        if !spin_fallback {
            let spin_left = load_and_scale_animation(
                "Spin",
                &sheets,
                &anim_infos,
                scale,
                Some((idle_w, idle_h)),
                DIR_ROWS[1],
            )
            .map(|r| r.0)
            .unwrap_or_else(|_| spin_down.clone());
            let spin_up = load_and_scale_animation(
                "Spin",
                &sheets,
                &anim_infos,
                scale,
                Some((idle_w, idle_h)),
                DIR_ROWS[2],
            )
            .map(|r| r.0)
            .unwrap_or_else(|_| spin_down.clone());
            let spin_right = load_and_scale_animation(
                "Spin",
                &sheets,
                &anim_infos,
                scale,
                Some((idle_w, idle_h)),
                DIR_ROWS[3],
            )
            .map(|r| r.0)
            .unwrap_or_else(|_| spin_down.clone());
            ([spin_down, spin_left, spin_up, spin_right], "Spin")
        } else {
            let (rotate_down, _, _, _, rotate_fallback) = load_and_scale_animation(
                "Rotate",
                &sheets,
                &anim_infos,
                scale,
                Some((idle_w, idle_h)),
                DIR_ROWS[0],
            )?;
            if !rotate_fallback {
                let rotate_left = load_and_scale_animation(
                    "Rotate",
                    &sheets,
                    &anim_infos,
                    scale,
                    Some((idle_w, idle_h)),
                    DIR_ROWS[1],
                )
                .map(|r| r.0)
                .unwrap_or_else(|_| rotate_down.clone());
                let rotate_up = load_and_scale_animation(
                    "Rotate",
                    &sheets,
                    &anim_infos,
                    scale,
                    Some((idle_w, idle_h)),
                    DIR_ROWS[2],
                )
                .map(|r| r.0)
                .unwrap_or_else(|_| rotate_down.clone());
                let rotate_right = load_and_scale_animation(
                    "Rotate",
                    &sheets,
                    &anim_infos,
                    scale,
                    Some((idle_w, idle_h)),
                    DIR_ROWS[3],
                )
                .map(|r| r.0)
                .unwrap_or_else(|_| rotate_down.clone());
                (
                    [rotate_down, rotate_left, rotate_up, rotate_right],
                    "Rotate",
                )
            } else {
                (slot.cached_idle.clone(), "Idle")
            }
        }
    };
    slot.cached_recall = recall_frames_by_dir;

    // Give the animator timing-only Animation objects (no pixel data).
    slot.animator = Animator::new();
    slot.animator
        .load_animations(idle_timing, eat_timing, sleep_timing);

    // Invalidate encoded frames so the first render re-encodes for the actual Rect.
    slot.encoded_rect = None;
    slot.encoded_frames = std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));

    // Filter out warnings for animations we gracefully handled via Idle fallback
    // or optional recall animation fallbacks (Spin/Rotate).
    let filtered_warnings = if eat_fallback || sleep_fallback {
        warnings
            .into_iter()
            .filter(|w| {
                let w_lower = w.to_lowercase();
                // Keep warnings that aren't about the animations we handled
                !(eat_fallback && w_lower.contains("eat")
                    || sleep_fallback && w_lower.contains("sleep")
                    || w_lower.contains("spin")
                    || w_lower.contains("rotate"))
            })
            .collect()
    } else {
        warnings
            .into_iter()
            .filter(|w| {
                let w_lower = w.to_lowercase();
                !(w_lower.contains("spin") || w_lower.contains("rotate"))
            })
            .collect()
    };

    Ok(filtered_warnings)
}

/// Load an animation, pre-scale its frames by `scale`, cap to
/// `MAX_CACHED_FRAMES`, then normalize to `canonical_size` (if provided).
///
/// `dir_row` selects which PMDCollab direction row to extract (0=Down, 2=Left,
/// 4=Up, 6=Right). If the sheet doesn't have that row, falls back to row 0.
///
/// Returns `(frames, timing_animation, frame_width, frame_height, is_fallback)`.
/// `is_fallback` is `true` when the animation was missing and a fallback frame
/// was used — callers can substitute Idle frames to avoid a broken placeholder.
/// The returned `Animation` is timing-only — no pixel data.
fn load_and_scale_animation(
    anim_name: &str,
    sheets: &[(String, PathBuf)],
    anim_infos: &HashMap<String, AnimInfo>,
    scale: u32,
    canonical_size: Option<(u32, u32)>,
    dir_row: u32,
) -> Result<(Vec<image::DynamicImage>, Animation, u32, u32, bool)> {
    let sheet_path = sheets
        .iter()
        .find(|(name, _)| name == anim_name)
        .map(|(_, path)| path);

    let anim_info = anim_infos.get(anim_name);

    let (raw_frames, raw_durations, is_fallback) = match (sheet_path, anim_info) {
        (Some(path), Some(info)) => {
            let sheet = image::ImageReader::open(path)?.decode()?;
            // Try requested direction row; fall back to row 0 if out of bounds.
            let mut frames = sprite_sheet::extract_frames(&sheet, info, dir_row);
            if frames.is_empty() && dir_row != 0 {
                frames = sprite_sheet::extract_frames(&sheet, info, 0);
            }
            if frames.is_empty() {
                let fallback = sprite::fallback::create_fallback_frame()?;
                (vec![fallback], vec![20u32], true)
            } else {
                let durations = info.durations.clone();
                (frames, durations, false)
            }
        }
        _ => {
            let fallback = sprite::fallback::create_fallback_frame()?;
            (vec![fallback], vec![20u32], true)
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

    Ok((final_frames, timing, out_w, out_h, is_fallback))
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
/// Encodes all 4 directions (Down/Left/Up/Right) for each state
/// (Idle/Eat/Sleep/Recall), giving `encoded_frames[state][dir][frame]`.
///
/// Memory: each `Protocol::Halfblocks` stores only `Vec<HalfBlock>` + a
/// `Rect`, no source image. 8 frames × 4 dirs × 4 states × 6 slots is bounded
/// and cached per-slot.
fn encode_all_frames(slot: &mut CreatureSlot, picker: &Picker, area: Rect) {
    // Clone caches to avoid simultaneous shared+mutable borrows of `slot`.
    let idle = slot.cached_idle.clone();
    let eat = slot.cached_eat.clone();
    let sleep = slot.cached_sleep.clone();
    let recall = slot.cached_recall.clone();

    let caches: [&[Vec<image::DynamicImage>; 4]; 4] = [&idle, &eat, &sleep, &recall];

    slot.encoded_frames = std::array::from_fn(|state_idx| {
        let cache = caches[state_idx];
        std::array::from_fn(|dir_idx| {
            cache[dir_idx]
                .iter()
                .map(|img| {
                    picker
                        .new_protocol(img.clone(), area, Resize::Fit(None))
                        .ok()
                })
                .collect()
        })
    });
    slot.encoded_rect = Some(area);
}

// ── Direction + collision helpers ─────────────────────────────────────────────

/// Map velocity to a cardinal direction index.
/// Returns: 0=Down, 1=Left, 2=Up, 3=Right
fn velocity_to_dir(vel_x: f32, vel_y: f32) -> usize {
    if vel_x.abs() < 0.01 && vel_y.abs() < 0.01 {
        return 0; // stationary → face down
    }
    // In terminal space: vel_y positive = moving down screen
    let angle = vel_y.atan2(vel_x); // atan2(y, x)
    use std::f32::consts::PI;
    let p4 = PI / 4.0;
    if angle > -p4 && angle <= p4 {
        3 // Right
    } else if angle > p4 && angle <= 3.0 * p4 {
        0 // Down
    } else if angle > -3.0 * p4 && angle <= -p4 {
        2 // Up
    } else {
        1 // Left
    }
}

/// Direction mapping with hysteresis to avoid rapid up/down flips while
/// mostly moving left/right (and vice-versa).
fn stable_velocity_to_dir(vel_x: f32, vel_y: f32, current_dir: usize) -> usize {
    let ax = vel_x.abs();
    let ay = vel_y.abs();
    if ax < 0.01 && ay < 0.01 {
        return current_dir;
    }

    if current_dir == 1 || current_dir == 3 {
        // Keep horizontal facing unless vertical speed clearly dominates.
        if ay >= ax * 1.8 {
            return if vel_y >= 0.0 { 0 } else { 2 };
        }
        return if vel_x >= 0.0 { 3 } else { 1 };
    }
    if current_dir == 0 || current_dir == 2 {
        // Keep vertical facing unless horizontal speed clearly dominates.
        if ax >= ay * 1.8 {
            return if vel_x >= 0.0 { 3 } else { 1 };
        }
        return if vel_y >= 0.0 { 0 } else { 2 };
    }

    velocity_to_dir(vel_x, vel_y)
}

/// Elastic circle collision between all creature pairs.
/// Treats each sprite as a circle with radius = min(sprite_w, sprite_h) / 2 cells.
/// Pushes overlapping pairs apart and reflects velocity along the collision normal.
fn resolve_collisions(
    slots: &mut [CreatureSlot],
    sprite_w: u16,
    sprite_h: u16,
    pen_w: u16,
    pen_h: u16,
) {
    let n = slots.len();
    if n < 2 {
        return;
    }

    let sprite_wf = sprite_w as f32;
    let sprite_hf = sprite_h as f32;

    for i in 0..n {
        for j in (i + 1)..n {
            let left_i = slots[i].pos_x;
            let top_i = slots[i].pos_y;
            let right_i = left_i + sprite_wf;
            let bottom_i = top_i + sprite_hf;

            let left_j = slots[j].pos_x;
            let top_j = slots[j].pos_y;
            let right_j = left_j + sprite_wf;
            let bottom_j = top_j + sprite_hf;

            let overlap_x = right_i.min(right_j) - left_i.max(left_j);
            let overlap_y = bottom_i.min(bottom_j) - top_i.max(top_j);

            if overlap_x <= 0.0 || overlap_y <= 0.0 {
                continue;
            }
            if overlap_x.min(overlap_y) < COLLISION_MIN_PENETRATION {
                // Anti-stacking: when overlap is shallow on one axis but very deep on the other,
                // nudge apart along the shallow axis so one sprite doesn't visually hide the other.
                let wide_overlap_x = overlap_x > sprite_wf * OVERLAP_STACK_THRESHOLD;
                let wide_overlap_y = overlap_y > sprite_hf * OVERLAP_STACK_THRESHOLD;
                if wide_overlap_x && overlap_y > 0.01 {
                    let center_i = top_i + sprite_hf / 2.0;
                    let center_j = top_j + sprite_hf / 2.0;
                    let j_is_below = center_j >= center_i;
                    let push = overlap_y / 2.0 + 0.2;
                    if j_is_below {
                        slots[i].pos_y -= push;
                        slots[j].pos_y += push;
                    } else {
                        slots[i].pos_y += push;
                        slots[j].pos_y -= push;
                    }
                } else if wide_overlap_y && overlap_x > 0.01 {
                    let center_i = left_i + sprite_wf / 2.0;
                    let center_j = left_j + sprite_wf / 2.0;
                    let j_is_right = center_j >= center_i;
                    let push = overlap_x / 2.0 + 0.2;
                    if j_is_right {
                        slots[i].pos_x -= push;
                        slots[j].pos_x += push;
                    } else {
                        slots[i].pos_x += push;
                        slots[j].pos_x -= push;
                    }
                }
                debug_log(format!(
                    "collision_skip_shallow i={} j={} ox={:.2} oy={:.2}",
                    slots[i].creature_id, slots[j].creature_id, overlap_x, overlap_y
                ));
                continue;
            }

            // Resolve along the axis of least penetration to prevent deep overlap.
            if overlap_x <= overlap_y {
                let push = overlap_x / 2.0 + 0.01;
                let center_i = left_i + sprite_wf / 2.0;
                let center_j = left_j + sprite_wf / 2.0;
                let j_is_right = center_j >= center_i;
                if j_is_right {
                    slots[i].pos_x -= push;
                    slots[j].pos_x += push;
                } else {
                    slots[i].pos_x += push;
                    slots[j].pos_x -= push;
                }

                // Bounce away from each other on X.
                if j_is_right {
                    slots[i].vel_x = -slots[i].vel_x.abs();
                    slots[j].vel_x = slots[j].vel_x.abs();
                } else {
                    slots[i].vel_x = slots[i].vel_x.abs();
                    slots[j].vel_x = -slots[j].vel_x.abs();
                }
            } else {
                let push = overlap_y / 2.0 + 0.01;
                let center_i = top_i + sprite_hf / 2.0;
                let center_j = top_j + sprite_hf / 2.0;
                let j_is_below = center_j >= center_i;
                if j_is_below {
                    slots[i].pos_y -= push;
                    slots[j].pos_y += push;
                } else {
                    slots[i].pos_y += push;
                    slots[j].pos_y -= push;
                }

                // Bounce away from each other on Y.
                if j_is_below {
                    slots[i].vel_y = -slots[i].vel_y.abs();
                    slots[j].vel_y = slots[j].vel_y.abs();
                } else {
                    slots[i].vel_y = slots[i].vel_y.abs();
                    slots[j].vel_y = -slots[j].vel_y.abs();
                }
            }

            debug_log(format!(
                "collision i={} j={} ox={:.2} oy={:.2} dir_i={} dir_j={} vix={:.3} viy={:.3} vjx={:.3} vjy={:.3}",
                slots[i].creature_id,
                slots[j].creature_id,
                overlap_x,
                overlap_y,
                slots[i].current_dir,
                slots[j].current_dir,
                slots[i].vel_x,
                slots[i].vel_y,
                slots[j].vel_x,
                slots[j].vel_y
            ));
        }
    }

    // Re-clamp positions to pen bounds after collision resolution.
    // `sprite_h` is the full collision height (sprite + nameplate footprint).
    let max_x = (pen_w as f32 - sprite_w as f32).max(0.0);
    let max_y = (pen_h as f32 - sprite_h as f32).max(0.0);
    for slot in slots.iter_mut() {
        slot.pos_x = slot.pos_x.clamp(0.0, max_x);
        slot.pos_y = slot.pos_y.clamp(0.0, max_y);
    }
}

/// Pick a renderable protocol frame with fallbacks:
/// 1) requested state+dir with wrapped frame index
/// 2) any frame in requested state+dir
/// 3) any direction in requested state
/// 4) any direction in Idle state
fn pick_protocol_index(
    encoded: &[[Vec<Option<Protocol>>; 4]; 4],
    state_idx: usize,
    dir_idx: usize,
    frame_idx: usize,
) -> Option<(usize, usize, usize)> {
    fn pick_from_dir_index(dir_frames: &[Option<Protocol>], frame_idx: usize) -> Option<usize> {
        if dir_frames.is_empty() {
            return None;
        }
        let wrapped = frame_idx % dir_frames.len();
        if dir_frames[wrapped].is_some() {
            return Some(wrapped);
        }
        dir_frames.iter().position(|o| o.is_some())
    }

    for s in [state_idx, 0] {
        if let Some(fi) = pick_from_dir_index(&encoded[s][dir_idx], frame_idx) {
            return Some((s, dir_idx, fi));
        }
        for d in 0..4 {
            if d == dir_idx {
                continue;
            }
            if let Some(fi) = pick_from_dir_index(&encoded[s][d], frame_idx) {
                return Some((s, d, fi));
            }
        }
    }
    None
}

fn maybe_update_facing_from_velocity(slot: &mut CreatureSlot) {
    if !matches!(slot.animator.state(), AnimationState::Idle) || slot.pause_ticks > 0 {
        return;
    }
    if slot.dir_cooldown_ticks > 0 {
        return;
    }
    let speed_sq = slot.vel_x * slot.vel_x + slot.vel_y * slot.vel_y;
    if speed_sq < 0.02 {
        return;
    }
    let new_dir = stable_velocity_to_dir(slot.vel_x, slot.vel_y, slot.current_dir);
    if new_dir != slot.current_dir {
        slot.current_dir = new_dir;
        slot.dir_cooldown_ticks = 5;
        debug_log(format!(
            "facing_update id={} dir={} vx={:.3} vy={:.3}",
            slot.creature_id, slot.current_dir, slot.vel_x, slot.vel_y
        ));
    }
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

    // TUI is now active — sprite load failures become in-TUI notifications
    // rather than eprintln! (which would corrupt the alternate screen).
    app.load_all_sprites();

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
            format!(" [selected: {selected_name}]"),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("⚡ PoCLImon v0.2.1"),
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
        Span::raw(format!("{selected_name}: ")),
        Span::styled(state_label, Style::default().fg(status_color)),
    ])];
    if let Some(transition) = app.swap_transition.as_ref() {
        let action = if transition.recall_ticks > 0 {
            "Recalling"
        } else {
            "Loading"
        };
        status_lines.push(Line::from(vec![
            Span::styled("[Swap]  ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                format!("{action} {}...", transition.target_name),
                Style::default().fg(Color::LightMagenta),
            ),
        ]));
    } else if let Some(transition) = app.add_transition.as_ref() {
        status_lines.push(Line::from(vec![
            Span::styled("[Add]   ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                format!("Loading {}...", transition.target_name),
                Style::default().fg(Color::LightMagenta),
            ),
        ]));
    }

    let notif_rows = if app.swap_transition.is_some() || app.add_transition.is_some() {
        1
    } else {
        2
    };
    for notif in app.notifications.iter().rev().take(notif_rows) {
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
/// A single outer border wraps the pen area. Creatures wander freely using
/// `pos_x`/`pos_y` + `vel_x`/`vel_y`, bouncing off walls, overlapping freely
/// (later slots render on top). Name labels follow each sprite.
///
/// Sprite protocols are encoded at a fixed size `(sprite_w, sprite_h)` at
/// position `(0,0)` — position-independent. They are only re-encoded when
/// the pen size changes (terminal resize). At render time the widget is
/// placed at the creature's current position.
fn render_pen(f: &mut Frame<'_>, area: Rect, app: &mut App, picker: &mut Picker) {
    let count = app.slots.len();
    if count == 0 {
        return;
    }

    // Single outer border — no inner dividers.
    let block = Block::default().borders(Borders::ALL).title("🌿 Pen");
    let pen_inner = block.inner(area);
    f.render_widget(block, area);

    let selected = app.selected;
    let transition_slot_index = app.transition_slot_index();
    let transition_state = app
        .swap_transition
        .as_ref()
        .map(|t| (t.slot_index, t.recall_ticks, t.worker_result.is_some()));

    // Sprite size: fixed constants — same for all creatures regardless of pen dimensions.
    let sprite_w = SPRITE_W;
    let sprite_h = SPRITE_H;

    // Size rect used for protocol encoding (position 0,0 — decoupled from render pos).
    let size_rect = Rect::new(0, 0, SPRITE_W, SPRITE_H);

    // Phase 1: initialize positions, update movement, and set direction for all slots.
    for i in 0..count {
        let slot = &mut app.slots[i];

        // First time this slot enters the pen: randomize position and velocity.
        if slot.encoded_rect.is_none() {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let max_px = (pen_inner.width.saturating_sub(sprite_w)) as f32;
            let max_py = (pen_inner.height.saturating_sub(sprite_stack_h(sprite_h))) as f32;
            slot.pos_x = rng.gen_range(0.0..=max_px.max(0.0));
            // Staggered vertical start: divide pen height into count slots.
            // Each creature gets its own slot so they start spread out vertically.
            let y_step = if count > 1 {
                max_py / (count - 1) as f32
            } else {
                0.0
            };
            let base_y = i as f32 * y_step;
            // Small random jitter (±20% of step) so they don't look rigidly spaced.
            let jitter = rng.gen_range(-y_step * 0.2..=y_step * 0.2_f32);
            slot.pos_y = (base_y + jitter).clamp(0.0, max_py.max(0.0));
            slot.vel_x = rng.gen_range(-0.4..=0.4_f32);
            slot.vel_y = rng.gen_range(-0.4..=0.4_f32);
            if slot.vel_x.abs() < 0.12 {
                slot.vel_x = if slot.vel_x >= 0.0 { 0.18 } else { -0.18 };
            }
            if slot.vel_y.abs() < 0.12 {
                slot.vel_y = if slot.vel_y >= 0.0 { 0.18 } else { -0.18 };
            }
            slot.dir_hold_ticks = rng.gen_range(40_u32..160);
        }

        // Update position for this tick (frozen during eating/sleeping).
        // Direction is locked inside update_position when a new heading is picked.
        let is_moving = matches!(slot.animator.state(), AnimationState::Idle)
            && transition_slot_index != Some(i);
        slot.update_position(
            pen_inner.width,
            pen_inner.height,
            sprite_w,
            sprite_h,
            is_moving,
        );
        if slot.current_dir > 3 {
            debug_log(format!(
                "bad_dir id={} dir={} vx={:.3} vy={:.3}",
                slot.creature_id, slot.current_dir, slot.vel_x, slot.vel_y
            ));
            slot.current_dir = stable_velocity_to_dir(slot.vel_x, slot.vel_y, slot.current_dir);
        }

        // Lazily encode (or re-encode on resize) — compare size only, not position.
        if slot.encoded_rect != Some(size_rect) {
            encode_all_frames(slot, picker, size_rect);
        }
    }

    // Phase 2: resolve creature-to-creature collisions (elastic bounce).
    resolve_collisions(
        &mut app.slots,
        SPRITE_W,
        sprite_stack_h(SPRITE_H),
        pen_inner.width,
        pen_inner.height,
    );

    for slot in &mut app.slots {
        maybe_update_facing_from_velocity(slot);
    }

    // ── Phase 3a: render all sprites ──────────────────────────────────────────────
    for i in 0..count {
        let slot = &mut app.slots[i];

        let state = slot.animator.state();
        let mut frame_idx = slot.animator.current_frame_index().unwrap_or(0);
        let mut state_idx = match state {
            AnimationState::Idle => 0,
            AnimationState::Eating => 1,
            AnimationState::Sleeping => 2,
        };
        let dir_idx = slot.current_dir;

        let render_x = (pen_inner.x + slot.pos_x.round() as u16)
            .min(pen_inner.x + pen_inner.width.saturating_sub(sprite_w));
        let render_y = (pen_inner.y + slot.pos_y.round() as u16)
            .min(pen_inner.y + pen_inner.height.saturating_sub(sprite_stack_h(sprite_h)));
        let mut img_area = Rect::new(render_x, render_y, sprite_w, sprite_h);

        let is_transition_slot = transition_slot_index == Some(i);
        let mut render_waiting_ball = false;
        let mut white_flash = false;
        if let Some((_, recall_ticks, worker_done)) = transition_state
            && is_transition_slot
        {
            if recall_ticks > 0 {
                state_idx = 3; // Recall (Spin/Rotate fallback)
                let elapsed = RECALL_TICKS.saturating_sub(recall_ticks);
                frame_idx = elapsed as usize;
                if elapsed >= RECALL_FLASH_SHRINK_DELAY_TICKS {
                    let shrink_phase = elapsed - RECALL_FLASH_SHRINK_DELAY_TICKS;
                    let shrink_total = (RECALL_TICKS - RECALL_FLASH_SHRINK_DELAY_TICKS).max(1);
                    let scale = 1.0 - (shrink_phase as f32 / shrink_total as f32);
                    let w = ((sprite_w as f32 * scale).round() as u16).clamp(2, sprite_w);
                    let h = ((sprite_h as f32 * scale).round() as u16).clamp(1, sprite_h);
                    let x = render_x + sprite_w.saturating_sub(w) / 2;
                    let y = render_y + sprite_h.saturating_sub(h) / 2;
                    img_area = Rect::new(x, y, w, h);
                    white_flash = shrink_phase % 2 == 0;
                }
            } else if !worker_done {
                render_waiting_ball = true;
            }
        }

        if render_waiting_ball {
            f.render_widget(
                Paragraph::new("⚪")
                    .style(Style::default().fg(Color::LightRed)),
                Rect::new(
                    render_x + sprite_w.saturating_sub(3) / 2,
                    render_y + sprite_h.saturating_sub(1) / 2,
                    3,
                    1,
                ),
            );
            continue;
        }

        if white_flash {
            let flash = Block::default().style(Style::default().bg(Color::White));
            f.render_widget(flash, img_area);
            continue;
        }

        match pick_protocol_index(&slot.encoded_frames, state_idx, dir_idx, frame_idx) {
            Some((picked_state, picked_dir, picked_frame)) => {
                if let Some(protocol) =
                    slot.encoded_frames[picked_state][picked_dir][picked_frame].as_mut()
                {
                    f.render_widget(Image::new(protocol), img_area);
                } else {
                    debug_log(format!(
                        "protocol_race_miss id={} state={} dir={} frame={}",
                        slot.creature_id, picked_state, picked_dir, picked_frame
                    ));
                    f.render_widget(
                        Paragraph::new("Loading…").style(Style::default().fg(Color::DarkGray)),
                        img_area,
                    );
                }
            }
            None => {
                debug_log(format!(
                    "protocol_miss id={} state={} dir={} frame={} lens=[{}/{}/{}/{}]",
                    slot.creature_id,
                    state_idx,
                    dir_idx,
                    frame_idx,
                    slot.encoded_frames[state_idx][0].len(),
                    slot.encoded_frames[state_idx][1].len(),
                    slot.encoded_frames[state_idx][2].len(),
                    slot.encoded_frames[state_idx][3].len()
                ));
                f.render_widget(
                    Paragraph::new("Loading…").style(Style::default().fg(Color::DarkGray)),
                    img_area,
                );
            }
        }
    }

    // ── Phase 3b: render compact bordered nameplates ──────────────────────────────
    for i in 0..count {
        let slot = &app.slots[i];

        let render_x = (pen_inner.x + slot.pos_x.round() as u16)
            .min(pen_inner.x + pen_inner.width.saturating_sub(sprite_w));
        let render_y = (pen_inner.y + slot.pos_y.round() as u16)
            .min(pen_inner.y + pen_inner.height.saturating_sub(sprite_stack_h(sprite_h)));

        let is_selected = selected == i;
        let selected_prefix = if is_selected { "◉ " } else { "" };
        let label_text = format!(
            "{}{} Lv.1",
            selected_prefix,
            slot.creature_name.to_uppercase()
        );
        let label_w = (label_text.chars().count() as u16 + 2).clamp(8, sprite_w);
        let label_x = render_x + (sprite_w.saturating_sub(label_w) / 2);
        let label_y = render_y + sprite_h.saturating_sub(LABEL_OVERLAP);

        if label_y + LABEL_H <= pen_inner.y + pen_inner.height {
            let label_area = Rect::new(label_x, label_y, label_w, LABEL_H);
            let label_color = if is_selected {
                Color::Yellow
            } else {
                Color::Gray
            };
            let block = Block::default().borders(Borders::ALL);
            let inner = block.inner(label_area);
            f.render_widget(block, label_area);
            f.render_widget(
                Paragraph::new(label_text).style(Style::default().fg(label_color)),
                Rect::new(inner.x, inner.y, inner.width, 1),
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
#[cfg(test)]
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
    use std::sync::mpsc;

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
        let mut app = make_app(&[(1, "Bulbasaur"), (4, "Charmander"), (7, "Squirtle")]);
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
        assert_eq!(
            levels,
            vec![NotifLevel::Info, NotifLevel::Warn, NotifLevel::Error]
        );
    }

    #[test]
    fn test_notify_max_capacity_drops_oldest() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        // Fill to capacity
        for i in 0..MAX_NOTIFICATIONS {
            app.notify(NotifLevel::Info, format!("msg {i}"));
        }
        assert_eq!(app.notifications.len(), MAX_NOTIFICATIONS);
        assert_eq!(app.notifications.front().unwrap().message, "msg 0");

        // Adding one more should drop "msg 0"
        app.notify(NotifLevel::Warn, format!("msg {MAX_NOTIFICATIONS}"));
        assert_eq!(app.notifications.len(), MAX_NOTIFICATIONS);
        assert_eq!(app.notifications.front().unwrap().message, "msg 1");
        assert_eq!(
            app.notifications.back().unwrap().message,
            format!("msg {MAX_NOTIFICATIONS}")
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

    #[test]
    fn test_swap_transition_applies_loaded_slot_after_recall() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        let (tx, rx) = mpsc::channel();
        let replacement = CreatureSlot::new(25, "Pikachu".to_string());
        tx.send(SwapWorkerResult::Loaded {
            slot: Box::new(replacement),
            warnings: Vec::new(),
        })
        .unwrap();

        app.swap_transition = Some(SwapTransition {
            slot_index: 0,
            recall_ticks: 1,
            target_name: "Pikachu".to_string(),
            worker_rx: rx,
            worker_result: None,
        });

        app.update_swap_transition();
        assert!(app.swap_transition.is_none());
        assert_eq!(app.slots[0].creature_id, 25);
    }

    #[test]
    fn test_swap_transition_waits_until_recall_ends() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        let (tx, rx) = mpsc::channel();
        let replacement = CreatureSlot::new(7, "Squirtle".to_string());
        tx.send(SwapWorkerResult::Loaded {
            slot: Box::new(replacement),
            warnings: Vec::new(),
        })
        .unwrap();

        app.swap_transition = Some(SwapTransition {
            slot_index: 0,
            recall_ticks: 2,
            target_name: "Squirtle".to_string(),
            worker_rx: rx,
            worker_result: None,
        });

        app.update_swap_transition();
        assert!(app.swap_transition.is_some());
        assert_eq!(app.slots[0].creature_id, 1);

        app.update_swap_transition();
        assert!(app.swap_transition.is_none());
        assert_eq!(app.slots[0].creature_id, 7);
    }

    #[test]
    fn test_add_transition_pushes_loaded_slot() {
        let mut app = make_app(&[(1, "Bulbasaur")]);
        let (tx, rx) = mpsc::channel();
        let addition = CreatureSlot::new(4, "Charmander".to_string());
        tx.send(SwapWorkerResult::Loaded {
            slot: Box::new(addition),
            warnings: Vec::new(),
        })
        .unwrap();

        app.add_transition = Some(AddTransition {
            target_name: "Charmander".to_string(),
            worker_rx: rx,
            worker_result: None,
        });

        app.update_add_transition();
        assert!(app.add_transition.is_none());
        assert_eq!(app.slots.len(), 2);
        assert_eq!(app.slots[1].creature_id, 4);
    }

    // ── movement + collisions ────────────────────────────────────────────

    #[test]
    fn test_pause_face_down_keeps_down_direction() {
        let mut slot = CreatureSlot::new(1, "Bulbasaur".to_string());
        slot.pause_ticks = 3;
        slot.pause_face_down = true;
        slot.current_dir = 2; // up
        slot.vel_x = 0.3;
        slot.vel_y = 0.0;

        slot.update_position(120, 40, SPRITE_W, SPRITE_H, true);

        assert_eq!(slot.pause_ticks, 2);
        assert_eq!(slot.current_dir, 0); // forced down while paused
    }

    #[test]
    fn test_pause_end_resumes_velocity_direction() {
        let mut slot = CreatureSlot::new(1, "Bulbasaur".to_string());
        slot.pause_ticks = 1;
        slot.pause_face_down = true;
        slot.current_dir = 0;
        slot.vel_x = -0.4;
        slot.vel_y = 0.0;

        slot.update_position(120, 40, SPRITE_W, SPRITE_H, true);

        assert_eq!(slot.pause_ticks, 0);
        assert_eq!(slot.current_dir, 1); // left
        assert!(!slot.pause_face_down);
    }

    #[test]
    fn test_collision_resolution_separates_overlapping_sprites() {
        let mut slots = vec![
            CreatureSlot::new(1, "Bulbasaur".to_string()),
            CreatureSlot::new(4, "Charmander".to_string()),
        ];
        slots[0].pos_x = 10.0;
        slots[0].pos_y = 10.0;
        slots[1].pos_x = 20.0; // overlaps on x since SPRITE_W is 32
        slots[1].pos_y = 10.0; // same y band => guaranteed overlap
        slots[0].vel_x = 0.3;
        slots[1].vel_x = -0.3;

        resolve_collisions(&mut slots, SPRITE_W, SPRITE_H, 200, 80);

        let overlap_x = (slots[0].pos_x + SPRITE_W as f32).min(slots[1].pos_x + SPRITE_W as f32)
            - slots[0].pos_x.max(slots[1].pos_x);
        let overlap_y = (slots[0].pos_y + SPRITE_H as f32).min(slots[1].pos_y + SPRITE_H as f32)
            - slots[0].pos_y.max(slots[1].pos_y);
        assert!(
            overlap_x <= 0.0 || overlap_y <= 0.0,
            "sprites still overlap"
        );
    }
}
