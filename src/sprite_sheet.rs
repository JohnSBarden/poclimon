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
use image::{DynamicImage, GenericImageView};

/// Extract individual animation frames from a sprite sheet.
///
/// The sprite sheet layout:
/// - Each row is a direction (row 0 = Down, facing the viewer)
/// - Each column is a frame
/// - Frame size is defined by `anim_info.frame_width` x `anim_info.frame_height`
///
/// Returns a `Vec` of cropped frame images, one per frame in the animation.
pub fn extract_frames(sheet: &DynamicImage, anim_info: &AnimInfo) -> Vec<DynamicImage> {
    let frame_count = anim_info.frame_count();
    let fw = anim_info.frame_width;
    let fh = anim_info.frame_height;
    let (sheet_w, sheet_h) = sheet.dimensions();

    let mut frames = Vec::with_capacity(frame_count);

    for col in 0..frame_count {
        let x = col as u32 * fw;
        let y = 0; // Row 0 = Down direction

        // Make sure we don't go out of bounds
        if x + fw > sheet_w || y + fh > sheet_h {
            break;
        }

        let frame = sheet.crop_imm(x, y, fw, fh);
        frames.push(frame);
    }

    frames
}

/// Normalize a set of frames to the given target dimensions using
/// nearest-neighbor scaling.
///
/// This ensures that animations with different native frame sizes (e.g., Idle
/// vs. Sleep) all render at the same size in the TUI, preventing layout jumps
/// when the user changes animation state.
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
            if frame.width() == target_w && frame.height() == target_h {
                frame
            } else {
                DynamicImage::ImageRgba8(image::imageops::resize(
                    &frame,
                    target_w,
                    target_h,
                    image::imageops::FilterType::Nearest,
                ))
            }
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
            Rgba([255, 0, 0, 255]),   // Frame 0: red
            Rgba([0, 255, 0, 255]),   // Frame 1: green
            Rgba([0, 0, 255, 255]),   // Frame 2: blue
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
        let frames = extract_frames(&sheet, &info);
        assert_eq!(frames.len(), 3);
    }

    #[test]
    fn test_extract_correct_frame_dimensions() {
        let sheet = make_test_sheet();
        let info = make_test_anim_info();
        let frames = extract_frames(&sheet, &info);
        for frame in &frames {
            assert_eq!(frame.dimensions(), (10, 10));
        }
    }

    #[test]
    fn test_extract_correct_frame_colors() {
        let sheet = make_test_sheet();
        let info = make_test_anim_info();
        let frames = extract_frames(&sheet, &info);

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
        let frames = extract_frames(&sheet, &info);
        // Should only extract 3 frames (stops at sheet boundary)
        assert_eq!(frames.len(), 3);
    }

    #[test]
    fn test_normalize_frames_resizes_to_target() {
        // Create a 4x4 frame
        let small = DynamicImage::ImageRgba8(RgbaImage::new(4, 4));
        let frames = vec![small];
        let normalized = normalize_frames(frames, 8, 8);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].dimensions(), (8, 8));
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
