use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use std::time::{Duration, Instant};

/// The creature's current animation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationState {
    Idle,
    Eating,
    Sleeping,
}

/// Which idle variant is currently playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdleVariant {
    Breathe,  // Gentle vertical scale oscillation
    Bounce,   // Small hop up and down
    Sway,     // Slight horizontal lean
}

impl IdleVariant {
    fn next(self) -> Self {
        match self {
            IdleVariant::Breathe => IdleVariant::Bounce,
            IdleVariant::Bounce => IdleVariant::Sway,
            IdleVariant::Sway => IdleVariant::Breathe,
        }
    }

    /// How long this idle variant plays before cycling to the next.
    fn duration(self) -> Duration {
        match self {
            IdleVariant::Breathe => Duration::from_secs(4),
            IdleVariant::Bounce => Duration::from_secs(3),
            IdleVariant::Sway => Duration::from_secs(3),
        }
    }
}

pub struct Animator {
    state: AnimationState,
    idle_variant: IdleVariant,
    start_time: Instant,
    idle_variant_start: Instant,
    state_end: Option<Instant>,
}

impl Animator {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            state: AnimationState::Idle,
            idle_variant: IdleVariant::Breathe,
            start_time: now,
            idle_variant_start: now,
            state_end: None,
        }
    }

    pub fn state(&self) -> AnimationState {
        self.state
    }

    /// Transition to a new animation state.
    pub fn set_state(&mut self, state: AnimationState) {
        let now = Instant::now();
        self.state = state;
        self.start_time = now;
        self.state_end = match state {
            AnimationState::Idle => None,
            AnimationState::Eating => Some(now + Duration::from_secs(3)),
            AnimationState::Sleeping => Some(now + Duration::from_secs(5)),
        };
        if state == AnimationState::Idle {
            self.idle_variant = IdleVariant::Breathe;
            self.idle_variant_start = now;
        }
    }

    /// Call each frame. Returns true if the animation state changed.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();

        if let Some(end) = self.state_end {
            if now >= end {
                self.set_state(AnimationState::Idle);
                return true;
            }
        }

        if self.state == AnimationState::Idle {
            let elapsed = now.duration_since(self.idle_variant_start);
            if elapsed >= self.idle_variant.duration() {
                self.idle_variant = self.idle_variant.next();
                self.idle_variant_start = now;
            }
        }

        false
    }

    /// Generate the current animation frame from the base sprite.
    pub fn render_frame(&self, base: &DynamicImage) -> DynamicImage {
        let elapsed = Instant::now()
            .duration_since(self.start_time)
            .as_secs_f64();

        match self.state {
            AnimationState::Idle => self.render_idle(base),
            AnimationState::Eating => eating_effect(base, elapsed),
            AnimationState::Sleeping => sleeping_effect(base, elapsed),
        }
    }

    fn render_idle(&self, base: &DynamicImage) -> DynamicImage {
        let variant_elapsed = Instant::now()
            .duration_since(self.idle_variant_start)
            .as_secs_f64();

        match self.idle_variant {
            IdleVariant::Breathe => breathe_effect(base, variant_elapsed),
            IdleVariant::Bounce => bounce_effect(base, variant_elapsed),
            IdleVariant::Sway => sway_effect(base, variant_elapsed),
        }
    }
}

// --- Animation Effects ---

/// Breathe: gentle vertical squash/stretch oscillation.
fn breathe_effect(base: &DynamicImage, t: f64) -> DynamicImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);

    let scale_y = 1.0 + 0.03 * (t * 2.0 * std::f64::consts::PI / 2.0).sin();
    let scale_x = 1.0 - 0.015 * (t * 2.0 * std::f64::consts::PI / 2.0).sin();

    let cx = w as f64 / 2.0;
    let cy = h as f64;

    for y in 0..h {
        for x in 0..w {
            let src_x = cx + (x as f64 - cx) / scale_x;
            let src_y = cy + (y as f64 - cy) / scale_y;

            if src_x >= 0.0 && src_x < w as f64 && src_y >= 0.0 && src_y < h as f64 {
                let px = base.get_pixel(src_x as u32, src_y as u32);
                out.put_pixel(x, y, px);
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

/// Bounce: small hop up and down.
fn bounce_effect(base: &DynamicImage, t: f64) -> DynamicImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);

    let phase = (t * 3.0 * std::f64::consts::PI).sin();
    let offset_y = (phase.abs() * 4.0) as i32;

    for y in 0..h {
        for x in 0..w {
            let src_y = y as i32 + offset_y;
            if src_y >= 0 && src_y < h as i32 {
                let px = base.get_pixel(x, src_y as u32);
                out.put_pixel(x, y, px);
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

/// Sway: slight horizontal lean left/right.
fn sway_effect(base: &DynamicImage, t: f64) -> DynamicImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);

    let sway_amount = (t * 2.0 * std::f64::consts::PI / 2.5).sin() * 3.0;

    for y in 0..h {
        let row_factor = 1.0 - (y as f64 / h as f64);
        let offset_x = (sway_amount * row_factor) as i32;

        for x in 0..w {
            let src_x = x as i32 - offset_x;
            if src_x >= 0 && src_x < w as i32 {
                let px = base.get_pixel(src_x as u32, y);
                out.put_pixel(x, y, px);
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

/// Eating: quick chomping motion — squash vertically in pulses.
fn eating_effect(base: &DynamicImage, t: f64) -> DynamicImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);

    let chomp = ((t * 6.0 * std::f64::consts::PI).sin().abs()) * 0.08;
    let scale_y = 1.0 - chomp;
    let scale_x = 1.0 + chomp * 0.5;

    let cx = w as f64 / 2.0;
    let cy = h as f64;

    for y in 0..h {
        for x in 0..w {
            let src_x = cx + (x as f64 - cx) / scale_x;
            let src_y = cy + (y as f64 - cy) / scale_y;

            if src_x >= 0.0 && src_x < w as f64 && src_y >= 0.0 && src_y < h as f64 {
                let px = base.get_pixel(src_x as u32, src_y as u32);
                out.put_pixel(x, y, px);
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

/// Sleeping: slow breathing + slight darken/blue tint, with periodic head nod.
fn sleeping_effect(base: &DynamicImage, t: f64) -> DynamicImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);

    let scale_y = 1.0 + 0.02 * (t * std::f64::consts::PI / 2.0).sin();

    let nod_phase = (t * std::f64::consts::PI / 3.0).sin();
    let nod = if nod_phase > 0.7 { (nod_phase - 0.7) * 8.0 } else { 0.0 };

    let cx = w as f64 / 2.0;
    let cy = h as f64;

    for y in 0..h {
        let row_factor = 1.0 - (y as f64 / h as f64);
        let nod_offset = (nod * row_factor) as i32;

        for x in 0..w {
            let src_x = cx + (x as f64 - cx);
            let src_y_base = cy + (y as f64 - cy) / scale_y;
            let src_y = src_y_base - nod_offset as f64;

            if src_x >= 0.0 && src_x < w as f64 && src_y >= 0.0 && src_y < h as f64 {
                let px = base.get_pixel(src_x as u32, src_y as u32);
                let r = (px[0] as f64 * 0.85) as u8;
                let g = (px[1] as f64 * 0.85) as u8;
                let b = (px[2] as f64 * 0.95).min(255.0) as u8;
                out.put_pixel(x, y, Rgba([r, g, b, px[3]]));
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbaImage, Rgba};

    fn test_sprite() -> DynamicImage {
        let mut img = RgbaImage::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                img.put_pixel(x, y, Rgba([255, 200, 50, 255]));
            }
        }
        DynamicImage::ImageRgba8(img)
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
    fn test_render_frame_produces_same_dimensions() {
        let animator = Animator::new();
        let base = test_sprite();
        let frame = animator.render_frame(&base);
        assert_eq!(frame.dimensions(), base.dimensions());
    }

    #[test]
    fn test_all_effects_produce_valid_frames() {
        let base = test_sprite();

        let f1 = breathe_effect(&base, 0.5);
        assert_eq!(f1.dimensions(), base.dimensions());

        let f2 = bounce_effect(&base, 0.5);
        assert_eq!(f2.dimensions(), base.dimensions());

        let f3 = sway_effect(&base, 0.5);
        assert_eq!(f3.dimensions(), base.dimensions());

        let f4 = eating_effect(&base, 0.5);
        assert_eq!(f4.dimensions(), base.dimensions());

        let f5 = sleeping_effect(&base, 0.5);
        assert_eq!(f5.dimensions(), base.dimensions());
    }

    #[test]
    fn test_sleeping_applies_tint() {
        let base = test_sprite();
        let frame = sleeping_effect(&base, 0.0);
        let frame_rgba = frame.to_rgba8();

        let px = frame_rgba.get_pixel(16, 16);
        assert!(px[0] < 255, "Red channel should be dimmed");
        assert!(px[1] < 200, "Green channel should be dimmed");
    }

    #[test]
    fn test_idle_variant_cycling() {
        let mut v = IdleVariant::Breathe;
        let variants: Vec<_> = (0..3).map(|_| { let current = v; v = v.next(); current }).collect();
        assert_eq!(variants, vec![IdleVariant::Breathe, IdleVariant::Bounce, IdleVariant::Sway]);
        assert_eq!(v, IdleVariant::Breathe);
    }
}
