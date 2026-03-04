use crate::animation::Animator;
use image::DynamicImage;
use ratatui_image::protocol::Protocol;
use std::io::Write;
use std::sync::Mutex;

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
pub const MAX_CACHED_FRAMES: usize = 8;

/// Fixed sprite render size in terminal cells. All sprites are this size
/// regardless of pen dimensions.
pub const SPRITE_W: u16 = 32;
/// Sprite height for image-protocol terminals (Kitty/Sixel/iTerm2).
pub const SPRITE_H: u16 = 10;
/// Sprite height for halfblock terminals (Alacritty, macOS Terminal, etc.).
/// 16 rows × 2 pixel-rows/row = 32 pixel rows, matching SPRITE_W=32 columns
/// for a 32×32 "pixel" canvas — enough to recognize creature sprites.
pub const SPRITE_H_HALFBLOCKS: u16 = 16;

pub const LABEL_H: u16 = 4; // top border + 2 content rows + bottom border
pub const LABEL_OVERLAP: u16 = 0; // keep readable by hugging sprite edge, not overlapping pixels
pub const OVERLAP_STACK_THRESHOLD: f32 = 0.60;
pub const RECALL_TICKS: u8 = 18;
pub const RECALL_FLASH_SHRINK_DELAY_TICKS: u8 = 10;

/// A single creature slot in the shared-pen display.
///
/// Pixel data lives here; the animator only knows timing/state.
pub struct CreatureSlot {
    pub creature_id: u32,
    pub creature_name: String,
    pub animator: Animator,
    /// Pre-scaled, normalized frames for the Idle animation, indexed by direction.
    /// [dir_idx][frame_idx] where dir: 0=Down, 1=Left, 2=Up, 3=Right
    pub cached_idle: [Vec<DynamicImage>; 4],
    /// Pre-scaled, normalized frames for the Eat animation, indexed by direction.
    pub cached_eat: [Vec<DynamicImage>; 4],
    /// Pre-scaled, normalized frames for the Sleep animation, indexed by direction.
    pub cached_sleep: [Vec<DynamicImage>; 4],
    /// Pre-scaled, normalized frames for recall animation, preferring Spin and
    /// falling back to Rotate, then Idle.
    pub cached_recall: [Vec<DynamicImage>; 4],
    /// Pre-encoded Protocol objects, indexed by [state_index][dir_index][frame_index].
    /// state 0 = Idle, 1 = Eat, 2 = Sleep, 3 = Recall.
    /// dir: 0=Down, 1=Left, 2=Up, 3=Right.
    /// `None` entries mean encoding failed for that frame (fallback shown).
    /// Rebuilt whenever `encoded_rect` changes (terminal resize or first render).
    pub encoded_frames: [[Vec<Option<Protocol>>; 4]; 4],
    /// The size `Rect` (position 0,0) these protocols were encoded for.
    /// `None` means not yet encoded. Position-independent — re-encode only on resize.
    pub encoded_rect: Option<ratatui::layout::Rect>,
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
    pub fn new(creature_id: u32, creature_name: String) -> Self {
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

/// Optional debug log sink enabled by `POCLIMON_DEBUG_LOG=/path/to/file`.
/// Logs are append-only and intentionally low-level for render/movement triage.
pub fn debug_log(msg: impl AsRef<str>) {
    static DEBUG_FILE: std::sync::OnceLock<Option<Mutex<std::fs::File>>> =
        std::sync::OnceLock::new();
    let sink = DEBUG_FILE.get_or_init(|| {
        let path = std::env::var("POCLIMON_DEBUG_LOG").ok()?;
        let file = std::fs::OpenOptions::new()
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
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = writeln!(file, "{} {}", ts_ms, msg.as_ref());
}

pub fn sprite_stack_h(sprite_h: u16) -> u16 {
    sprite_h + LABEL_H - LABEL_OVERLAP
}

/// Map velocity to a cardinal direction index.
/// Returns: 0=Down, 1=Left, 2=Up, 3=Right
pub fn velocity_to_dir(vel_x: f32, vel_y: f32) -> usize {
    if vel_x.abs() < 0.01 && vel_y.abs() < 0.01 {
        return 0; // stationary → face down
    }
    if vel_x.abs() > vel_y.abs() {
        if vel_x > 0.0 { 3 } else { 1 } // Right : Left
    } else if vel_y > 0.0 {
        2 // Up
    } else {
        0 // Down
    }
}

/// Direction mapping with hysteresis to avoid rapid up/down flips while
/// mostly moving left/right (and vice-versa).
pub fn stable_velocity_to_dir(vel_x: f32, vel_y: f32, current_dir: usize) -> usize {
    let ax = vel_x.abs();
    let ay = vel_y.abs();
    let threshold = 0.12;
    let new_dir = if ax < threshold && ay < threshold {
        current_dir // Keep current if barely moving
    } else if ax > ay {
        if vel_x > 0.0 { 3 } else { 1 }
    } else if vel_y > 0.0 {
        2
    } else {
        0
    };
    // Only change if crossing diagonal threshold
    if new_dir != current_dir {
        new_dir
    } else {
        current_dir
    }
}

/// Update facing direction based on velocity when appropriate.
pub fn maybe_update_facing_from_velocity(slot: &mut CreatureSlot) {
    if !matches!(
        slot.animator.state(),
        crate::animation::AnimationState::Idle
    ) || slot.pause_ticks > 0
    {
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

/// Resolve creature-to-creature collisions using elastic bounce physics.
///
/// For overlapping creatures, pushes them apart along the axis of maximum overlap.
/// Uses a minimum penetration threshold to avoid jitter from near-misses.
pub fn resolve_collisions(
    slots: &mut [CreatureSlot],
    sprite_w: u16,
    sprite_h: u16,
    pen_w: u16,
    pen_h: u16,
) {
    for i in 0..slots.len() {
        for j in (i + 1)..slots.len() {
            let overlap_x = (slots[i].pos_x + sprite_w as f32)
                .min(slots[j].pos_x + sprite_w as f32)
                - slots[i].pos_x.max(slots[j].pos_x);
            let overlap_y = (slots[i].pos_y + sprite_h as f32)
                .min(slots[j].pos_y + sprite_h as f32)
                - slots[i].pos_y.max(slots[j].pos_y);

            if overlap_x > 0.0 && overlap_y > 0.0 {
                let overlap_area = overlap_x * overlap_y;
                let sprite_area = sprite_w as f32 * sprite_h as f32;
                if overlap_area / sprite_area > OVERLAP_STACK_THRESHOLD {
                    continue; // Skip stack resolution — they're meant to overlap when stacked
                }

                // Elastic bounce: push apart along the axis of maximum overlap.
                let push = (overlap_x.max(overlap_y) / 2.0) + 0.01;
                if overlap_x > overlap_y {
                    let center_i = slots[i].pos_x + sprite_w as f32 / 2.0;
                    let center_j = slots[j].pos_x + sprite_w as f32 / 2.0;
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
                    let center_i = slots[i].pos_y + sprite_h as f32 / 2.0;
                    let center_j = slots[j].pos_y + sprite_h as f32 / 2.0;
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

