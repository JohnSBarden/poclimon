//! Sprite sheet-based animation system.
//!
//! Replaces the old programmatic animation (squash/stretch, pixel manipulation)
//! with proper pre-rendered sprite sheet animations from PMDCollab.
//!
//! Each animation state (Idle, Eating, Sleeping) plays a corresponding
//! sprite sheet animation. Frame timing comes from AnimData.xml durations.

use image::DynamicImage;
use std::time::Instant;

/// The creature's current animation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationState {
    Idle,
    Eating,
    Sleeping,
}

/// A loaded animation: its frames and per-frame timing.
#[derive(Clone)]
pub struct Animation {
    /// Individual frames extracted from the sprite sheet (row 0 only).
    pub frames: Vec<DynamicImage>,
    /// Duration of each frame in milliseconds.
    pub durations_ms: Vec<u64>,
    /// Total duration of one cycle in milliseconds.
    pub total_ms: u64,
}

impl Animation {
    /// Create a new Animation from frames and tick durations.
    /// `tick_durations` are in game ticks (1 tick ≈ 50ms).
    pub fn new(frames: Vec<DynamicImage>, tick_durations: &[u32]) -> Self {
        // Use the minimum of frames and durations to stay in bounds
        let count = frames.len().min(tick_durations.len());
        let durations_ms: Vec<u64> = tick_durations[..count]
            .iter()
            .map(|&t| t as u64 * 50)
            .collect();
        let total_ms: u64 = durations_ms.iter().sum();

        Self {
            frames: frames[..count].to_vec(),
            durations_ms,
            total_ms,
        }
    }

    /// Get the frame index for a given elapsed time (in ms), looping.
    fn frame_index_looping(&self, elapsed_ms: u64) -> usize {
        if self.total_ms == 0 || self.frames.is_empty() {
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

        self.frames.len() - 1
    }

    /// Get the frame index for a given elapsed time (in ms), clamping at end.
    fn frame_index_once(&self, elapsed_ms: u64) -> (usize, bool) {
        if self.total_ms == 0 || self.frames.is_empty() {
            return (0, true);
        }

        if elapsed_ms >= self.total_ms {
            return (self.frames.len() - 1, true);
        }

        let mut accumulated = 0u64;
        for (i, &dur) in self.durations_ms.iter().enumerate() {
            accumulated += dur;
            if elapsed_ms < accumulated {
                return (i, false);
            }
        }

        (self.frames.len() - 1, true)
    }
}

/// The main animator that manages animation state and frame selection.
pub struct Animator {
    state: AnimationState,
    /// When the current state started (for elapsed time calculation).
    state_start: Instant,
    /// Loaded animations for each state.
    idle_anim: Option<Animation>,
    eat_anim: Option<Animation>,
    sleep_anim: Option<Animation>,
}

impl Animator {
    /// Create a new Animator. Call `load_animations` to set up the sprite data.
    pub fn new() -> Self {
        Self {
            state: AnimationState::Idle,
            state_start: Instant::now(),
            idle_anim: None,
            eat_anim: None,
            sleep_anim: None,
        }
    }

    /// Load animations for all three states.
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

    /// Get the current animation state.
    pub fn state(&self) -> AnimationState {
        self.state
    }

    /// Switch to a new animation state.
    pub fn set_state(&mut self, state: AnimationState) {
        if self.state != state {
            self.state = state;
            self.state_start = Instant::now();
        }
    }

    /// Advance the animation. Call this each frame.
    /// Returns true if the state changed (e.g., Eating finished → Idle).
    pub fn tick(&mut self) -> bool {
        // If eating, check if the animation has finished playing once
        if self.state == AnimationState::Eating {
            if let Some(ref anim) = self.eat_anim {
                let elapsed_ms = self.state_start.elapsed().as_millis() as u64;
                let (_, done) = anim.frame_index_once(elapsed_ms);
                if done {
                    self.state = AnimationState::Idle;
                    self.state_start = Instant::now();
                    return true;
                }
            }
        }
        false
    }

    /// Get the current animation frame to display.
    /// Returns None if no animations are loaded.
    pub fn render_frame(&self) -> Option<&DynamicImage> {
        let elapsed_ms = self.state_start.elapsed().as_millis() as u64;

        match self.state {
            AnimationState::Idle => {
                let anim = self.idle_anim.as_ref()?;
                let idx = anim.frame_index_looping(elapsed_ms);
                anim.frames.get(idx)
            }
            AnimationState::Sleeping => {
                let anim = self.sleep_anim.as_ref()?;
                let idx = anim.frame_index_looping(elapsed_ms);
                anim.frames.get(idx)
            }
            AnimationState::Eating => {
                let anim = self.eat_anim.as_ref()?;
                let (idx, _) = anim.frame_index_once(elapsed_ms);
                anim.frames.get(idx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    /// Make a simple 4x4 colored frame for testing.
    fn make_frame(r: u8, g: u8, b: u8) -> DynamicImage {
        let mut img = RgbaImage::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.put_pixel(x, y, Rgba([r, g, b, 255]));
            }
        }
        DynamicImage::ImageRgba8(img)
    }

    fn make_test_animation() -> Animation {
        let frames = vec![
            make_frame(255, 0, 0),
            make_frame(0, 255, 0),
            make_frame(0, 0, 255),
        ];
        Animation::new(frames, &[2, 2, 2]) // 100ms each, 300ms total
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
    }

    #[test]
    fn test_animation_frame_count() {
        let anim = make_test_animation();
        assert_eq!(anim.frames.len(), 3);
        assert_eq!(anim.total_ms, 300); // 3 frames × 100ms
    }

    #[test]
    fn test_animation_looping_frame_index() {
        let anim = make_test_animation();
        // 100ms per frame, 300ms total
        assert_eq!(anim.frame_index_looping(0), 0);    // start of frame 0
        assert_eq!(anim.frame_index_looping(50), 0);   // middle of frame 0
        assert_eq!(anim.frame_index_looping(100), 1);  // start of frame 1
        assert_eq!(anim.frame_index_looping(200), 2);  // start of frame 2
        assert_eq!(anim.frame_index_looping(300), 0);  // loops back
        assert_eq!(anim.frame_index_looping(400), 1);  // looped frame 1
    }

    #[test]
    fn test_animation_once_frame_index() {
        let anim = make_test_animation();
        assert_eq!(anim.frame_index_once(0), (0, false));
        assert_eq!(anim.frame_index_once(150), (1, false));
        assert_eq!(anim.frame_index_once(300), (2, true));  // done
        assert_eq!(anim.frame_index_once(500), (2, true));  // still done
    }

    #[test]
    fn test_render_frame_returns_none_without_animations() {
        let animator = Animator::new();
        assert!(animator.render_frame().is_none());
    }

    #[test]
    fn test_render_frame_returns_some_with_animations() {
        let mut animator = Animator::new();
        animator.load_animations(
            make_test_animation(),
            make_test_animation(),
            make_test_animation(),
        );
        assert!(animator.render_frame().is_some());
    }
}
