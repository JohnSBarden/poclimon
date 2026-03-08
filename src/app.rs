use crate::config::{GameConfig, MAX_ACTIVE_CREATURES};
use crate::creature::CreatureSlot;
use crate::notification::{MAX_NOTIFICATIONS, NotifLevel, Notification};
use crate::sprite_loading::{AddTransition, SwapTransition, SwapWorkerResult};
use std::collections::VecDeque;
use std::sync::mpsc::{self};
use std::time::{Duration, Instant};

pub const RECALL_TICKS: u8 = 18;

/// Which prompt dialog is currently showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    /// No prompt — normal gameplay.
    None,
    /// "Add a Pokémon" prompt — typing Pokédex number.
    Add,
    /// "Swap selected Pokémon" prompt — typing Pokédex number.
    Swap,
}

pub struct App {
    pub config: GameConfig,
    pub slots: Vec<CreatureSlot>,
    pub selected: usize,
    pub running: bool,
    /// In-TUI notification messages (replaces eprintln! during TUI operation).
    pub notifications: VecDeque<Notification>,
    pub swap_transition: Option<SwapTransition>,
    pub add_transition: Option<AddTransition>,
    /// Background sprite loads kicked off at startup — one per initial roster slot.
    /// Each entry is (slot_index, expected_creature_id, receiver). The creature_id
    /// guard discards results if the slot was swapped before the load finished.
    pub startup_loads: Vec<(usize, u32, mpsc::Receiver<SwapWorkerResult>)>,
    /// The poke-doll sprite image, decoded once from the bundled PNG at startup.
    pub toy_image: image::DynamicImage,
    /// Encoded `Protocol` for the poke-doll, cached and re-encoded only when the
    /// terminal size or protocol type changes (same lazy pattern as creature sprites).
    pub toy_proto: Option<ratatui_image::protocol::Protocol>,
    /// The size `Rect` (position 0,0) the toy protocol was encoded for.
    pub toy_proto_rect: Option<ratatui::layout::Rect>,
    /// What kind of text prompt is currently active (if any).
    pub prompt_mode: PromptMode,
    /// Characters typed so far in the active prompt.
    pub prompt_buffer: String,
    /// Pen dimensions (width, height, sprite_h) from the most recent render.
    /// Set by `render_pen` each frame; used by `update_physics` on the next tick.
    /// `None` until the first render completes.
    pub pen_dims: Option<(u16, u16, u16)>,
}

impl App {
    pub fn new(config: GameConfig) -> Self {
        let slots: Vec<CreatureSlot> = config
            .roster
            .iter()
            .map(|(id, name)| CreatureSlot::new(*id, name.clone()))
            .collect();

        const TOY_PNG: &[u8] = include_bytes!("../assets/poke_doll.png");
        let toy_image = image::load_from_memory(TOY_PNG)
            .expect("bundled poke_doll.png is a valid PNG");

        Self {
            config,
            slots,
            selected: 0,
            running: true,
            notifications: VecDeque::new(),
            swap_transition: None,
            add_transition: None,
            startup_loads: Vec::new(),
            toy_image,
            toy_proto: None,
            toy_proto_rect: None,
            prompt_mode: PromptMode::None,
            prompt_buffer: String::new(),
            pen_dims: None,
        }
    }

    /// Post a notification to the in-TUI message log.
    ///
    /// Displayed in the status+notifications panel. If the deque is at
    /// capacity, the oldest entry is dropped to make room.
    pub fn notify(&mut self, level: NotifLevel, message: impl Into<String>) {
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
    pub fn expire_notifications(&mut self, ttl: Duration) {
        self.notifications.retain(|n| n.created_at.elapsed() < ttl);
    }

    /// Kick off background sprite loads for every slot in the initial roster.
    pub fn start_background_loads(&mut self) {
        let scale = self.config.scale;
        for (idx, slot) in self.slots.iter().enumerate() {
            let id = slot.creature_id;
            let name = slot.creature_name.clone();
            let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
            std::thread::spawn(move || {
                let mut new_slot = CreatureSlot::new(id, name);
                let msg = match crate::sprite_loading::load_slot_sprites(&mut new_slot, scale) {
                    Ok(warnings) => SwapWorkerResult::Loaded {
                        slot: Box::new(new_slot),
                        warnings,
                    },
                    Err(e) => SwapWorkerResult::Failed(e.to_string()),
                };
                let _ = tx.send(msg);
            });
            self.startup_loads.push((idx, id, rx));
        }
    }

    /// Poll completed startup loads and apply them.
    fn update_startup_loads(&mut self) {
        let mut completions: Vec<(usize, u32, SwapWorkerResult)> = Vec::new();
        self.startup_loads.retain(|(slot_index, creature_id, rx)| {
            match rx.try_recv() {
                Ok(result) => {
                    completions.push((*slot_index, *creature_id, result));
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
                Err(mpsc::TryRecvError::Disconnected) => false,
            }
        });
        for (slot_index, expected_id, result) in completions {
            match result {
                SwapWorkerResult::Loaded { mut slot, warnings } => {
                    if slot_index >= self.slots.len() {
                        continue;
                    }
                    let existing = &self.slots[slot_index];
                    // Discard if the slot was swapped to a different creature.
                    if existing.creature_id != expected_id {
                        continue;
                    }
                    // Preserve physics so the creature doesn't teleport.
                    slot.pos_x = existing.pos_x;
                    slot.pos_y = existing.pos_y;
                    slot.vel_x = existing.vel_x;
                    slot.vel_y = existing.vel_y;
                    slot.current_dir = existing.current_dir;
                    slot.dir_hold_ticks = existing.dir_hold_ticks;
                    slot.pause_ticks = existing.pause_ticks;
                    slot.pause_face_down = existing.pause_face_down;
                    slot.dir_cooldown_ticks = existing.dir_cooldown_ticks;
                    self.slots[slot_index] = *slot;
                    for w in warnings {
                        self.notify(NotifLevel::Warn, w);
                    }
                }
                SwapWorkerResult::Failed(err) => {
                    let name = self
                        .slots
                        .get(slot_index)
                        .map(|s| s.creature_name.clone())
                        .unwrap_or_default();
                    self.notify(NotifLevel::Error, format!("Failed to load {name}: {err}"));
                }
            }
        }
    }

    pub fn update_all_displays(&mut self) {
        self.update_physics();
        self.tick_xp();
        self.update_startup_loads();
        self.update_swap_transition();
        self.update_add_transition();
        self.expire_notifications(Duration::from_secs(crate::notification::NOTIF_TTL_SECS));
    }

    /// Advance physics for all slots using the pen dimensions from the last render.
    ///
    /// Skips slots not yet initialized (no prior render). Called once per game tick.
    fn update_physics(&mut self) {
        let Some((pen_w, pen_h, sprite_h)) = self.pen_dims else {
            return;
        };
        let transition_slot_index = self.transition_slot_index();

        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.sprites.encoded_rect.is_none() {
                continue; // Not yet initialized — skip until first render.
            }
            let is_moving =
                matches!(slot.animator.state(), crate::animation::AnimationState::Idle)
                    && transition_slot_index != Some(i);
            slot.update_position(pen_w, pen_h, crate::creature::SPRITE_W, sprite_h, is_moving);
        }

        crate::creature::resolve_collisions(
            &mut self.slots,
            crate::creature::SPRITE_W,
            crate::creature::sprite_stack_h(sprite_h),
            pen_w,
            pen_h,
        );

        for slot in &mut self.slots {
            crate::creature::maybe_update_facing_from_velocity(slot);
        }
    }

    /// Accrue XP for all slots and emit level-up notifications.
    ///
    /// Delegates per-slot math to `CreatureSlot::tick_xp`. We can't borrow
    /// `self.slots` and `self.notifications` mutably at the same time, so
    /// level-up events are collected first and emitted after the slot loop.
    pub fn tick_xp(&mut self) {
        let mut level_up_events: Vec<(String, u32)> = Vec::new();

        for slot in &mut self.slots {
            if let Some(new_level) = slot.tick_xp() {
                level_up_events.push((slot.creature_name.clone(), new_level));
            }
        }

        for (name, level) in level_up_events {
            self.notify(
                NotifLevel::Info,
                format!("{name} leveled up! Now level {level} ✨"),
            );
        }
    }

    pub fn select_next(&mut self) {
        if !self.slots.is_empty() {
            self.selected = (self.selected + 1) % self.slots.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.slots.is_empty() {
            self.selected = if self.selected == 0 {
                self.slots.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    pub fn select_slot(&mut self, index: usize) {
        if index < self.slots.len() {
            self.selected = index;
        }
    }

    pub fn set_selected_state(&mut self, state: crate::animation::AnimationState) {
        if let Some(slot) = self.slots.get_mut(self.selected) {
            slot.animator.set_state(state);
            // Reset activity timer when leaving an XP state so the next session
            // starts at the fast tier again.
            let is_xp_state = matches!(
                state,
                crate::animation::AnimationState::Eating
                    | crate::animation::AnimationState::Playing
            );
            if !is_xp_state {
                slot.anim_active_secs = 0.0;
            }
        }
    }

    pub fn transition_slot_index(&self) -> Option<usize> {
        self.swap_transition.as_ref().map(|t| t.slot_index)
    }

    pub fn has_background_load(&self) -> bool {
        self.swap_transition.is_some() || self.add_transition.is_some()
    }

    /// Remove the currently selected slot from the roster.
    ///
    /// Silently does nothing if the roster would drop below 1 creature.
    pub fn remove_selected(&mut self) {
        if self.has_background_load() {
            self.notify(
                NotifLevel::Warn,
                "Please wait for the current load to finish",
            );
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
    pub fn update_swap_transition(&mut self) {
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

    pub fn update_add_transition(&mut self) {
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

    /// Add a creature by National Pokédex number.
    /// Looks up the name from FULL_DEX, then starts a background sprite load.
    pub fn add_creature_by_dex(&mut self, id: u32) {
        if self.slots.len() >= MAX_ACTIVE_CREATURES {
            self.notify(NotifLevel::Warn, "Pen is full — remove a creature first");
            return;
        }
        let Some(name) = crate::creatures::lookup_name(id) else {
            self.notify(NotifLevel::Warn, format!("#{id} is not a known Pokémon"));
            return;
        };
        let target_name = name.to_string();
        let worker_name = target_name.clone();
        let scale = self.config.scale;
        let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
        std::thread::spawn(move || {
            let mut slot = CreatureSlot::new(id, worker_name);
            let msg = match crate::sprite_loading::load_slot_sprites(&mut slot, scale) {
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

    /// Swap the selected creature to a new Pokémon by National Pokédex number.
    pub fn swap_selected_to_dex(&mut self, id: u32) {
        let Some(name) = crate::creatures::lookup_name(id) else {
            self.notify(NotifLevel::Warn, format!("#{id} is not a known Pokémon"));
            return;
        };
        let selected_index = self.selected;
        let target_name = name.to_string();
        let worker_name = target_name.clone();
        let scale = self.config.scale;
        let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
        std::thread::spawn(move || {
            let mut new_slot = CreatureSlot::new(id, worker_name);
            let msg = match crate::sprite_loading::load_slot_sprites(&mut new_slot, scale) {
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
