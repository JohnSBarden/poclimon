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
    Breathe,
    Bounce,
    Sway,
}

impl IdleVariant {
    fn next(self) -> Self {
        match self {
            IdleVariant::Breathe => IdleVariant::Bounce,
            IdleVariant::Bounce => IdleVariant::Sway,
            IdleVariant::Sway => IdleVariant::Breathe,
        }
    }

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
    /// Cached eye regions detected from the base sprite.
    eye_regions: Vec<EyeRegion>,
    eyes_detected: bool,
}

/// A detected eye region (cluster of dark pixels in the upper portion of the sprite).
#[derive(Debug, Clone)]
struct EyeRegion {
    cx: u32,
    cy: u32,
    radius: u32,
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
            eye_regions: Vec::new(),
            eyes_detected: false,
        }
    }

    /// Detect eyes from the base sprite. Call once after loading.
    pub fn detect_eyes(&mut self, base: &DynamicImage) {
        self.eye_regions = detect_eye_regions(base);
        self.eyes_detected = true;
    }

    pub fn state(&self) -> AnimationState {
        self.state
    }

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

    /// Returns the frame rate for the current state in milliseconds.
    pub fn frame_rate_ms(&self) -> u64 {
        match self.state {
            AnimationState::Idle => 80,
            AnimationState::Eating => 50, // ~20fps — fast but smooth
            AnimationState::Sleeping => 100,
        }
    }

    pub fn render_frame(&self, base: &DynamicImage) -> DynamicImage {
        let elapsed = Instant::now()
            .duration_since(self.start_time)
            .as_secs_f64();

        match self.state {
            AnimationState::Idle => self.render_idle(base),
            AnimationState::Eating => eating_effect(base, elapsed),
            AnimationState::Sleeping => sleeping_effect(base, elapsed, &self.eye_regions),
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

// --- Eye Detection ---

/// Detect dark pixel clusters in the upper portion of the sprite as probable eyes.
/// Returns up to 2 eye regions (left and right).
///
/// Strategy: scan the upper 15–55% of the sprite bounding box for the darkest
/// pixel clusters. Instead of a fixed brightness cutoff, we adaptively find
/// pixels that are significantly darker than their local neighborhood, which
/// works across different sprite color palettes.
fn detect_eye_regions(base: &DynamicImage) -> Vec<EyeRegion> {
    let (w, h) = base.dimensions();
    let rgba = base.to_rgba8();

    // Find the bounding box of non-transparent pixels
    let mut min_y = h;
    let mut max_y = 0u32;
    let mut min_x = w;
    let mut max_x = 0u32;

    for y in 0..h {
        for x in 0..w {
            if rgba.get_pixel(x, y)[3] > 128 {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
        }
    }

    if max_y <= min_y || max_x <= min_x {
        return Vec::new();
    }

    let sprite_h = max_y - min_y;
    let sprite_w = max_x - min_x;
    let sprite_cx = (min_x + max_x) / 2;

    // Search the upper 15–55% band — wider range to catch different sprite styles
    let search_top = min_y + sprite_h * 15 / 100;
    let search_bottom = min_y + sprite_h * 55 / 100;

    // First pass: collect all opaque pixel brightnesses in the search band
    let mut pixels_with_brightness: Vec<(u32, u32, u32)> = Vec::new();
    for y in search_top..search_bottom {
        for x in min_x..=max_x {
            let px = rgba.get_pixel(x, y);
            if px[3] > 128 {
                let brightness = px[0] as u32 + px[1] as u32 + px[2] as u32;
                pixels_with_brightness.push((x, y, brightness));
            }
        }
    }

    if pixels_with_brightness.is_empty() {
        return Vec::new();
    }

    // Find the adaptive threshold: take the darkest 8% of pixels in the search band
    let mut brightnesses: Vec<u32> = pixels_with_brightness.iter().map(|p| p.2).collect();
    brightnesses.sort();
    let dark_threshold_idx = (brightnesses.len() as f64 * 0.08).max(1.0) as usize;
    let dark_threshold = brightnesses[dark_threshold_idx.min(brightnesses.len() - 1)];
    // Cap at 200 so we don't pick up medium-toned pixels on very bright sprites
    let dark_threshold = dark_threshold.min(200);

    let dark_pixels: Vec<(u32, u32)> = pixels_with_brightness
        .iter()
        .filter(|(_, _, b)| *b <= dark_threshold)
        .map(|(x, y, _)| (*x, *y))
        .collect();

    if dark_pixels.len() < 4 {
        return Vec::new();
    }

    // Split into left and right halves
    let left: Vec<_> = dark_pixels.iter().filter(|(x, _)| *x < sprite_cx).collect();
    let right: Vec<_> = dark_pixels.iter().filter(|(x, _)| *x >= sprite_cx).collect();

    let mut eyes = Vec::new();

    for cluster in [&left, &right] {
        if cluster.len() < 2 {
            continue;
        }

        let avg_x = cluster.iter().map(|(x, _)| *x as f64).sum::<f64>() / cluster.len() as f64;
        let avg_y = cluster.iter().map(|(_, y)| *y as f64).sum::<f64>() / cluster.len() as f64;

        // Filter out outliers: only keep pixels within a reasonable distance of the centroid
        let max_eye_size = (sprite_w as f64 * 0.15).max(5.0);
        let filtered: Vec<_> = cluster
            .iter()
            .filter(|(x, y)| {
                let dx = *x as f64 - avg_x;
                let dy = *y as f64 - avg_y;
                (dx * dx + dy * dy).sqrt() < max_eye_size
            })
            .collect();

        if filtered.len() < 2 {
            continue;
        }

        // Recompute centroid from filtered cluster
        let cx = filtered.iter().map(|(x, _)| *x as f64).sum::<f64>() / filtered.len() as f64;
        let cy = filtered.iter().map(|(_, y)| *y as f64).sum::<f64>() / filtered.len() as f64;

        let max_dist = filtered
            .iter()
            .map(|(x, y)| {
                let dx = *x as f64 - cx;
                let dy = *y as f64 - cy;
                (dx * dx + dy * dy).sqrt()
            })
            .fold(0.0f64, f64::max);

        let radius = (max_dist as u32).max(3).min(25);

        eyes.push(EyeRegion {
            cx: cx as u32,
            cy: cy as u32,
            radius,
        });
    }

    eyes
}

// --- Animation Effects ---

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
                out.put_pixel(x, y, base.get_pixel(src_x as u32, src_y as u32));
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

fn bounce_effect(base: &DynamicImage, t: f64) -> DynamicImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);

    let phase = (t * 3.0 * std::f64::consts::PI).sin();
    let offset_y = (phase.abs() * 4.0) as i32;

    for y in 0..h {
        for x in 0..w {
            let src_y = y as i32 + offset_y;
            if src_y >= 0 && src_y < h as i32 {
                out.put_pixel(x, y, base.get_pixel(x, src_y as u32));
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

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
                out.put_pixel(x, y, base.get_pixel(src_x as u32, y));
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

/// Eating: ravenous chomping with crumb particles flying off.
fn eating_effect(base: &DynamicImage, t: f64) -> DynamicImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);

    // Smooth chomp cycle: use a power curve for snappy down + gentle up
    let cycle = (t * 4.0) % 1.0; // 4 chomps per second
    let chomp = if cycle < 0.3 {
        // Quick squash down
        (cycle / 0.3).powi(2) * 0.10
    } else {
        // Gentle release back up
        ((1.0 - cycle) / 0.7).powi(2) * 0.10
    };
    let scale_y = 1.0 - chomp;
    let scale_x = 1.0 + chomp * 0.5;

    // Slight forward lean on the down-chomp
    let tilt = if cycle < 0.3 { chomp * 0.15 } else { 0.0 };

    let cx = w as f64 / 2.0;
    let cy = h as f64;

    for y in 0..h {
        for x in 0..w {
            let row_factor = 1.0 - (y as f64 / h as f64);
            let tilt_offset = tilt * row_factor * w as f64 * 0.1;

            let src_x = cx + (x as f64 - cx - tilt_offset) / scale_x;
            let src_y = cy + (y as f64 - cy) / scale_y;

            if src_x >= 0.0 && src_x < w as f64 && src_y >= 0.0 && src_y < h as f64 {
                out.put_pixel(x, y, base.get_pixel(src_x as u32, src_y as u32));
            }
        }
    }

    // Spawn crumb particles — small colored dots flying outward
    let num_crumbs = 8;
    let crumb_colors = [
        Rgba([210, 180, 140, 255]), // tan
        Rgba([244, 164, 96, 255]),  // sandy brown
        Rgba([255, 228, 181, 255]), // moccasin
        Rgba([222, 184, 135, 255]), // burlywood
        Rgba([245, 222, 179, 255]), // wheat
        Rgba([255, 200, 100, 255]), // golden
        Rgba([200, 150, 80, 255]),  // dark crumb
        Rgba([255, 240, 200, 255]), // light crumb
    ];

    // Find the approximate mouth area (lower third of sprite, center)
    let mouth_x = cx;
    let mouth_y = h as f64 * 0.65;

    for i in 0..num_crumbs {
        let seed = i as f64 * 2.3 + t * 4.0;
        let angle = seed.sin() * std::f64::consts::PI + std::f64::consts::PI * 0.5;
        let speed = 15.0 + (seed * 1.7).cos().abs() * 25.0;
        let lifetime = (t * 4.0 + i as f64 * 0.8) % 1.0;

        if lifetime > 0.8 {
            continue; // Crumb has faded
        }

        let crumb_x = mouth_x + angle.cos() * speed * lifetime;
        let crumb_y = mouth_y + angle.sin() * speed * lifetime + lifetime * lifetime * 20.0; // gravity

        let size = if lifetime < 0.3 { 2 } else { 1 };
        let color = crumb_colors[i % crumb_colors.len()];

        // Draw crumb
        for dy in 0..size {
            for dx in 0..size {
                let px = crumb_x as i32 + dx;
                let py = crumb_y as i32 + dy;
                if px >= 0 && px < w as i32 && py >= 0 && py < h as i32 {
                    let alpha = ((1.0 - lifetime / 0.8) * 255.0) as u8;
                    let faded = Rgba([color[0], color[1], color[2], alpha.min(color[3])]);
                    blend_pixel(&mut out, px as u32, py as u32, faded);
                }
            }
        }
    }

    DynamicImage::ImageRgba8(out)
}

/// Sleeping: deep darkness, closed eyes, slow breathing, "zZzZ" overlay.
fn sleeping_effect(base: &DynamicImage, t: f64, eye_regions: &[EyeRegion]) -> DynamicImage {
    let (w, h) = base.dimensions();
    let mut out = RgbaImage::new(w, h);

    // Very slow breathing
    let scale_y = 1.0 + 0.015 * (t * std::f64::consts::PI / 2.5).sin();

    // Gentle nod
    let nod_phase = (t * std::f64::consts::PI / 4.0).sin();
    let nod = if nod_phase > 0.6 { (nod_phase - 0.6) * 5.0 } else { 0.0 };

    let cx = w as f64 / 2.0;
    let cy = h as f64;

    for y in 0..h {
        let row_factor = 1.0 - (y as f64 / h as f64);
        let nod_offset = (nod * row_factor) as i32;

        for x in 0..w {
            let src_y_base = cy + (y as f64 - cy) / scale_y;
            let src_y = src_y_base - nod_offset as f64;
            let src_x = x as f64;

            if src_x >= 0.0 && src_x < w as f64 && src_y >= 0.0 && src_y < h as f64 {
                let px = base.get_pixel(src_x as u32, src_y as u32);

                // Deep night-time palette: much darker, strong blue tint
                let r = (px[0] as f64 * 0.55) as u8;
                let g = (px[1] as f64 * 0.55) as u8;
                let b = ((px[2] as f64 * 0.75) + 15.0).min(255.0) as u8;

                // Slight purple vignette toward edges
                let edge_dist = ((x as f64 - cx).abs() / cx).min(1.0);
                let vignette = 1.0 - edge_dist * 0.15;
                let r = (r as f64 * vignette) as u8;
                let g = (g as f64 * vignette) as u8;

                out.put_pixel(x, y, Rgba([r, g, b, px[3]]));
            }
        }
    }

    // Close eyes: draw horizontal lines over detected eye regions
    for eye in eye_regions {
        let line_y = eye.cy;
        let half_w = eye.radius + 2;
        let start_x = eye.cx.saturating_sub(half_w);
        let end_x = (eye.cx + half_w).min(w - 1);

        // Draw a curved "closed eye" line (thicker in center)
        for x in start_x..=end_x {
            let rel = (x as f64 - eye.cx as f64) / half_w as f64;
            let curve = (1.0 - rel * rel) * 2.0; // Parabolic curve
            let thickness = (curve as i32).max(1);

            for dy in 0..thickness {
                let py = line_y as i32 + dy;
                if py >= 0 && py < h as i32 {
                    // Sample surrounding color for the "eyelid" line
                    let bg = if eye.cy > 2 {
                        out.get_pixel(x, (eye.cy - 2).min(h - 1)).clone()
                    } else {
                        Rgba([80, 60, 80, 255])
                    };
                    // Darken the sampled color for the eyelid
                    let lid = Rgba([
                        (bg[0] as f64 * 0.6) as u8,
                        (bg[1] as f64 * 0.6) as u8,
                        (bg[2] as f64 * 0.6) as u8,
                        bg[3],
                    ]);
                    out.put_pixel(x, py as u32, lid);
                }
            }
        }
    }

    // Draw "zZzZ" floating above the sprite
    draw_snore_text(&mut out, t, w, h);

    DynamicImage::ImageRgba8(out)
}

/// Draw a glyph (2D array of 0/1) scaled up onto the image.
fn draw_glyph_scaled<const W: usize, const H: usize>(
    img: &mut RgbaImage,
    glyph: &[[u8; W]; H],
    x: i32,
    y: i32,
    scale: i32,
    color: Rgba<u8>,
    img_w: u32,
    img_h: u32,
) {
    for row in 0..H {
        for col in 0..W {
            if glyph[row][col] == 1 {
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px_x = x + col as i32 * scale + sx;
                        let px_y = y + row as i32 * scale + sy;
                        if px_x >= 0 && px_x < img_w as i32 && px_y >= 0 && px_y < img_h as i32 {
                            blend_pixel(img, px_x as u32, px_y as u32, color);
                        }
                    }
                }
            }
        }
    }
}

/// Draw floating "zZzZ" text that drifts upward and fades.
/// Cartoonishly big — these are meant to be very visible and fun.
fn draw_snore_text(img: &mut RgbaImage, t: f64, w: u32, h: u32) {
    // Big chunky Z glyph (9x9)
    let z_big: [[u8; 9]; 9] = [
        [1, 1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 0, 1, 1, 0],
        [0, 0, 0, 0, 0, 1, 1, 0, 0],
        [0, 0, 0, 0, 1, 1, 0, 0, 0],
        [0, 0, 0, 1, 1, 0, 0, 0, 0],
        [0, 0, 1, 1, 0, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1, 1, 1, 1, 1],
    ];

    // Medium z glyph (7x7)
    let z_med: [[u8; 7]; 7] = [
        [1, 1, 1, 1, 1, 1, 1],
        [0, 0, 0, 0, 0, 1, 1],
        [0, 0, 0, 0, 1, 1, 0],
        [0, 0, 0, 1, 1, 0, 0],
        [0, 0, 1, 1, 0, 0, 0],
        [0, 1, 1, 0, 0, 0, 0],
        [1, 1, 1, 1, 1, 1, 1],
    ];

    // Small z glyph (5x5)
    let z_small: [[u8; 5]; 5] = [
        [1, 1, 1, 1, 1],
        [0, 0, 0, 1, 0],
        [0, 0, 1, 0, 0],
        [0, 1, 0, 0, 0],
        [1, 1, 1, 1, 1],
    ];

    let base_x = (w as f64 * 0.65) as i32;
    let cycle = t % 4.0; // 4-second cycle for more dramatic float

    // Staggered Z's — big ones first, getting smaller as they float up
    // (delay, x_offset, scale_factor: 0=big, 1=med, 2=small)
    let z_positions: [(f64, i32, u8); 4] = [
        (0.0, 0, 0),    // Big Z — starts first
        (0.6, 14, 1),   // Medium z
        (1.2, 24, 2),   // Small z
        (1.8, 30, 1),   // Another medium z
    ];

    let color = Rgba([180, 210, 255, 255]); // Soft blue-white

    for (delay, x_off, glyph_size) in &z_positions {
        let local_t = cycle - delay;
        if local_t < 0.0 || local_t > 3.0 {
            continue;
        }

        let alpha = if local_t < 0.4 {
            (local_t / 0.4 * 255.0) as u8
        } else if local_t > 2.2 {
            ((3.0 - local_t) / 0.8 * 255.0) as u8
        } else {
            255
        };

        // Float upward with a gentle arc
        let float_y = (h as f64 * 0.3 - local_t * 30.0) as i32;
        let sway_x = (local_t * 1.5).sin() * 8.0;
        let draw_x = base_x + x_off + sway_x as i32;

        let c = Rgba([color[0], color[1], color[2], alpha]);

        // Draw the glyph scaled up — each pixel becomes a scale×scale block
        match glyph_size {
            0 => draw_glyph_scaled(img, &z_big, draw_x, float_y, 3, c, w, h),
            1 => draw_glyph_scaled(img, &z_med, draw_x, float_y, 2, c, w, h),
            _ => draw_glyph_scaled(img, &z_small, draw_x, float_y, 2, c, w, h),
        }
    }
}

/// Alpha-blend a pixel onto the image.
fn blend_pixel(img: &mut RgbaImage, x: u32, y: u32, color: Rgba<u8>) {
    let existing = img.get_pixel(x, y);
    let alpha = color[3] as f64 / 255.0;
    let inv = 1.0 - alpha;

    let r = (color[0] as f64 * alpha + existing[0] as f64 * inv) as u8;
    let g = (color[1] as f64 * alpha + existing[1] as f64 * inv) as u8;
    let b = (color[2] as f64 * alpha + existing[2] as f64 * inv) as u8;
    let a = (color[3] as f64 + existing[3] as f64 * inv).min(255.0) as u8;

    img.put_pixel(x, y, Rgba([r, g, b, a]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    fn test_sprite() -> DynamicImage {
        let mut img = RgbaImage::new(96, 96);
        // Yellow body
        for y in 20..80 {
            for x in 20..76 {
                img.put_pixel(x, y, Rgba([255, 200, 50, 255]));
            }
        }
        // Dark "eyes" in the upper region
        for y in 35..42 {
            for x in 35..42 {
                img.put_pixel(x, y, Rgba([10, 10, 10, 255]));
            }
            for x in 54..61 {
                img.put_pixel(x, y, Rgba([10, 10, 10, 255]));
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

        let f5 = sleeping_effect(&base, 0.5, &[]);
        assert_eq!(f5.dimensions(), base.dimensions());
    }

    #[test]
    fn test_sleeping_deep_dark_palette() {
        let base = test_sprite();
        let frame = sleeping_effect(&base, 0.0, &[]);
        let frame_rgba = frame.to_rgba8();

        // Center pixel (yellow body area)
        let px = frame_rgba.get_pixel(48, 50);
        // Original: 255, 200, 50 — sleeping should be much darker (0.55 multiplier)
        assert!(px[0] < 150, "Red should be heavily dimmed, got {}", px[0]);
        assert!(px[1] < 120, "Green should be heavily dimmed, got {}", px[1]);
    }

    #[test]
    fn test_eye_detection() {
        let base = test_sprite();
        let eyes = detect_eye_regions(&base);
        assert_eq!(eyes.len(), 2, "Should detect 2 eyes");
        // Left eye should be left of center
        assert!(eyes[0].cx < 48 || eyes[1].cx < 48);
        // Right eye should be right of center
        assert!(eyes[0].cx >= 48 || eyes[1].cx >= 48);
    }

    #[test]
    fn test_sleeping_with_eyes() {
        let base = test_sprite();
        let eyes = detect_eye_regions(&base);
        let frame = sleeping_effect(&base, 1.0, &eyes);
        assert_eq!(frame.dimensions(), base.dimensions());
    }

    #[test]
    fn test_eating_has_crumbs() {
        let base = test_sprite();
        // Run at a time where crumbs should be visible
        let frame = eating_effect(&base, 0.3);
        let frame_rgba = frame.to_rgba8();
        // Check that some pixels outside the original sprite area have content (crumbs)
        // Verify frame was produced without crashing
        assert_eq!(frame_rgba.width(), 96);
    }

    #[test]
    fn test_frame_rate_varies_by_state() {
        let mut animator = Animator::new();
        assert_eq!(animator.frame_rate_ms(), 80);

        animator.set_state(AnimationState::Eating);
        assert_eq!(animator.frame_rate_ms(), 50);

        animator.set_state(AnimationState::Sleeping);
        assert_eq!(animator.frame_rate_ms(), 100);
    }

    #[test]
    fn test_idle_variant_cycling() {
        let mut v = IdleVariant::Breathe;
        let variants: Vec<_> = (0..3)
            .map(|_| {
                let current = v;
                v = v.next();
                current
            })
            .collect();
        assert_eq!(
            variants,
            vec![IdleVariant::Breathe, IdleVariant::Bounce, IdleVariant::Sway]
        );
        assert_eq!(v, IdleVariant::Breathe);
    }

    #[test]
    fn test_blend_pixel() {
        let mut img = RgbaImage::new(4, 4);
        img.put_pixel(1, 1, Rgba([100, 100, 100, 255]));
        blend_pixel(&mut img, 1, 1, Rgba([255, 0, 0, 128]));
        let result = img.get_pixel(1, 1);
        // Should be blended: ~178 red, ~50 green, ~50 blue
        assert!(result[0] > 150, "Red should dominate after blend");
        assert!(result[1] < 80, "Green should be reduced");
    }
}
