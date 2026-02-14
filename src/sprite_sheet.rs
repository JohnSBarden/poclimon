//! Sprite sheet frame extraction.
//!
//! PMDCollab sprite sheets have 8 rows (one per direction) and N columns
//! (one per frame). We only need row 0 (facing down/toward the viewer)
//! for our virtual pet display.

use crate::anim_data::AnimInfo;
use image::{DynamicImage, GenericImageView};

/// Extract individual animation frames from a sprite sheet.
///
/// The sprite sheet layout:
/// - Each row is a direction (row 0 = Down, facing the viewer)
/// - Each column is a frame
/// - Frame size is defined by `anim_info.frame_width` x `anim_info.frame_height`
///
/// Returns a Vec of cropped frame images, one per frame in the animation.
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
            name: "Test".to_string(),
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
            name: "Test".to_string(),
            frame_width: 10,
            frame_height: 10,
            durations: vec![5, 5, 5, 5, 5],
        };
        let frames = extract_frames(&sheet, &info);
        // Should only extract 3 frames (stops at sheet boundary)
        assert_eq!(frames.len(), 3);
    }
}
