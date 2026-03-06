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
    /// Index into `creatures::ROSTER` used by the `A` key (old cycle behavior).
    #[allow(dead_code)]
    pub next_add_index: usize,
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
            next_add_index: 0,
            notifications: VecDeque::new(),
            swap_transition: None,
            add_transition: None,
            startup_loads: Vec::new(),
            toy_image,
            toy_proto: None,
            toy_proto_rect: None,
            prompt_mode: PromptMode::None,
            prompt_buffer: String::new(),
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
    ///
    /// Returns immediately so the game loop can render its first frame at once.
    /// Slots show "Loading…" until their worker finishes and `update_startup_loads`
    /// swaps in the populated slot.
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
    ///
    /// Physics state (position, velocity, direction) is copied from the current
    /// slot so the creature doesn't teleport when sprites arrive. If the slot's
    /// creature was swapped before the load finished, the stale result is dropped.
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
        for slot in &mut self.slots {
            slot.animator.tick();
        }
        self.tick_xp();
        self.update_startup_loads();
        self.update_swap_transition();
        self.update_add_transition();
        self.expire_notifications(Duration::from_secs(crate::notification::NOTIF_TTL_SECS));
    }

    /// Accrue XP for all slots currently in an XP-earning state (Eating or Playing).
    ///
    /// Called every game tick (50ms = 0.05 seconds). XP rate decays over time
    /// to reward short bursts of activity rather than leaving the game AFK:
    ///   - 0–10 s:  2 xp/sec
    ///   - 10–40 s: 1 xp/sec
    ///   - 40+ s:   0 xp/sec
    ///
    /// When a creature collects enough XP it levels up, its XP resets to 0,
    /// and a notification appears in the status panel.
    pub fn tick_xp(&mut self) {
        // Each game tick is 50ms = 0.05 seconds.
        const TICK_SECS: f32 = 0.05;

        // We can't borrow self.slots and self.notifications mutably at the
        // same time, so collect level-up events and emit them after the loop.
        let mut level_up_events: Vec<(String, u32)> = Vec::new();

        for slot in &mut self.slots {
            let state = slot.animator.state();

            let is_xp_state = matches!(
                state,
                crate::animation::AnimationState::Eating
                    | crate::animation::AnimationState::Playing
            );
            if !is_xp_state {
                continue;
            }

            slot.anim_active_secs += TICK_SECS;

            // Rate decays over time: 2xp/s for the first 10s, 1xp/s to 40s, then 0.
            let xp_rate = if slot.anim_active_secs <= 10.0 {
                2.0_f32
            } else if slot.anim_active_secs <= 40.0 {
                1.0_f32
            } else {
                0.0_f32
            };

            // Bank fractional XP; flush whole points to avoid flooring every tick to 0.
            slot.xp_frac += xp_rate * TICK_SECS;
            let whole = slot.xp_frac.floor() as u32;
            if whole > 0 {
                slot.xp = slot.xp.saturating_add(whole);
                slot.xp_frac -= whole as f32;
            }

            let threshold = 50 * slot.level;
            if slot.xp >= threshold {
                slot.xp = 0;
                slot.level += 1;
                level_up_events.push((slot.creature_name.clone(), slot.level));
            }
        }

        // Emit level-up notifications now that we're no longer borrowing slots.
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

    /// Add the next available creature (not already in roster) to the end.
    ///
    /// Cycles through `creatures::ROSTER` in order, skipping IDs already
    /// present.  Does nothing when all creatures are already in the roster
    /// or the roster is already at the display limit (6 slots).
    #[allow(dead_code)]
    pub fn add_creature(&mut self) {
        if self.has_background_load() {
            self.notify(
                NotifLevel::Warn,
                "Please wait for the current load to finish",
            );
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
        let roster = crate::creatures::ROSTER;
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

    /// Cycle the creature in the selected slot through all `creatures::ROSTER`
    /// entries, wrapping around. Recall animation plays while sprites load in
    /// the background and then the slot swaps without freezing the app.
    #[allow(dead_code)]
    pub fn cycle_selected_creature(&mut self) {
        if self.has_background_load() {
            self.notify(NotifLevel::Warn, "A creature load is already in progress");
            return;
        }

        let Some(slot) = self.slots.get(self.selected) else {
            return;
        };

        let current_id = slot.creature_id;
        let roster = crate::creatures::ROSTER;

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
