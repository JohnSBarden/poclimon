//! Timing-only animation: frame durations without pixel data.

use std::time::Instant;

/// The creature's current animation state.
///
/// Each variant maps to a specific sprite sheet and a slot in
/// `CreatureSlot::sprites.encoded` (0=Idle, 1=Eat, 2=Sleep, 3=Recall, 4=Playing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationState {
    /// Standing around, wandering the pen.
    Idle,
    /// Eating — creature plays the "Eat" animation.
    Eating,
    /// Sleeping.
    Sleeping,
    /// Playing / hopping around. Uses the "Hop" animation from PMDCollab.
    /// Earns XP just like Eating.
    Playing,
}

impl AnimationState {
    /// Return the index into `SpriteCache::encoded` for this state.
    ///
    /// Recall (index 3) is a transition visual, not a game state, so it is
    /// not represented here and callers must use the literal `3` for it.
    pub fn encoded_index(self) -> usize {
        match self {
            AnimationState::Idle    => 0,
            AnimationState::Eating  => 1,
            AnimationState::Sleeping => 2,
            AnimationState::Playing => 4,
        }
    }
}

/// Timing information for one animation cycle (no pixel data stored here).
///
/// Frame images are stored separately in `CreatureSlot::cached_*`.
#[derive(Clone)]
pub struct Animation {
    /// Duration of each frame in milliseconds.
    pub durations_ms: Vec<u64>,
    /// Total duration of one cycle in milliseconds.
    pub total_ms: u64,
}

impl Animation {
    /// Create a new Animation from a frame count and per-frame tick durations.
    ///
    /// `frame_count` must match the number of images stored in the
    /// corresponding `CreatureSlot::cached_*` vector.
    /// `tick_durations` are in game ticks (1 tick ≈ 50 ms).
    pub fn new(frame_count: usize, tick_durations: &[u32]) -> Self {
        let count = frame_count.min(tick_durations.len());
        let durations_ms: Vec<u64> = tick_durations[..count]
            .iter()
            .map(|&t| t as u64 * 50)
            .collect();
        let total_ms: u64 = durations_ms.iter().sum();

        Self {
            durations_ms,
            total_ms,
        }
    }

    /// Get the frame index for a given elapsed time (in ms), looping infinitely.
    pub fn frame_index_at(&self, elapsed_ms: u64) -> usize {
        if self.total_ms == 0 || self.durations_ms.is_empty() {
            return 0;
        }

        let elapsed_in_cycle = elapsed_ms % self.total_ms;
        let mut accumulated = 0u64;

        for (i, &dur) in self.durations_ms.iter().enumerate() {
            accumulated += dur;
            if elapsed_in_cycle < accumulated {
                return i;
            }
        }

        self.durations_ms.len() - 1
    }
}

/// The main animator: manages animation state and frame-index selection.
///
/// The animator does **not** own any pixel data — it is purely a timing
/// engine. The caller is responsible for mapping the returned frame index
/// to the correct image in a `CreatureSlot::cached_*` vector.
pub struct Animator {
    state: AnimationState,
    /// When the current state started (for elapsed time calculation).
    state_start: Instant,
    /// Timing animations for each state (no frames stored here).
    idle_anim: Option<Animation>,
    eat_anim: Option<Animation>,
    sleep_anim: Option<Animation>,
    /// Timing for the "Hop" / Playing animation (index 4 in encoded_frames).
    /// Stored separately so `load_animations` signature stays backward-compatible;
    /// set via `set_hop_animation` after `load_animations`.
    hop_anim: Option<Animation>,
}

impl Animator {
    /// Create a new Animator. Call `load_animations` to set up timing data.
    pub fn new() -> Self {
        Self {
            state: AnimationState::Idle,
            state_start: Instant::now(),
            idle_anim: None,
            eat_anim: None,
            sleep_anim: None,
            hop_anim: None,
        }
    }

    /// Load timing animations for the three core states (Idle, Eat, Sleep).
    ///
    /// The `Animation` values here are timing-only; the corresponding
    /// pixel frames must be stored in `CreatureSlot::cached_*`.
    /// Call `set_hop_animation` afterwards to wire up the Playing state.
    pub fn load_animations(
        &mut self,
        idle: Animation,
        eat: Animation,
        sleep: Animation,
    ) {
        self.idle_anim = Some(idle);
        self.eat_anim = Some(eat);
        self.sleep_anim = Some(sleep);
    }

    /// Set the timing animation for the Playing / "Hop" state.
    ///
    /// Kept separate from `load_animations` so the hop timing can be wired in
    /// without changing the existing call sites in `sprite_loading.rs`.
    pub fn set_hop_animation(&mut self, hop: Animation) {
        self.hop_anim = Some(hop);
    }

    /// Get the current animation state.
    pub fn state(&self) -> AnimationState {
        self.state
    }

    /// Switch to a new animation state, resetting the animation timer.
    pub fn set_state(&mut self, state: AnimationState) {
        if self.state != state {
            self.state = state;
            self.state_start = Instant::now();
        }
    }

    /// Get the current frame index based on elapsed time.
    ///
    /// Returns `None` if no timing data has been loaded yet.
    /// The returned index is safe to use directly into `CreatureSlot::cached_*`.
    pub fn current_frame_index(&self) -> Option<usize> {
        let elapsed_ms = self.state_start.elapsed().as_millis() as u64;
        match self.state {
            AnimationState::Idle => {
                let anim = self.idle_anim.as_ref()?;
                Some(anim.frame_index_at(elapsed_ms))
            }
            AnimationState::Eating => {
                let anim = self.eat_anim.as_ref()?;
                Some(anim.frame_index_at(elapsed_ms))
            }
            AnimationState::Sleeping => {
                let anim = self.sleep_anim.as_ref()?;
                Some(anim.frame_index_at(elapsed_ms))
            }
            // Playing uses the hop animation; falls back to idle timing if
            // no hop animation was loaded (same pattern as Eat/Sleep fallback).
            AnimationState::Playing => {
                let anim = self.hop_anim.as_ref().or(self.idle_anim.as_ref())?;
                Some(anim.frame_index_at(elapsed_ms))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_animation() -> Animation {
        // 3 frames, 100 ms each (tick=2 → 2×50=100 ms), 300 ms total.
        Animation::new(3, &[2, 2, 2])
    }

    #[test]
    fn test_animator_starts_idle() {
        let animator = Animator::new();
        assert_eq!(animator.state(), AnimationState::Idle);
    }

    #[test]
    fn test_animator_set_state() {
        let mut animator = Animator::new();
        animator.set_state(AnimationState::Eating);
        assert_eq!(animator.state(), AnimationState::Eating);
        animator.set_state(AnimationState::Sleeping);
        assert_eq!(animator.state(), AnimationState::Sleeping);
        animator.set_state(AnimationState::Idle);
        assert_eq!(animator.state(), AnimationState::Idle);
        // Playing state should round-trip correctly.
        animator.set_state(AnimationState::Playing);
        assert_eq!(animator.state(), AnimationState::Playing);
    }

    #[test]
    fn test_animation_frame_count() {
        let anim = make_test_animation();
        assert_eq!(anim.durations_ms.len(), 3);
        assert_eq!(anim.total_ms, 300); // 3 frames × 100 ms
    }

    #[test]
    fn test_animation_looping_frame_index() {
        let anim = make_test_animation();
        // 100 ms per frame, 300 ms total
        assert_eq!(anim.frame_index_at(0), 0);    // start of frame 0
        assert_eq!(anim.frame_index_at(50), 0);   // middle of frame 0
        assert_eq!(anim.frame_index_at(100), 1);  // start of frame 1
        assert_eq!(anim.frame_index_at(200), 2);  // start of frame 2
        assert_eq!(anim.frame_index_at(300), 0);  // loops back
        assert_eq!(anim.frame_index_at(400), 1);  // looped frame 1
    }

    #[test]
    fn test_eating_state_is_stable() {
        // Eating should not spontaneously transition back to Idle.
        let mut animator = Animator::new();
        animator.load_animations(
            make_test_animation(),
            make_test_animation(),
            make_test_animation(),
        );
        animator.set_state(AnimationState::Eating);
        assert_eq!(animator.state(), AnimationState::Eating);
    }

    #[test]
    fn test_playing_state_is_stable() {
        // Playing should not spontaneously transition to another state.
        let mut animator = Animator::new();
        animator.load_animations(
            make_test_animation(),
            make_test_animation(),
            make_test_animation(),
        );
        animator.set_hop_animation(make_test_animation());
        animator.set_state(AnimationState::Playing);
        assert_eq!(animator.state(), AnimationState::Playing);
    }

    #[test]
    fn test_encoded_index() {
        assert_eq!(AnimationState::Idle.encoded_index(), 0);
        assert_eq!(AnimationState::Eating.encoded_index(), 1);
        assert_eq!(AnimationState::Sleeping.encoded_index(), 2);
        assert_eq!(AnimationState::Playing.encoded_index(), 4);
    }

    #[test]
    fn test_current_frame_index_returns_none_without_animations() {
        let animator = Animator::new();
        assert!(animator.current_frame_index().is_none());
    }

    #[test]
    fn test_current_frame_index_returns_some_with_animations() {
        let mut animator = Animator::new();
        animator.load_animations(
            make_test_animation(),
            make_test_animation(),
            make_test_animation(),
        );
        assert!(animator.current_frame_index().is_some());
    }

    #[test]
    fn test_playing_state_falls_back_to_idle_when_no_hop_anim() {
        // If no hop animation was loaded, Playing should fall back to idle timing.
        let mut animator = Animator::new();
        animator.load_animations(
            make_test_animation(),
            make_test_animation(),
            make_test_animation(),
        );
        // No set_hop_animation() call — should still return Some (using idle fallback).
        animator.set_state(AnimationState::Playing);
        assert!(
            animator.current_frame_index().is_some(),
            "Playing state should fall back to idle timing when hop_anim is None"
        );
    }

    #[test]
    fn test_playing_state_uses_hop_anim_when_loaded() {
        // When a hop animation is loaded, Playing should use it.
        let mut animator = Animator::new();
        animator.load_animations(
            make_test_animation(),
            make_test_animation(),
            make_test_animation(),
        );
        animator.set_hop_animation(make_test_animation());
        animator.set_state(AnimationState::Playing);
        assert!(
            animator.current_frame_index().is_some(),
            "Playing state should return a frame index when hop_anim is loaded"
        );
    }
}
