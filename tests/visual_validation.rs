//! Visual validation tests — verify eye positions and render annotated sprites.
//! Run with: cargo test --test visual_validation -- --nocapture
//!
//! Outputs annotated images to /tmp/poclimon_validation/ for review.

use image::{Rgba, RgbaImage};
use std::path::Path;
use std::process::Command;

fn mark_eye(img: &mut RgbaImage, cx: u32, cy: u32, r: u32) {
    let red = Rgba([255, 0, 0, 220]);
    let (w, h) = (img.width(), img.height());
    // Circle
    for angle in 0..360 {
        let rad = (angle as f64) * std::f64::consts::PI / 180.0;
        let x = cx as f64 + (r + 3) as f64 * rad.cos();
        let y = cy as f64 + (r + 3) as f64 * rad.sin();
        if x >= 0.0 && x < w as f64 && y >= 0.0 && y < h as f64 {
            img.put_pixel(x as u32, y as u32, red);
        }
    }
    // Crosshair
    for d in 0..=(r + 5) {
        for &(dx, dy) in &[(d as i32, 0), (-(d as i32), 0), (0, d as i32), (0, -(d as i32))] {
            let x = cx as i32 + dx;
            let y = cy as i32 + dy;
            if x >= 0 && x < w as i32 && y >= 0 && y < h as i32 {
                img.put_pixel(x as u32, y as u32, red);
            }
        }
    }
}

fn get_sprite(id: u32, name: &str) -> Option<image::DynamicImage> {
    let dir = "/tmp/poclimon_validation/sprites";
    std::fs::create_dir_all(dir).ok()?;
    let path = format!("{}/{}.png", dir, name.to_lowercase());
    if !Path::new(&path).exists() {
        let url = format!(
            "https://raw.githubusercontent.com/PokeAPI/sprites/master/sprites/pokemon/other/official-artwork/{}.png",
            id
        );
        Command::new("curl").args(["-sL", "-o", &path, &url]).output().ok()?;
    }
    image::open(&path).ok()
}

/// Draw grid overlays on sprites for coordinate calibration.
#[test]
fn render_grid_overlays() {
    let out_dir = "/tmp/poclimon_validation/grid";
    std::fs::create_dir_all(out_dir).unwrap();

    let creatures = [(1, "Bulbasaur"), (4, "Charmander"), (7, "Squirtle"), (25, "Pikachu"), (133, "Eevee")];

    for (id, name) in &creatures {
        let img = match get_sprite(*id, name) { Some(i) => i, None => continue };
        let mut out = img.to_rgba8();
        let (w, h) = out.dimensions();
        let green = Rgba([0, 255, 0, 100]);
        let bright = Rgba([0, 255, 0, 180]);

        for i in (0..w).step_by(50) {
            for y in 0..h { out.put_pixel(i, y, if i % 100 == 0 { bright } else { green }); }
        }
        for j in (0..h).step_by(50) {
            for x in 0..w { out.put_pixel(x, j, if j % 100 == 0 { bright } else { green }); }
        }

        out.save(format!("{}/{}_grid.png", out_dir, name.to_lowercase())).unwrap();
    }
}

/// Render annotated sprites with eye markers for visual review.
/// This test always passes — it's for generating review images.
#[test]
fn render_eye_annotations() {
    let out_dir = "/tmp/poclimon_validation/eyes";
    std::fs::create_dir_all(out_dir).unwrap();

    let creatures = [
        (1, "Bulbasaur"),
        (4, "Charmander"),
        (7, "Squirtle"),
        (25, "Pikachu"),
        (133, "Eevee"),
    ];

    for (id, name) in &creatures {
        let img = match get_sprite(*id, name) {
            Some(img) => img,
            None => {
                println!("⚠ SKIP {}: couldn't download sprite", name);
                continue;
            }
        };

        // Use the same eye data the app uses
        let eyes = poclimon::eyes::get_eye_regions(*id);

        println!("{}: {} eyes at {:?}",
            name,
            eyes.len(),
            eyes.iter().map(|e| (e.cx, e.cy)).collect::<Vec<_>>()
        );

        let mut annotated = img.to_rgba8();
        for eye in &eyes {
            mark_eye(&mut annotated, eye.cx, eye.cy, eye.radius);
        }
        let out_path = format!("{}/{}_eyes.png", out_dir, name.to_lowercase());
        annotated.save(&out_path).unwrap();
        println!("  → {}", out_path);
    }
}
