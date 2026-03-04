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

        Self {
            config,
            slots,
            selected: 0,
            running: true,
            next_add_index: 0,
            notifications: VecDeque::new(),
            swap_transition: None,
            add_transition: None,
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

    /// Load sprites for all creatures currently in the roster.
    ///
    /// Errors and sprite warnings are posted as notifications rather than
    /// written to stderr (which would corrupt the TUI canvas).
    pub fn load_all_sprites(&mut self) {
        for i in 0..self.slots.len() {
            match crate::sprite_loading::load_slot_sprites(&mut self.slots[i], self.config.scale) {
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
    pub fn update_all_displays(&mut self) {
        for slot in &mut self.slots {
            slot.animator.tick();
        }
        // Advance XP for creatures that are actively eating or playing.
        self.tick_xp();
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

            // Only accrue XP while eating or playing.
            let is_xp_state = matches!(
                state,
                crate::animation::AnimationState::Eating
                    | crate::animation::AnimationState::Playing
            );

            if !is_xp_state {
                // Not in an XP-earning state — nothing to do this tick.
                // (anim_active_secs is reset in set_selected_state when the
                // player switches away, so we don't touch it here.)
                continue;
            }

            // Advance the continuous activity timer.
            slot.anim_active_secs += TICK_SECS;

            // Determine XP rate for the current activity duration.
            let xp_rate = if slot.anim_active_secs <= 10.0 {
                2.0_f32 // Fast gain for the first 10 seconds
            } else if slot.anim_active_secs <= 40.0 {
                1.0_f32 // Slower gain for 10–40 seconds
            } else {
                0.0_f32 // No gain after 40 seconds (diminishing returns)
            };

            // Accumulate fractional XP, then floor to u32.
            // Using a small float accumulator avoids rounding every tick.
            let fractional_gain = xp_rate * TICK_SECS;
            let new_xp_f = slot.xp as f32 + fractional_gain;
            slot.xp = new_xp_f.floor() as u32;

            // Check for level-up: threshold is 50 * current_level.
            let threshold = 50 * slot.level;
            if slot.xp >= threshold {
                // Level up! Reset XP and increment level.
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

    /// Switch the selected creature to a new animation state.
    ///
    /// Also resets `anim_active_secs` to 0 whenever the player moves to a
    /// non-XP-earning state (Idle or Sleeping). This ensures XP rate always
    /// starts fresh from the "0–10 second" fast tier when the creature goes
    /// back to eating or playing.
    pub fn set_selected_state(&mut self, state: crate::animation::AnimationState) {
        if let Some(slot) = self.slots.get_mut(self.selected) {
            slot.animator.set_state(state);

            // If the new state is NOT an XP-earning state, reset the activity
            // timer so the next Eat/Play session starts at the fast XP rate.
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
