//! Fallback sprite generation.
//!
//! Creates a simple placeholder sprite when sprite downloads fail.
//! This ensures the app can still run even without network access.

use anyhow::Result;
use image::{DynamicImage, Rgba, RgbaImage};

/// Create a simple fallback sprite (a yellow circle with a "?" mark).
/// Returns a DynamicImage that can be used as a single-frame animation.
pub fn create_fallback_frame() -> Result<DynamicImage> {
    let size = 48u32;
    let mut img = RgbaImage::new(size, size);

    let yellow = Rgba([255, 220, 50, 255]);
    let black = Rgba([30, 30, 30, 255]);

    // Yellow circle body
    let center = size as f32 / 2.0;
    let radius = (size as f32 / 2.0) - 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            if dx * dx + dy * dy < radius * radius {
                img.put_pixel(x, y, yellow);
            }
        }
    }

    // Simple "?" in the center
    // Top curve of ?
    for x in 18..30 {
        img.put_pixel(x, 12, black);
        img.put_pixel(x, 13, black);
    }
    for y in 14..22 {
        img.put_pixel(28, y, black);
        img.put_pixel(29, y, black);
    }
    for x in 22..30 {
        img.put_pixel(x, 22, black);
        img.put_pixel(x, 23, black);
    }
    // Stem
    for y in 24..30 {
        img.put_pixel(22, y, black);
        img.put_pixel(23, y, black);
    }
    // Dot
    for y in 32..35 {
        for x in 22..25 {
            img.put_pixel(x, y, black);
        }
    }

    Ok(DynamicImage::ImageRgba8(img))
}
