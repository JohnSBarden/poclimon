use crate::animation::Animator;
use image::DynamicImage;
use ratatui_image::protocol::Protocol;
use std::io::Write;
use std::sync::Mutex;

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

/// Cardinal direction a creature is facing.
///
/// The index value corresponds to the direction row used for array lookups:
/// 0=Down, 1=Left, 2=Up, 3=Right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Down = 0,
    Left = 1,
    Up = 2,
    Right = 3,
}

impl Direction {
    /// Return the array index for this direction (0–3).
    pub fn as_index(self) -> usize {
        self as usize
    }
}

/// All pixel frames and terminal-encoded protocols for a single creature slot.
///
/// Collects the 5 animation caches (Idle/Eat/Sleep/Recall/Hop) and their
/// pre-encoded `Protocol` objects into one place, reducing `CreatureSlot` from
/// 7 separate cache fields to a single named group.
pub struct SpriteCache {
    pub idle: [Vec<DynamicImage>; 4],
    pub eat: [Vec<DynamicImage>; 4],
    pub sleep: [Vec<DynamicImage>; 4],
    pub recall: [Vec<DynamicImage>; 4],
    pub hop: [Vec<DynamicImage>; 4],
    /// Pre-encoded Protocol objects indexed by [state_index][dir_index][frame_index].
    /// state 0=Idle, 1=Eat, 2=Sleep, 3=Recall, 4=Playing (Hop).
    /// dir: 0=Down, 1=Left, 2=Up, 3=Right.
    pub encoded: [[Vec<Option<Protocol>>; 4]; 5],
    /// The size `Rect` (position 0,0) these protocols were encoded for.
    /// `None` means not yet encoded. Position-independent — re-encode only on resize.
    pub encoded_rect: Option<ratatui::layout::Rect>,
}

impl Default for SpriteCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SpriteCache {
    pub fn new() -> Self {
        Self {
            idle: std::array::from_fn(|_| Vec::new()),
            eat: std::array::from_fn(|_| Vec::new()),
            sleep: std::array::from_fn(|_| Vec::new()),
            recall: std::array::from_fn(|_| Vec::new()),
            hop: std::array::from_fn(|_| Vec::new()),
            encoded: std::array::from_fn(|_| std::array::from_fn(|_| Vec::new())),
            encoded_rect: None,
        }
    }
}

/// A single creature slot in the shared-pen display.
///
/// Pixel data lives here; the animator only knows timing/state.
pub struct CreatureSlot {
    pub creature_id: u32,
    /// Unique identifier for this slot instance. Generated randomly on creation
    /// so two slots with the same Pokédex ID can still be distinguished.
    pub slot_id: u64,
    pub creature_name: String,
    pub animator: Animator,
    /// All sprite frames and terminal-encoded protocols for this slot.
    pub sprites: SpriteCache,
    /// Current experience points earned by this creature (whole numbers).
    /// Increases while the creature is Eating or Playing. Resets to 0 on level-up.
    pub xp: u32,
    /// Sub-integer XP accumulator.
    ///
    /// XP is earned at rates like 2xp/sec, accrued 0.1xp per 50ms tick. Storing
    /// only `u32` would floor every tick to 0 and XP would never increase.
    /// Instead we bank the fractional part here and only "cash out" whole points
    /// into `xp` once the accumulator crosses 1.0.
    pub xp_frac: f32,
    /// Current level (starts at 1). Increases when `xp` hits the level threshold.
    /// Threshold = `50 * level` (50 xp for level 1→2, 100 for 2→3, etc.).
    pub level: u32,
    /// Seconds the creature has been continuously in an XP-earning state (Eating
    /// or Playing). Used to throttle XP gain over time — rate slows at 10s and
    /// stops at 40s. Resets to 0.0 whenever the state switches away from
    /// Eating/Playing.
    pub anim_active_secs: f32,
    /// Current X position in terminal cells, relative to pen_inner.x.
    pub pos_x: f32,
    /// Current Y position in terminal cells, relative to pen_inner.y.
    pub pos_y: f32,
    /// Horizontal velocity in cells per 50ms tick.
    pub vel_x: f32,
    /// Vertical velocity in cells per 50ms tick.
    pub vel_y: f32,
    /// Current facing direction.
    pub current_dir: Direction,
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
            slot_id: rand::random::<u64>(),
            creature_name,
            animator: Animator::new(),
            sprites: SpriteCache::new(),
            pos_x: 0.0,
            pos_y: 0.0,
            vel_x: 0.0,
            vel_y: 0.0,
            current_dir: Direction::Down,
            dir_hold_ticks: 0,
            pause_ticks: 0,
            pause_face_down: false,
            dir_cooldown_ticks: 0,
            // XP and leveling — start at level 1, no XP yet.
            xp: 0,
            xp_frac: 0.0,
            level: 1,
            anim_active_secs: 0.0,
        }
    }

    /// Accrue XP for one 50ms tick. Returns the new level if the creature leveled up.
    ///
    /// Only accrues XP while in an XP-earning state (Eating or Playing).
    /// XP rate decays over time: 2xp/s for first 10s, 1xp/s to 40s, then 0.
    pub fn tick_xp(&mut self) -> Option<u32> {
        const TICK_SECS: f32 = 0.05;

        let is_xp_state = matches!(
            self.animator.state(),
            crate::animation::AnimationState::Eating | crate::animation::AnimationState::Playing
        );
        if !is_xp_state {
            return None;
        }

        self.anim_active_secs += TICK_SECS;

        let xp_rate = if self.anim_active_secs <= 10.0 {
            2.0_f32
        } else if self.anim_active_secs <= 40.0 {
            1.0_f32
        } else {
            0.0_f32
        };

        self.xp_frac += xp_rate * TICK_SECS;
        let whole = self.xp_frac.floor() as u32;
        if whole > 0 {
            self.xp = self.xp.saturating_add(whole);
            self.xp_frac -= whole as f32;
        }

        let threshold = 50 * self.level;
        if self.xp >= threshold {
            self.xp = 0;
            self.level += 1;
            return Some(self.level);
        }

        None
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
                    self.creature_id,
                    self.current_dir.as_index(),
                    self.vel_x,
                    self.vel_y
                ));
            } else if self.pause_face_down {
                self.current_dir = Direction::Down;
            }
            return; // Frozen in place — no movement or timer updates this tick.
        }

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

            let old_dir = velocity_to_dir(self.vel_x, self.vel_y);
            let new_dir = velocity_to_dir(new_vx, new_vy);
            if new_dir != old_dir {
                // Pause for 1–2 seconds (20–40 ticks at 50ms each).
                self.pause_ticks = rng.gen_range(20_u32..40);
                self.pause_face_down = rng.gen_bool(0.30);
                if self.pause_face_down {
                    self.current_dir = Direction::Down;
                } else {
                    self.current_dir = old_dir;
                }
                debug_log(format!(
                    "heading_change id={} old_dir={} new_dir={} hold={} pause={} face_down={}",
                    self.creature_id,
                    old_dir.as_index(),
                    new_dir.as_index(),
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
                    self.creature_id,
                    self.current_dir.as_index(),
                    self.vel_x,
                    self.vel_y
                ));
            }

            // Hold this direction for 2–8 seconds (40–160 ticks).
            self.dir_hold_ticks = rng.gen_range(40_u32..160);
        } else {
            self.dir_hold_ticks -= 1;
        }

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
                self.creature_id,
                self.current_dir.as_index(),
                self.vel_x,
                self.vel_y
            ));
        }
        if self.pos_x > max_x {
            self.pos_x = max_x;
            self.vel_x = -self.vel_x.abs();
            debug_log(format!(
                "wall_bounce id={} axis=x dir={} vx={:.3} vy={:.3}",
                self.creature_id,
                self.current_dir.as_index(),
                self.vel_x,
                self.vel_y
            ));
        }
        if self.pos_y < 0.0 {
            self.pos_y = 0.0;
            self.vel_y = self.vel_y.abs();
            debug_log(format!(
                "wall_bounce id={} axis=y dir={} vx={:.3} vy={:.3}",
                self.creature_id,
                self.current_dir.as_index(),
                self.vel_x,
                self.vel_y
            ));
        }
        if self.pos_y > max_y {
            self.pos_y = max_y;
            self.vel_y = -self.vel_y.abs();
            debug_log(format!(
                "wall_bounce id={} axis=y dir={} vx={:.3} vy={:.3}",
                self.creature_id,
                self.current_dir.as_index(),
                self.vel_x,
                self.vel_y
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

/// Map velocity to a cardinal direction.
///
/// Terminal Y increases downward, so positive vel_y = moving down the screen.
pub fn velocity_to_dir(vel_x: f32, vel_y: f32) -> Direction {
    if vel_x.abs() < 0.01 && vel_y.abs() < 0.01 {
        return Direction::Down; // stationary → face down
    }
    if vel_x.abs() > vel_y.abs() {
        if vel_x > 0.0 {
            Direction::Right
        } else {
            Direction::Left
        }
    } else if vel_y > 0.0 {
        Direction::Down // moving toward bottom of screen
    } else {
        Direction::Up
    }
}

/// Direction mapping with hysteresis to avoid rapid up/down flips while
/// mostly moving left/right (and vice-versa).
pub fn stable_velocity_to_dir(vel_x: f32, vel_y: f32, current_dir: Direction) -> Direction {
    let ax = vel_x.abs();
    let ay = vel_y.abs();
    let threshold = 0.12;
    let new_dir = if ax < threshold && ay < threshold {
        current_dir // Keep current if barely moving
    } else if ax > ay {
        if vel_x > 0.0 {
            Direction::Right
        } else {
            Direction::Left
        }
    } else if vel_y > 0.0 {
        Direction::Down // terminal Y increases downward
    } else {
        Direction::Up
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
            slot.creature_id,
            slot.current_dir.as_index(),
            slot.vel_x,
            slot.vel_y
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
                    slots[i].current_dir.as_index(),
                    slots[j].current_dir.as_index(),
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
