use crate::anim_data::AnimInfo;
use crate::animation::Animation;
use crate::creature::CreatureSlot;
use crate::sprite::{self};
use crate::sprite_sheet;
use anyhow::Result;
use image::imageops::FilterType;
use ratatui_image::{Resize, picker::Picker, protocol::Protocol};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use crate::creature::MAX_CACHED_FRAMES;

pub enum SwapWorkerResult {
    Loaded {
        slot: Box<CreatureSlot>,
        warnings: Vec<String>,
    },
    Failed(String),
}

pub struct SwapTransition {
    pub slot_index: usize,
    pub recall_ticks: u8,
    pub target_name: String,
    pub worker_rx: Receiver<SwapWorkerResult>,
    pub worker_result: Option<SwapWorkerResult>,
}

pub struct AddTransition {
    pub target_name: String,
    pub worker_rx: Receiver<SwapWorkerResult>,
    pub worker_result: Option<SwapWorkerResult>,
}

/// Cap a frame list to at most `MAX_CACHED_FRAMES`, selecting evenly-spaced
/// frames so the animation remains representative.
///
/// Also truncates `durations` to match `frames` in case they differ (defensive).
pub fn cap_frames(
    frames: Vec<image::DynamicImage>,
    durations: Vec<u32>,
) -> (Vec<image::DynamicImage>, Vec<u32>) {
    // Align lengths defensively.
    let n = frames.len().min(durations.len());
    let mut frames = frames;
    let mut durations = durations;
    frames.truncate(n);
    durations.truncate(n);

    if n <= MAX_CACHED_FRAMES {
        return (frames, durations);
    }

    // Pick MAX_CACHED_FRAMES evenly-spaced indices.
    let cap = MAX_CACHED_FRAMES;
    let indices: Vec<usize> = (0..cap).map(|i| i * n / cap).collect();
    let capped_frames: Vec<image::DynamicImage> =
        indices.iter().map(|&i| frames[i].clone()).collect();
    let capped_durations: Vec<u32> = indices.iter().map(|&i| durations[i]).collect();
    (capped_frames, capped_durations)
}

/// Download, parse, and cache all animation frames for a single slot.
///
/// Frames are pre-scaled by `scale` and normalized to the Idle animation's
/// canonical dimensions so the render loop never has to resize.
/// Frames live only in `slot.cached_*`; the `Animator` holds timing only.
///
/// Returns a Vec of non-fatal warning strings (e.g., a missing animation
/// sheet that was replaced with a fallback). These are shown as in-TUI
/// notifications rather than written to stderr.
///
/// Creatures missing an Eat or Sleep animation (e.g. Articuno, Zapdos, Moltres,
/// Vaporeon) silently fall back to their Idle frames — no yellow "?" placeholder,
/// no size change on state switch, no warning noise.
pub fn load_slot_sprites(slot: &mut CreatureSlot, scale: u32) -> Result<Vec<String>> {
    let (anim_data_path, sheets, warnings) = sprite::download_all_sprites(slot.creature_id)?;

    let xml = std::fs::read_to_string(&anim_data_path)?;
    let anim_infos = crate::anim_data::parse_anim_data(&xml);

    // PMDCollab direction row indices: 0=Down, 2=Left, 4=Up, 6=Right
    // Our dir_idx mapping:             0=Down, 1=Left, 2=Up, 3=Right
    const DIR_ROWS: [u32; 4] = [0, 2, 4, 6];

    // Load Idle for all 4 directions — use dir 0 (Down) to establish canonical size.
    let (idle_down, idle_timing, idle_w, idle_h, _) =
        load_and_scale_animation("Idle", &sheets, &anim_infos, scale, None, DIR_ROWS[0])?;
    let idle_left = load_and_scale_animation(
        "Idle",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[1],
    )
    .map(|r| r.0)
    .unwrap_or_else(|_| idle_down.clone());
    let idle_up = load_and_scale_animation(
        "Idle",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[2],
    )
    .map(|r| r.0)
    .unwrap_or_else(|_| idle_down.clone());
    let idle_right = load_and_scale_animation(
        "Idle",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[3],
    )
    .map(|r| r.0)
    .unwrap_or_else(|_| idle_down.clone());
    slot.cached_idle = [idle_down.clone(), idle_left, idle_up, idle_right];

    // Try Eat dir 0 first to get fallback status.
    let (eat_down_raw, eat_timing_raw, _, _, eat_fallback) = load_and_scale_animation(
        "Eat",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[0],
    )?;
    let (eat_frames_by_dir, eat_timing) = if eat_fallback {
        // Reuse Idle frames for all 4 directions
        (slot.cached_idle.clone(), idle_timing.clone())
    } else {
        let eat_left = load_and_scale_animation(
            "Eat",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[1],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| eat_down_raw.clone());
        let eat_up = load_and_scale_animation(
            "Eat",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[2],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| eat_down_raw.clone());
        let eat_right = load_and_scale_animation(
            "Eat",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[3],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| eat_down_raw.clone());
        ([eat_down_raw, eat_left, eat_up, eat_right], eat_timing_raw)
    };
    slot.cached_eat = eat_frames_by_dir;

    // Try Sleep dir 0 first to get fallback status.
    let (sleep_down_raw, sleep_timing_raw, _, _, sleep_fallback) = load_and_scale_animation(
        "Sleep",
        &sheets,
        &anim_infos,
        scale,
        Some((idle_w, idle_h)),
        DIR_ROWS[0],
    )?;
    let (sleep_frames_by_dir, sleep_timing) = if sleep_fallback {
        // Reuse Idle frames for all 4 directions
        (slot.cached_idle.clone(), idle_timing.clone())
    } else {
        let sleep_left = load_and_scale_animation(
            "Sleep",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[1],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| sleep_down_raw.clone());
        let sleep_up = load_and_scale_animation(
            "Sleep",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[2],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| sleep_down_raw.clone());
        let sleep_right = load_and_scale_animation(
            "Sleep",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[3],
        )
        .map(|r| r.0)
        .unwrap_or_else(|_| sleep_down_raw.clone());
        (
            [sleep_down_raw, sleep_left, sleep_up, sleep_right],
            sleep_timing_raw,
        )
    };
    slot.cached_sleep = sleep_frames_by_dir;

    // Recall animation for swap transitions:
    // prefer Spin -> fallback Rotate -> fallback Idle.
    let (recall_frames_by_dir, _recall_name) = {
        let (spin_down, _, _, _, spin_fallback) = load_and_scale_animation(
            "Spin",
            &sheets,
            &anim_infos,
            scale,
            Some((idle_w, idle_h)),
            DIR_ROWS[0],
        )?;
        if !spin_fallback {
            let spin_left = load_and_scale_animation(
                "Spin",
                &sheets,
                &anim_infos,
                scale,
                Some((idle_w, idle_h)),
                DIR_ROWS[1],
            )
            .map(|r| r.0)
            .unwrap_or_else(|_| spin_down.clone());
            let spin_up = load_and_scale_animation(
                "Spin",
                &sheets,
                &anim_infos,
                scale,
                Some((idle_w, idle_h)),
                DIR_ROWS[2],
            )
            .map(|r| r.0)
            .unwrap_or_else(|_| spin_down.clone());
            let spin_right = load_and_scale_animation(
                "Spin",
                &sheets,
                &anim_infos,
                scale,
                Some((idle_w, idle_h)),
                DIR_ROWS[3],
            )
            .map(|r| r.0)
            .unwrap_or_else(|_| spin_down.clone());
            ([spin_down, spin_left, spin_up, spin_right], "Spin")
        } else {
            let (rotate_down, _rotate_anim, _rotate_w, _rotate_h, rotate_fallback) =
                load_and_scale_animation(
                    "Rotate",
                    &sheets,
                    &anim_infos,
                    scale,
                    Some((idle_w, idle_h)),
                    DIR_ROWS[0],
                )?;
            if !rotate_fallback {
                let rotate_left = load_and_scale_animation(
                    "Rotate",
                    &sheets,
                    &anim_infos,
                    scale,
                    Some((idle_w, idle_h)),
                    DIR_ROWS[1],
                )
                .map(|r| r.0)
                .unwrap_or_else(|_| rotate_down.clone());
                let rotate_up = load_and_scale_animation(
                    "Rotate",
                    &sheets,
                    &anim_infos,
                    scale,
                    Some((idle_w, idle_h)),
                    DIR_ROWS[2],
                )
                .map(|r| r.0)
                .unwrap_or_else(|_| rotate_down.clone());
                let rotate_right = load_and_scale_animation(
                    "Rotate",
                    &sheets,
                    &anim_infos,
                    scale,
                    Some((idle_w, idle_h)),
                    DIR_ROWS[3],
                )
                .map(|r| r.0)
                .unwrap_or_else(|_| rotate_down.clone());
                (
                    [rotate_down, rotate_left, rotate_up, rotate_right],
                    "Rotate",
                )
            } else {
                (slot.cached_idle.clone(), "Idle")
            }
        }
    };
    slot.cached_recall = recall_frames_by_dir;

    // Give the animator timing-only Animation objects (no pixel data).
    slot.animator = crate::animation::Animator::new();
    slot.animator
        .load_animations(idle_timing, eat_timing, sleep_timing);

    // Invalidate encoded frames so that first render re-encodes for the actual Rect.
    slot.encoded_rect = None;
    slot.encoded_frames = std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));

    // Filter out warnings for animations we gracefully handled via Idle fallback
    // or optional recall animation fallbacks (Spin/Rotate).
    let filtered_warnings = if eat_fallback || sleep_fallback {
        warnings
            .into_iter()
            .filter(|w| {
                let w_lower = w.to_lowercase();
                // Keep warnings that aren't about the animations we handled
                !(eat_fallback && w_lower.contains("eat")
                    || sleep_fallback && w_lower.contains("sleep")
                    || w_lower.contains("spin")
                    || w_lower.contains("rotate"))
            })
            .collect()
    } else {
        warnings
            .into_iter()
            .filter(|w| {
                let w_lower = w.to_lowercase();
                !(w_lower.contains("spin") || w_lower.contains("rotate"))
            })
            .collect()
    };

    Ok(filtered_warnings)
}

/// Load an animation, pre-scale its frames by `scale`, cap to
/// `MAX_CACHED_FRAMES`, then normalize to `canonical_size` (if provided).
///
/// `dir_row` selects which PMDCollab direction row to extract (0=Down, 2=Left,
/// 4=Up, 6=Right). If the sheet doesn't have that row, falls back to row 0.
///
/// Returns `(frames, timing_animation, frame_width, frame_height, is_fallback)`.
/// `is_fallback` is `true` when animation was missing and a fallback frame
/// was used — callers can substitute Idle frames to avoid a broken placeholder.
/// The returned `Animation` is timing-only — no pixel data.
pub fn load_and_scale_animation(
    anim_name: &str,
    sheets: &[(String, PathBuf)],
    anim_infos: &HashMap<String, AnimInfo>,
    scale: u32,
    canonical_size: Option<(u32, u32)>,
    dir_row: u32,
) -> Result<(Vec<image::DynamicImage>, Animation, u32, u32, bool)> {
    let sheet_path = sheets
        .iter()
        .find(|(name, _)| name == anim_name)
        .map(|(_, path)| path);

    let anim_info = anim_infos.get(anim_name);

    let (raw_frames, raw_durations, is_fallback) = match (sheet_path, anim_info) {
        (Some(path), Some(info)) => {
            let sheet = image::ImageReader::open(path)?.decode()?;
            // Try requested direction row; fall back to row 0 if out of bounds.
            let mut frames = sprite_sheet::extract_frames(&sheet, info, dir_row);
            if frames.is_empty() && dir_row != 0 {
                frames = sprite_sheet::extract_frames(&sheet, info, 0);
            }
            if frames.is_empty() {
                let fallback = sprite::fallback::create_fallback_frame()?;
                (vec![fallback], vec![20u32], true)
            } else {
                let durations = info.durations.clone();
                (frames, durations, false)
            }
        }
        _ => {
            let fallback = sprite::fallback::create_fallback_frame()?;
            (vec![fallback], vec![20u32], true)
        }
    };

    // Step 1: scale by the display scale factor (Nearest-neighbor, RGBA8).
    let scaled: Vec<image::DynamicImage> = raw_frames
        .into_iter()
        .map(|f| {
            let (w, h) = (f.width() * scale, f.height() * scale);
            image::DynamicImage::ImageRgba8(image::imageops::resize(
                &f,
                w,
                h,
                image::imageops::FilterType::Nearest,
            ))
        })
        .collect();

    // Step 2: cap to MAX_CACHED_FRAMES (evenly-spaced sampling if needed).
    let (capped_frames, capped_durations) = cap_frames(scaled, raw_durations);

    // Record dimensions after scaling (before optional normalization).
    let scaled_w = capped_frames.first().map(|f| f.width()).unwrap_or(0);
    let scaled_h = capped_frames.first().map(|f| f.height()).unwrap_or(0);

    // Step 3: normalize to canonical size if provided.
    let final_frames = match canonical_size {
        Some((cw, ch)) => sprite_sheet::normalize_frames(capped_frames, cw, ch),
        None => capped_frames,
    };

    let out_w = final_frames.first().map(|f| f.width()).unwrap_or(scaled_w);
    let out_h = final_frames.first().map(|f| f.height()).unwrap_or(scaled_h);

    // Build a timing-only Animation aligned to final frame count.
    let timing = Animation::new(final_frames.len(), &capped_durations);

    Ok((final_frames, timing, out_w, out_h, is_fallback))
}

// ── Protocol encoding ──────────────────────────────────────────────────────────

/// Pre-encode all animation frames for a slot into non-stateful `Protocol`
/// objects sized for `area`.
///
/// Called lazily from `render_pen` whenever the render `Rect` changes
/// (terminal resize) or on the first render.  After this call, rendering
/// a frame is a cheap table lookup — no DynamicImage copies, no alloc/free
/// churn.
///
/// Encodes all 4 directions (Down/Left/Up/Right) for each state
/// (Idle/Eat/Sleep/Recall), giving `encoded_frames[state][dir][frame]`.
///
/// Memory: each `Protocol::Halfblocks` stores only `Vec<HalfBlock>` + a
/// `Rect`, no source image. 8 frames × 4 dirs × 4 states × 6 slots is bounded
/// and cached per-slot.
pub fn encode_all_frames(slot: &mut CreatureSlot, picker: &Picker, area: ratatui::layout::Rect) {
    // Clone caches to avoid simultaneous shared+mutable borrows of `slot`.
    let idle = slot.cached_idle.clone();
    let eat = slot.cached_eat.clone();
    let sleep = slot.cached_sleep.clone();
    let recall = slot.cached_recall.clone();

    let caches: [&[Vec<image::DynamicImage>; 4]; 4] = [&idle, &eat, &sleep, &recall];
    let is_halfblocks = picker.protocol_type() == ratatui_image::picker::ProtocolType::Halfblocks;

    slot.encoded_frames = std::array::from_fn(|state_idx| {
        let cache = caches[state_idx];
        std::array::from_fn(|dir_idx| {
            cache[dir_idx]
                .iter()
                .map(|img| {
                    if is_halfblocks {
                        encode_halfblock_frame(img, area)
                    } else {
                        picker
                            .new_protocol(
                                img.clone(),
                                area,
                                Resize::Scale(Some(FilterType::Nearest)),
                            )
                            .ok()
                    }
                })
                .collect()
        })
    });
    slot.encoded_rect = Some(area);
}

/// Encode a single frame for halfblock terminals, bypassing the picker's
/// padded-image pipeline which squishes sprite into only part of the canvas.
///
/// The picker pads the resized sprite into a full pixel area (e.g. 256×128),
/// then Halfblocks::new's resize_exact compresses that padded image — the sprite
/// ends up occupying only the left portion of the halfblock canvas, unrecognizable.
///
/// Instead: pre-resize to the exact halfblock pixel dimensions (area.width ×
/// area.height×2) with Lanczos3, center it on a transparent canvas, and hand
/// it straight to Halfblocks::new whose resize_exact becomes a no-op.
pub fn encode_halfblock_frame(
    img: &image::DynamicImage,
    area: ratatui::layout::Rect,
) -> Option<Protocol> {
    let pw = area.width as u32;
    let ph = area.height as u32 * 2; // halfblock: 2 pixel-rows per terminal row
    // Lanczos3 gives sharp edges — important for small pixel counts.
    let mut canvas = image::DynamicImage::ImageRgba8(image::RgbaImage::new(pw, ph));
    image::imageops::overlay(&mut canvas, img, 0, 0);
    // Center the sprite on the canvas.
    let ox = (pw.saturating_sub(img.width())) / 2;
    let oy = (ph.saturating_sub(img.height())) / 2;
    let mut centered = image::DynamicImage::ImageRgba8(image::RgbaImage::new(pw, ph));
    image::imageops::overlay(&mut centered, &canvas, ox as i64, oy as i64);
    ratatui_image::protocol::halfblocks::Halfblocks::new(centered, area)
        .ok()
        .map(Protocol::Halfblocks)
}
