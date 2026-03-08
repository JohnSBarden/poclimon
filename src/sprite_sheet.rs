//! Sprite sheet frame extraction and normalization.
//!
//! PMDCollab sprite sheets have 8 rows (one per direction) and N columns
//! (one per frame). We only need row 0 (facing down/toward the viewer)
//! for our virtual pet display.
//!
//! Because different animations (Idle, Sleep, Eat) may have different frame
//! dimensions in AnimData.xml, we normalize all frames to a canonical size
//! (the Idle animation's dimensions) after extraction to avoid layout jumps
//! when switching states.

use crate::anim_data::AnimInfo;
use image::{DynamicImage, GenericImage, GenericImageView, RgbaImage};

/// Extract individual animation frames from a sprite sheet.
///
/// The sprite sheet layout:
/// - Each row is a direction (row 0 = Down, row 2 = Left, row 4 = Up, row 6 = Right)
/// - Each column is a frame
/// - Frame size is defined by `anim_info.frame_width` x `anim_info.frame_height`
///
/// `dir_row` selects which row of the sheet to extract from.
/// If the row is out of bounds (sheet lacks that direction), returns an empty Vec.
/// The caller is responsible for fallback handling.
///
/// Returns a `Vec` of cropped frame images, one per frame in the animation.
pub fn extract_frames(
    sheet: &DynamicImage,
    anim_info: &AnimInfo,
    dir_row: u32,
) -> Vec<DynamicImage> {
    let frame_count = anim_info.frame_count();
    let fw = anim_info.frame_width;
    let fh = anim_info.frame_height;
    let (sheet_w, sheet_h) = sheet.dimensions();

    let y = dir_row * fh;

    // If requested row is out of bounds, return empty — caller handles fallback.
    if y + fh > sheet_h {
        return Vec::new();
    }

    let mut frames = Vec::with_capacity(frame_count);

    for col in 0..frame_count {
        let x = col as u32 * fw;

        if x + fw > sheet_w {
            break;
        }

        let frame = sheet.crop_imm(x, y, fw, fh);
        frames.push(frame);
    }

    frames
}

/// Normalize frames to the target dimensions by centering on a transparent canvas.
/// Does NOT scale the art — just pads. Preserves pixel-perfect fidelity.
///
/// This ensures that animations with different native frame sizes (e.g., Idle
/// vs. Sleep) all render at the same size in the TUI, preventing layout jumps
/// when the user changes animation state. Smaller frames are centered; larger
/// frames are cropped at the center.
///
/// If a frame is already the target size it is returned as-is without copying.
pub fn normalize_frames(
    frames: Vec<DynamicImage>,
    target_w: u32,
    target_h: u32,
) -> Vec<DynamicImage> {
    if target_w == 0 || target_h == 0 {
        return frames;
    }

    frames
        .into_iter()
        .map(|frame| {
            let fw = frame.width();
            let fh = frame.height();
            if fw == target_w && fh == target_h {
                return frame;
            }
            let mut canvas = DynamicImage::ImageRgba8(RgbaImage::new(target_w, target_h));
            let dst_x = (target_w as i32 - fw as i32).max(0) as u32 / 2;
            let dst_y = (target_h as i32 - fh as i32).max(0) as u32 / 2;
            let src_x = (fw as i32 - target_w as i32).max(0) as u32 / 2;
            let src_y = (fh as i32 - target_h as i32).max(0) as u32 / 2;
            let copy_w = fw.min(target_w);
            let copy_h = fh.min(target_h);
            let cropped = frame.crop_imm(src_x, src_y, copy_w, copy_h);
            let _ = canvas.copy_from(&cropped, dst_x, dst_y);
            canvas
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    /// Create a test sprite sheet: 3 frames wide, 8 rows tall.
    /// Each frame is 10x10 pixels. Each frame in row 0 gets a different color.
    fn make_test_sheet() -> DynamicImage {
        let fw = 10u32;
        let fh = 10u32;
        let cols = 3u32;
        let rows = 8u32;

        let mut img = RgbaImage::new(fw * cols, fh * rows);

        // Color row 0 frames distinctly
        let colors = [
            Rgba([255, 0, 0, 255]), // Frame 0: red
            Rgba([0, 255, 0, 255]), // Frame 1: green
            Rgba([0, 0, 255, 255]), // Frame 2: blue
        ];

        for col in 0..cols {
            for y in 0..fh {
                for x in 0..fw {
                    img.put_pixel(col * fw + x, y, colors[col as usize]);
                }
            }
        }

        DynamicImage::ImageRgba8(img)
    }

    fn make_test_anim_info() -> AnimInfo {
        AnimInfo {
            frame_width: 10,
            frame_height: 10,
            durations: vec![5, 5, 5],
        }
    }

    #[test]
    fn test_extract_correct_frame_count() {
        let sheet = make_test_sheet();
        let info = make_test_anim_info();
        let frames = extract_frames(&sheet, &info, 0);
        assert_eq!(frames.len(), 3);
    }

    #[test]
    fn test_extract_correct_frame_dimensions() {
        let sheet = make_test_sheet();
        let info = make_test_anim_info();
        let frames = extract_frames(&sheet, &info, 0);
        for frame in &frames {
            assert_eq!(frame.dimensions(), (10, 10));
        }
    }

    #[test]
    fn test_extract_correct_frame_colors() {
        let sheet = make_test_sheet();
        let info = make_test_anim_info();
        let frames = extract_frames(&sheet, &info, 0);

        // Frame 0 should be red
        assert_eq!(frames[0].get_pixel(5, 5), Rgba([255, 0, 0, 255]));
        // Frame 1 should be green
        assert_eq!(frames[1].get_pixel(5, 5), Rgba([0, 255, 0, 255]));
        // Frame 2 should be blue
        assert_eq!(frames[2].get_pixel(5, 5), Rgba([0, 0, 255, 255]));
    }

    #[test]
    fn test_extract_handles_oversized_frame_count() {
        let sheet = make_test_sheet();
        // Claim 5 frames but sheet only has 3 columns
        let info = AnimInfo {
            frame_width: 10,
            frame_height: 10,
            durations: vec![5, 5, 5, 5, 5],
        };
        let frames = extract_frames(&sheet, &info, 0);
        // Should only extract 3 frames (stops at sheet boundary)
        assert_eq!(frames.len(), 3);
    }

    #[test]
    fn test_extract_out_of_bounds_row_returns_empty() {
        let sheet = make_test_sheet(); // 8 rows (fh=10, total height=80)
        let info = make_test_anim_info();
        // Row 8 would start at y=80 which equals sheet_h=80 — out of bounds
        let frames = extract_frames(&sheet, &info, 8);
        assert!(frames.is_empty());
    }

    #[test]
    fn test_extract_last_valid_row() {
        let sheet = make_test_sheet(); // 8 rows
        let info = make_test_anim_info();
        // Row 7 starts at y=70, ends at y=80 — exactly in bounds
        let frames = extract_frames(&sheet, &info, 7);
        assert_eq!(frames.len(), 3);
    }

    #[test]
    fn test_normalize_frames_resizes_to_target() {
        // A 4x4 frame normalized (padded) to 8x8 should:
        // - produce an 8x8 image
        // - the original pixels should be centered (at offset 2,2)
        let color = Rgba([255u8, 0, 0, 255]);
        let mut small_img = RgbaImage::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                small_img.put_pixel(x, y, color);
            }
        }
        let small = DynamicImage::ImageRgba8(small_img);
        let frames = vec![small];
        let normalized = normalize_frames(frames, 8, 8);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].dimensions(), (8, 8));
        // Centering: dst_x = (8 - 4) / 2 = 2, dst_y = (8 - 4) / 2 = 2
        // The original red pixel at (0,0) maps to (2,2) in the canvas
        assert_eq!(normalized[0].get_pixel(2, 2), color);
        // Corners of canvas should be transparent
        assert_eq!(normalized[0].get_pixel(0, 0), Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn test_normalize_frames_passthrough_when_already_correct() {
        let frame = DynamicImage::ImageRgba8(RgbaImage::new(10, 10));
        let frames = vec![frame];
        let normalized = normalize_frames(frames, 10, 10);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].dimensions(), (10, 10));
    }

    #[test]
    fn test_normalize_frames_empty_vec() {
        let normalized = normalize_frames(vec![], 10, 10);
        assert!(normalized.is_empty());
    }

    #[test]
    fn test_normalize_frames_mixed_sizes() {
        let frames = vec![
            DynamicImage::ImageRgba8(RgbaImage::new(4, 4)),
            DynamicImage::ImageRgba8(RgbaImage::new(8, 12)),
            DynamicImage::ImageRgba8(RgbaImage::new(40, 56)),
        ];
        let normalized = normalize_frames(frames, 40, 56);
        for f in &normalized {
            assert_eq!(f.dimensions(), (40, 56));
        }
    }
}
