use crate::config::{GameConfig, MAX_ACTIVE_CREATURES};
use crate::creature::CreatureSlot;
use crate::notification::{MAX_NOTIFICATIONS, NotifLevel, Notification};
use crate::sprite_loading::{
    AddTransition, EVOLVE_TICKS, EvolutionTransition, SpriteEncoder, SwapTransition,
    SwapWorkerResult,
};
use std::collections::VecDeque;
use std::path::PathBuf;
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
    /// Choosing which branch to evolve into (`App::evolve_choices`).
    EvolveChoice,
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
    pub evolution: Option<EvolutionTransition>,
    /// Species offered by the branch popup (`PromptMode::EvolveChoice`).
    pub evolve_choices: Vec<u32>,
    /// Background sprite loads kicked off at startup — one per initial roster slot.
    /// Each entry is (slot_id, expected_creature_id, receiver). The creature_id
    /// guard discards results if the slot was swapped before the load finished.
    pub startup_loads: Vec<(u64, u32, mpsc::Receiver<SwapWorkerResult>)>,
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
    /// Path to save the roster state to (if persistence is enabled).
    pub save_path: Option<PathBuf>,
    /// Timestamp of the last periodic save (for throttling).
    last_save: Instant,
    /// Encoder handed to loader threads so frames arrive ready to draw.
    /// `None` only in tests / before the terminal is set up.
    pub encoder: Option<SpriteEncoder>,
    /// Raw escape sequences to write to the terminal ahead of the next frame's
    /// diff (Kitty image uploads and deletes). Drained by the draw loop.
    pub term_writes: Vec<String>,
    /// Pending Kitty upload for the poke-doll toy (see `kitty_upload`).
    pub toy_upload: Option<crate::kitty_upload::KittyUpload>,
    /// Kitty image id of the currently encoded toy, for deletion on re-encode.
    pub toy_kitty_id: Option<u32>,
    /// Whether we're inside tmux (Kitty commands need passthrough wrapping).
    pub is_tmux: bool,
}

impl App {
    pub fn new(config: GameConfig, save_path: Option<PathBuf>) -> Self {
        let slots: Vec<CreatureSlot> = config
            .roster
            .iter()
            .map(|r| {
                let mut slot = CreatureSlot::new(r.creature_id, r.name.clone());
                slot.slot_id = r.slot_id;
                slot.level = r.level;
                slot.xp = r.xp;
                slot
            })
            .collect();

        const TOY_PNG: &[u8] = include_bytes!("../assets/poke_doll.png");
        let toy_image =
            image::load_from_memory(TOY_PNG).expect("bundled poke_doll.png is a valid PNG");

        Self {
            config,
            slots,
            selected: 0,
            running: true,
            notifications: VecDeque::new(),
            swap_transition: None,
            add_transition: None,
            evolution: None,
            evolve_choices: Vec::new(),
            startup_loads: Vec::new(),
            toy_image,
            toy_proto: None,
            toy_proto_rect: None,
            prompt_mode: PromptMode::None,
            prompt_buffer: String::new(),
            pen_dims: None,
            save_path,
            last_save: Instant::now(),
            encoder: None,
            term_writes: Vec::new(),
            toy_upload: None,
            toy_kitty_id: None,
            is_tmux: false,
        }
    }

    /// Persist the current roster state back to the config file.
    ///
    /// Silently does nothing if no save path is set. Save errors are
    /// non-fatal — the next save attempt will retry.
    fn save_roster(&mut self) {
        if let Err(e) = self.try_save_roster() {
            self.notify(NotifLevel::Error, format!("Couldn't save progress: {e}"));
        }
    }

    /// Write the roster to `save_path`. A no-op when persistence is off
    /// (`--creature`).
    fn try_save_roster(&self) -> Result<(), crate::config::ConfigError> {
        let Some(path) = &self.save_path else {
            return Ok(());
        };
        let slot_refs: Vec<&CreatureSlot> = self.slots.iter().collect();
        GameConfig::save(path, &slot_refs, &self.config.unlocked)
    }

    /// Final bookkeeping when the game exits: persist the roster (progress
    /// since the last periodic save would otherwise be lost) and queue
    /// deletion of every Kitty image this app uploaded. The caller writes
    /// `term_writes` out before restoring the terminal.
    ///
    /// Returns the save result rather than notifying, since the TUI is
    /// closing and the caller has to report a failure on stderr.
    pub fn on_quit(&mut self) -> Result<(), crate::config::ConfigError> {
        let ids = self.all_kitty_ids();
        self.retire_kitty_images(&ids);
        self.try_save_roster()
    }

    /// Queue deletion of terminal-side Kitty images that are no longer used.
    ///
    /// Kitty and Ghostty keep uploaded images until explicitly deleted; left
    /// alone they pile up across swaps/reloads until the terminal's storage
    /// quota evicts images that are still on screen.
    pub fn retire_kitty_images(&mut self, ids: &[u32]) {
        let is_tmux = self.is_tmux;
        self.term_writes.extend(
            ids.iter()
                .map(|&id| crate::kitty_upload::delete_command(id, is_tmux)),
        );
    }

    /// Every Kitty image id this app has encoded that is still in use.
    pub fn all_kitty_ids(&self) -> Vec<u32> {
        // Include the evolving creature's new form: its silhouette may already
        // have been uploaded during the flicker.
        let pending = self
            .evolution
            .as_ref()
            .and_then(|e| match &e.worker_result {
                Some(SwapWorkerResult::Loaded { slot, .. }) => Some(&slot.sprites.kitty_ids),
                _ => None,
            });
        self.slots
            .iter()
            .flat_map(|s| s.sprites.kitty_ids.iter().copied())
            .chain(pending.into_iter().flatten().copied())
            .chain(self.toy_kitty_id)
            .collect()
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
    /// Uses a two-phase strategy per creature:
    /// - **Phase 1**: download + process only Idle → slot appears on screen quickly.
    /// - **Phase 2**: download + process all remaining animations → full state support.
    ///
    /// Both results are sent on the same channel; `update_startup_loads` applies
    /// them in order, keeping the receiver alive until the thread exits.
    pub fn start_background_loads(&mut self) {
        let scale = self.config.scale;
        for slot in &self.slots {
            let id = slot.creature_id;
            let name = slot.creature_name.clone();
            let encoder = self.encoder.clone();
            let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
            std::thread::spawn(move || {
                // Phase 1: load Idle only so the creature appears quickly.
                let mut slot1 = CreatureSlot::new(id, name.clone());
                match crate::sprite_loading::load_slot_idle_only(&mut slot1, scale) {
                    Ok(warnings) => {
                        if let Some(enc) = &encoder {
                            enc.encode(&mut slot1);
                        }
                        let _ = tx.send(SwapWorkerResult::Loaded {
                            slot: Box::new(slot1),
                            warnings,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(SwapWorkerResult::Failed(e.to_string()));
                        return; // Can't continue without Idle
                    }
                }

                // Phase 2: full load (downloads remaining animations in background).
                let mut slot2 = CreatureSlot::new(id, name);
                match crate::sprite_loading::load_slot_sprites(&mut slot2, scale) {
                    Ok(warnings) => {
                        if let Some(enc) = &encoder {
                            enc.encode(&mut slot2);
                        }
                        let _ = tx.send(SwapWorkerResult::Loaded {
                            slot: Box::new(slot2),
                            warnings,
                        });
                    }
                    Err(_) => {
                        // Non-fatal: creature is already visible with Idle fallback.
                    }
                }
                // tx dropped here → Disconnected signals receiver to stop.
            });
            self.startup_loads.push((slot.slot_id, id, rx));
        }
    }

    /// Poll completed startup loads and apply them.
    ///
    /// Keeps each receiver alive until the sending thread exits (Disconnected),
    /// so that Phase 1 and Phase 2 results are both received and applied.
    fn update_startup_loads(&mut self) {
        let mut completions: Vec<(u64, u32, SwapWorkerResult)> = Vec::new();
        self.startup_loads
            .retain(|(slot_id, creature_id, rx)| match rx.try_recv() {
                Ok(result) => {
                    completions.push((*slot_id, *creature_id, result));
                    true // Keep until Disconnected — Phase 2 may send another message.
                }
                Err(mpsc::TryRecvError::Empty) => true,
                Err(mpsc::TryRecvError::Disconnected) => false,
            });
        for (slot_id, expected_id, result) in completions {
            // Look the slot up by id: removals shift indices while loads are in flight.
            let slot_index = self.slots.iter().position(|s| s.slot_id == slot_id);
            match result {
                SwapWorkerResult::Loaded { slot, warnings } => {
                    let Some(slot_index) = slot_index else {
                        continue; // Removed before the load finished.
                    };
                    let existing = &mut self.slots[slot_index];
                    // Discard if the slot was swapped to a different creature.
                    if existing.creature_id != expected_id {
                        continue;
                    }
                    // Swap in only the new pixels and timings. Everything else
                    // (position, current action, XP, level) stays as it is, so the
                    // creature neither teleports nor drops what it was doing.
                    let slot = *slot;
                    let old_ids = std::mem::replace(&mut existing.sprites, slot.sprites).kitty_ids;
                    existing.animator.adopt_timings(slot.animator);
                    self.retire_kitty_images(&old_ids);
                    for w in warnings {
                        self.notify(NotifLevel::Warn, w);
                    }
                }
                SwapWorkerResult::Failed(err) => {
                    let name = slot_index
                        .map(|i| self.slots[i].creature_name.clone())
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
        self.update_evolution();
        self.expire_notifications(Duration::from_secs(crate::notification::NOTIF_TTL_SECS));

        // Periodic save every 30 seconds.
        if self.last_save.elapsed() > Duration::from_secs(30) {
            self.save_roster();
            self.last_save = Instant::now();
        }
    }

    /// Advance physics for all slots using the pen dimensions from the last render.
    ///
    /// Skips slots not yet initialized (no prior render). Called once per game tick.
    fn update_physics(&mut self) {
        let Some((pen_w, pen_h, sprite_h)) = self.pen_dims else {
            return;
        };
        let transition_slot_index = self.transition_slot_index();
        let evolving_slot_index = self.evolving_slot_index();

        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.sprites.encoded_rect.is_none() {
                continue; // Not yet initialized — skip until first render.
            }
            let is_moving = matches!(
                slot.animator.state(),
                crate::animation::AnimationState::Idle
            ) && transition_slot_index != Some(i)
                && evolving_slot_index != Some(i);
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
        let mut level_up_events: Vec<(String, u32, bool)> = Vec::new();

        for slot in &mut self.slots {
            let was_ready = slot.ready_to_evolve();
            if let Some(new_level) = slot.tick_xp() {
                let became_ready = !was_ready && slot.ready_to_evolve();
                level_up_events.push((slot.creature_name.clone(), new_level, became_ready));
            }
        }

        let leveled_up = !level_up_events.is_empty();
        for (name, level, became_ready) in level_up_events {
            self.notify(
                NotifLevel::Info,
                format!("{name} leveled up! Now level {level} ✨"),
            );
            if became_ready {
                self.notify(
                    NotifLevel::Info,
                    format!("✨ {name} is ready to evolve! Press [V]"),
                );
            }
        }
        if leveled_up {
            self.save_roster();
        }
    }

    /// On startup, mention creatures that are already past an evolution level.
    pub fn announce_ready_to_evolve(&mut self) {
        let ready: Vec<String> = self
            .slots
            .iter()
            .filter(|s| s.ready_to_evolve())
            .map(|s| s.creature_name.clone())
            .collect();
        for name in ready {
            self.notify(
                NotifLevel::Info,
                format!("✨ {name} is ready to evolve! Press [V]"),
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
                slot.active_ticks = 0;
            }
        }
    }

    pub fn transition_slot_index(&self) -> Option<usize> {
        self.swap_transition.as_ref().map(|t| t.slot_index)
    }

    pub fn has_background_load(&self) -> bool {
        self.swap_transition.is_some() || self.add_transition.is_some() || self.evolution.is_some()
    }

    /// Index of the creature currently evolving, if any.
    pub fn evolving_slot_index(&self) -> Option<usize> {
        let id = self.evolution.as_ref()?.slot_id;
        self.slots.iter().position(|s| s.slot_id == id)
    }

    /// `V`: evolve the selected creature, asking first if it can branch.
    pub fn request_evolution(&mut self) {
        if self.has_background_load() {
            self.notify(
                NotifLevel::Warn,
                "Please wait for the current load to finish",
            );
            return;
        }
        let Some(slot) = self.slots.get(self.selected) else {
            return;
        };
        let name = slot.creature_name.clone();
        let options = crate::evolutions::level_evolutions(slot.creature_id, slot.level);
        match options.as_slice() {
            [] => {
                let msg = match crate::evolutions::next_evolution_level(slot.creature_id) {
                    Some(at) => format!("{name} evolves at Lv.{at} (now Lv.{})", slot.level),
                    None => format!("{name} can't evolve by leveling up yet"),
                };
                self.notify(NotifLevel::Info, msg);
            }
            [only] => self.start_evolution(*only),
            _ => {
                self.evolve_choices = options;
                self.prompt_mode = PromptMode::EvolveChoice;
            }
        }
    }

    /// Begin evolving the selected creature into `target_id`: load the new
    /// form in the background while the flicker plays.
    pub fn start_evolution(&mut self, target_id: u32) {
        let Some(slot) = self.slots.get(self.selected) else {
            return;
        };
        let Some(target_name) = crate::creatures::lookup_name(target_id) else {
            return;
        };
        let from_name = slot.creature_name.clone();
        let slot_id = slot.slot_id;
        let worker_name = target_name.to_string();
        let scale = self.config.scale;
        let encoder = self.encoder.clone();
        let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
        std::thread::spawn(move || {
            let mut new_slot = CreatureSlot::new(target_id, worker_name);
            let msg = match crate::sprite_loading::load_slot_sprites(&mut new_slot, scale) {
                Ok(warnings) => {
                    if let Some(enc) = &encoder {
                        enc.encode(&mut new_slot);
                    }
                    SwapWorkerResult::Loaded {
                        slot: Box::new(new_slot),
                        warnings,
                    }
                }
                Err(e) => SwapWorkerResult::Failed(e.to_string()),
            };
            let _ = tx.send(msg);
        });
        self.notify(NotifLevel::Info, format!("What? {from_name} is evolving!"));
        self.evolution = Some(EvolutionTransition {
            slot_id,
            from_name,
            target_id,
            target_name: target_name.to_string(),
            elapsed_ticks: 0,
            worker_rx: rx,
            worker_result: None,
        });
    }

    /// Advance an in-progress evolution; apply it once the flicker has run
    /// its course and the new form has loaded.
    pub fn update_evolution(&mut self) {
        let Some(evo) = self.evolution.as_mut() else {
            return;
        };
        if evo.worker_result.is_none()
            && let Ok(result) = evo.worker_rx.try_recv()
        {
            evo.worker_result = Some(result);
        }
        evo.elapsed_ticks = evo.elapsed_ticks.saturating_add(1);

        if let Some(SwapWorkerResult::Failed(err)) = &evo.worker_result {
            let msg = format!("{} couldn't evolve: {err}", evo.from_name);
            self.evolution = None;
            self.notify(NotifLevel::Error, msg);
            return;
        }
        let done = evo.elapsed_ticks >= EVOLVE_TICKS && evo.worker_result.is_some();
        if !done {
            return;
        }
        let evo = self.evolution.take().expect("checked above");
        let Some(SwapWorkerResult::Loaded {
            slot: new,
            warnings,
        }) = evo.worker_result
        else {
            unreachable!("Failed handled above");
        };
        let Some(index) = self.slots.iter().position(|s| s.slot_id == evo.slot_id) else {
            self.retire_kitty_images(&new.sprites.kitty_ids);
            return;
        };

        // Same creature, new species: slot_id, level, XP, position and the
        // current action carry over.
        let new = *new;
        let slot = &mut self.slots[index];
        slot.creature_id = evo.target_id;
        slot.creature_name = evo.target_name.clone();
        let old_ids = std::mem::replace(&mut slot.sprites, new.sprites).kitty_ids;
        slot.animator.adopt_timings(new.animator);
        self.retire_kitty_images(&old_ids);

        if !self.config.unlocked.contains(&evo.target_id) {
            self.config.unlocked.push(evo.target_id);
            self.config.unlocked.sort_unstable();
        }
        self.save_roster();
        self.notify(
            NotifLevel::Info,
            format!(
                "Congratulations! Your {} evolved into {}!",
                evo.from_name, evo.target_name
            ),
        );
        for w in warnings {
            self.notify(NotifLevel::Warn, w);
        }
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
        let removed = self.slots.remove(self.selected);
        self.retire_kitty_images(&removed.sprites.kitty_ids);
        // Keep `selected` in bounds.
        if self.selected >= self.slots.len() {
            self.selected = self.slots.len() - 1;
        }
        self.save_roster();
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

        if let Some((slot_index, mut slot)) = apply_swap {
            if slot_index < self.slots.len() {
                // The newcomer appears where the recalled creature stood.
                slot.take_motion_from(&self.slots[slot_index]);
                let old_ids = std::mem::take(&mut self.slots[slot_index].sprites.kitty_ids);
                self.slots[slot_index] = slot;
                self.retire_kitty_images(&old_ids);
            }
            self.swap_transition = None;
            self.save_roster();
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
                self.save_roster();
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

    /// The name of species `id` if the player may add or swap it in;
    /// otherwise notifies why not (unknown, or locked until evolved into).
    fn selectable_name(&mut self, id: u32) -> Option<&'static str> {
        let Some(name) = crate::creatures::lookup_name(id) else {
            self.notify(NotifLevel::Warn, format!("#{id} is not a known Pokémon"));
            return None;
        };
        if !crate::evolutions::is_selectable(id, &self.config.unlocked) {
            self.notify(
                NotifLevel::Warn,
                format!("#{id} {name} is locked. Evolve one to unlock it."),
            );
            return None;
        }
        Some(name)
    }

    /// Add a creature by National Pokédex number.
    /// Looks up the name from FULL_DEX, then starts a background sprite load.
    pub fn add_creature_by_dex(&mut self, id: u32) {
        if self.slots.len() >= MAX_ACTIVE_CREATURES {
            self.notify(NotifLevel::Warn, "Pen is full — remove a creature first");
            return;
        }
        let Some(name) = self.selectable_name(id) else {
            return;
        };
        let target_name = name.to_string();
        let worker_name = target_name.clone();
        let scale = self.config.scale;
        let encoder = self.encoder.clone();
        let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
        std::thread::spawn(move || {
            let mut slot = CreatureSlot::new(id, worker_name);
            let msg = match crate::sprite_loading::load_slot_sprites(&mut slot, scale) {
                Ok(warnings) => {
                    if let Some(enc) = &encoder {
                        enc.encode(&mut slot);
                    }
                    SwapWorkerResult::Loaded {
                        slot: Box::new(slot),
                        warnings,
                    }
                }
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
        let Some(name) = self.selectable_name(id) else {
            return;
        };
        let selected_index = self.selected;
        let target_name = name.to_string();
        let worker_name = target_name.clone();
        let scale = self.config.scale;
        let encoder = self.encoder.clone();
        let (tx, rx) = mpsc::channel::<SwapWorkerResult>();
        std::thread::spawn(move || {
            let mut new_slot = CreatureSlot::new(id, worker_name);
            let msg = match crate::sprite_loading::load_slot_sprites(&mut new_slot, scale) {
                Ok(warnings) => {
                    if let Some(enc) = &encoder {
                        enc.encode(&mut new_slot);
                    }
                    SwapWorkerResult::Loaded {
                        slot: Box::new(new_slot),
                        warnings,
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory unique to this test.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "poclimon_app_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app_saving_to(path: PathBuf) -> App {
        let config = GameConfig::from_creature_name("pikachu").unwrap();
        App::new(config, Some(path))
    }

    #[test]
    fn test_on_quit_saves_progress_made_since_the_last_save() {
        let path = test_dir("quit").join("poclimon.toml");
        let mut app = app_saving_to(path.clone());
        app.slots[0].level = 7;
        app.slots[0].xp = 33;

        app.on_quit().unwrap();

        let crate::config::save::LoadOutcome::Loaded(saved) = crate::config::save::load(&path)
        else {
            panic!("quit save didn't verify");
        };
        assert_eq!(saved.slot.len(), 1);
        assert_eq!(saved.slot[0].level, 7);
        assert_eq!(saved.slot[0].xp, 33);
    }

    #[test]
    fn test_failed_save_posts_an_error_notification() {
        // A path whose parent is a regular file can never be written.
        let dir = test_dir("savefail");
        let blocker = dir.join("not-a-dir");
        std::fs::write(&blocker, "").unwrap();
        let mut app = app_saving_to(blocker.join("poclimon.toml"));

        app.save_roster();

        assert!(
            app.notifications
                .iter()
                .any(|n| matches!(n.level, NotifLevel::Error)),
            "save failure was swallowed silently"
        );
    }

    use crate::animation::{AnimationState, Animator};
    use std::sync::Arc;

    /// A loader result for `like`'s creature carrying a 1-frame Idle sprite,
    /// so tests can tell whether it was applied.
    fn loaded_result(like: &CreatureSlot) -> SwapWorkerResult {
        let mut slot = CreatureSlot::new(like.creature_id, like.creature_name.clone());
        slot.sprites.idle[0] = vec![Arc::new(image::DynamicImage::new_rgba8(1, 1))];
        slot.animator = Animator::new();
        SwapWorkerResult::Loaded {
            slot: Box::new(slot),
            warnings: Vec::new(),
        }
    }

    /// Register a fake in-flight startup load for slot `index` and return its sender.
    fn fake_load(app: &mut App, index: usize) -> mpsc::Sender<SwapWorkerResult> {
        let (tx, rx) = mpsc::channel();
        let slot = &app.slots[index];
        app.startup_loads.push((slot.slot_id, slot.creature_id, rx));
        tx
    }

    #[test]
    fn startup_load_lands_on_its_slot_after_an_earlier_removal() {
        let mut app = App::new(GameConfig::default(), None);
        let tx = fake_load(&mut app, 2);
        let target_id = app.slots[2].slot_id;
        let result = loaded_result(&app.slots[2]);

        app.select_slot(0);
        app.remove_selected();
        tx.send(result).unwrap();
        app.update_startup_loads();

        let target = app.slots.iter().find(|s| s.slot_id == target_id).unwrap();
        assert_eq!(target.sprites.idle[0].len(), 1, "load was dropped");
        // The slot that moved into index 2 must not receive it either.
        assert!(app.slots[2].sprites.idle[0].is_empty());
    }

    #[test]
    fn startup_load_keeps_current_action_and_progress() {
        let mut app = App::new(GameConfig::default(), None);
        let tx = fake_load(&mut app, 0);
        app.select_slot(0);
        app.set_selected_state(AnimationState::Eating);
        app.slots[0].xp = 7;
        app.slots[0].pos_x = 12.0;

        tx.send(loaded_result(&app.slots[0])).unwrap();
        app.update_startup_loads();

        let slot = &app.slots[0];
        assert_eq!(slot.animator.state(), AnimationState::Eating);
        assert_eq!(slot.xp, 7);
        assert_eq!(slot.pos_x, 12.0);
        assert_eq!(slot.sprites.idle[0].len(), 1);
    }

    #[test]
    fn swap_keeps_the_position_of_the_recalled_creature() {
        let mut app = App::new(GameConfig::default(), None);
        app.slots[1].pos_x = 30.0;
        app.slots[1].pos_y = 4.0;
        app.slots[1].position_initialized = true;
        let (tx, rx) = mpsc::channel();
        app.swap_transition = Some(SwapTransition {
            slot_index: 1,
            recall_ticks: 0,
            target_name: "Mew".to_string(),
            worker_rx: rx,
            worker_result: None,
        });

        let newcomer = CreatureSlot::new(151, "Mew".to_string());
        tx.send(loaded_result(&newcomer)).unwrap();
        app.update_swap_transition();

        let slot = &app.slots[1];
        assert_eq!(slot.creature_id, 151);
        assert_eq!((slot.pos_x, slot.pos_y), (30.0, 4.0));
        assert!(slot.position_initialized);
    }

    #[test]
    fn locked_species_cannot_be_added_or_swapped_in() {
        let mut app = App::new(GameConfig::default(), None);
        app.add_creature_by_dex(26); // Raichu
        app.swap_selected_to_dex(2); // Ivysaur
        assert!(app.add_transition.is_none() && app.swap_transition.is_none());
        let locked: Vec<_> = app
            .notifications
            .iter()
            .filter(|n| n.message.contains("is locked"))
            .collect();
        assert_eq!(locked.len(), 2);
        assert!(locked[0].message.starts_with("#26 Raichu"));
    }

    #[test]
    fn unlocked_species_can_be_swapped_in() {
        let config = GameConfig {
            unlocked: vec![2],
            ..GameConfig::default()
        };
        let mut app = App::new(config, None);
        app.swap_selected_to_dex(2);
        assert!(app.swap_transition.is_some());
    }

    #[test]
    fn reaching_an_evolution_level_is_announced_once() {
        let mut app = App::new(GameConfig::default(), None);
        app.selected = 0; // Bulbasaur evolves at 16
        app.slots[0].level = 15;
        app.slots[0].xp = crate::creature::xp_to_next_level(15) - 1;
        app.slots[0].xp_milli = 999;

        app.tick_xp();
        app.tick_xp();

        assert_eq!(app.slots[0].level, 16);
        let ready = app
            .notifications
            .iter()
            .filter(|n| n.message == "✨ Bulbasaur is ready to evolve! Press [V]")
            .count();
        assert_eq!(ready, 1);
    }

    /// An app whose first creature (Bulbasaur, Lv 16) is mid-evolution into
    /// Ivysaur, with the worker's channel handed back to the test.
    fn evolving_app() -> (App, mpsc::Sender<SwapWorkerResult>) {
        let mut app = App::new(GameConfig::default(), None);
        let slot = &mut app.slots[0];
        slot.level = 16;
        slot.xp = 123;
        slot.pos_x = 9.0;
        let (tx, rx) = mpsc::channel();
        app.evolution = Some(EvolutionTransition {
            slot_id: slot.slot_id,
            from_name: "Bulbasaur".to_string(),
            target_id: 2,
            target_name: "Ivysaur".to_string(),
            elapsed_ticks: 0,
            worker_rx: rx,
            worker_result: None,
        });
        (app, tx)
    }

    #[test]
    fn evolution_applies_after_the_flicker_and_keeps_progress() {
        let (mut app, tx) = evolving_app();
        let slot_id = app.slots[0].slot_id;
        tx.send(loaded_result(&CreatureSlot::new(2, "Ivysaur".into())))
            .unwrap();

        for _ in 0..EVOLVE_TICKS - 1 {
            app.update_evolution();
        }
        assert_eq!(
            app.slots[0].creature_id, 1,
            "applied before the flicker ended"
        );
        app.update_evolution();

        let slot = &app.slots[0];
        assert_eq!(
            (slot.creature_id, slot.creature_name.as_str()),
            (2, "Ivysaur")
        );
        assert_eq!((slot.slot_id, slot.level, slot.xp), (slot_id, 16, 123));
        assert_eq!(slot.pos_x, 9.0);
        assert_eq!(slot.sprites.idle[0].len(), 1, "new sprites not swapped in");
        assert!(app.evolution.is_none());
        assert!(app.config.unlocked.contains(&2));
        assert!(!app.slots[0].ready_to_evolve()); // Ivysaur's next is Lv 32
        assert!(
            app.notifications
                .iter()
                .any(|n| n.message == "Congratulations! Your Bulbasaur evolved into Ivysaur!")
        );
    }

    #[test]
    fn evolution_is_saved_with_the_unlock() {
        let (mut app, tx) = evolving_app();
        let path = test_dir("evolve-save").join("save.toml");
        app.save_path = Some(path.clone());
        let slot_id = app.slots[0].slot_id;
        tx.send(loaded_result(&CreatureSlot::new(2, "Ivysaur".into())))
            .unwrap();
        for _ in 0..EVOLVE_TICKS {
            app.update_evolution();
        }

        let crate::config::save::LoadOutcome::Loaded(saved) = crate::config::save::load(&path)
        else {
            panic!("evolution wasn't saved");
        };
        let evolved = saved.slot.iter().find(|s| s.slot_id == slot_id).unwrap();
        assert_eq!((evolved.id, evolved.level, evolved.xp), (2, 16, 123));
        assert_eq!(saved.progress.unlocked, vec![2]);
    }

    #[test]
    fn evolution_waits_for_a_slow_load() {
        let (mut app, tx) = evolving_app();
        for _ in 0..EVOLVE_TICKS * 2 {
            app.update_evolution();
        }
        assert!(app.evolution.is_some());
        tx.send(loaded_result(&CreatureSlot::new(2, "Ivysaur".into())))
            .unwrap();
        app.update_evolution();
        assert_eq!(app.slots[0].creature_id, 2);
    }

    #[test]
    fn failed_evolution_keeps_the_old_species() {
        let (mut app, tx) = evolving_app();
        tx.send(SwapWorkerResult::Failed("offline".into())).unwrap();
        app.update_evolution();

        assert!(app.evolution.is_none());
        assert_eq!(app.slots[0].creature_id, 1);
        assert!(app.slots[0].ready_to_evolve(), "can retry");
        assert!(app.config.unlocked.is_empty());
        assert!(
            app.notifications
                .iter()
                .any(|n| matches!(n.level, NotifLevel::Error) && n.message.contains("offline"))
        );
    }

    #[test]
    fn flicker_shows_the_new_form_only_once_loaded() {
        let (mut app, tx) = evolving_app();
        let shown = |app: &mut App| -> Vec<bool> {
            (0..EVOLVE_TICKS)
                .map(|t| {
                    let evo = app.evolution.as_mut().unwrap();
                    evo.elapsed_ticks = t;
                    evo.shows_new_form()
                })
                .collect()
        };
        assert!(shown(&mut app).iter().all(|&new| !new));

        tx.send(loaded_result(&CreatureSlot::new(2, "Ivysaur".into())))
            .unwrap();
        let evo = app.evolution.as_mut().unwrap();
        evo.worker_result = evo.worker_rx.try_recv().ok();
        let frames = shown(&mut app);
        let toggles = frames.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(!frames[0], "starts on the old form");
        assert!(toggles > 20, "only {toggles} toggles");
        // Faster and faster: the first switch takes longer than the last.
        let first = frames.iter().position(|&n| n).unwrap();
        assert!(first >= 8, "first switch at tick {first}");
    }

    #[test]
    fn branching_evolution_asks_which_form() {
        let mut app = App::new(GameConfig::from_creature_name("tyrogue").unwrap(), None);
        app.slots[0].level = 20;
        app.request_evolution();
        assert_eq!(app.prompt_mode, PromptMode::EvolveChoice);
        assert_eq!(app.evolve_choices, vec![106, 107, 237]);
        assert!(app.evolution.is_none());
    }

    #[test]
    fn evolving_too_early_says_when() {
        let mut app = App::new(GameConfig::default(), None);
        app.slots[0].level = 3;
        app.request_evolution();
        assert!(app.evolution.is_none());
        assert_eq!(
            app.notifications.back().unwrap().message,
            "Bulbasaur evolves at Lv.16 (now Lv.3)"
        );

        app.select_slot(4); // Eevee: item evolutions only, deferred
        app.request_evolution();
        assert_eq!(
            app.notifications.back().unwrap().message,
            "Eevee can't evolve by leveling up yet"
        );
    }

    #[test]
    fn evolution_is_blocked_during_a_swap() {
        let mut app = App::new(GameConfig::default(), None);
        app.slots[0].level = 16;
        let (_tx, rx) = mpsc::channel();
        app.swap_transition = Some(SwapTransition {
            slot_index: 1,
            recall_ticks: RECALL_TICKS,
            target_name: "Mew".to_string(),
            worker_rx: rx,
            worker_result: None,
        });
        app.request_evolution();
        assert!(app.evolution.is_none());
        assert!(app.notifications.back().unwrap().message.contains("wait"));
    }
}
