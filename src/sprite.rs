use anyhow::Result;
use std::path::Path;

/// Download a Pokémon sprite from PokeAPI sprites repo.
/// Uses the "official artwork" PNG which is a clean, high-res image.
pub fn download_sprite(pokemon_id: u32, dest: &Path) -> Result<()> {
    // Use official-artwork PNG (475x475, clean)
    let url = format!(
        "https://raw.githubusercontent.com/PokeAPI/sprites/master/sprites/pokemon/other/official-artwork/{}.png",
        pokemon_id
    );

    // Simple blocking HTTP download using std
    let output = std::process::Command::new("curl")
        .args(["-sL", "-o"])
        .arg(dest.as_os_str())
        .arg(&url)
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Verify it's actually an image (check file size)
    let metadata = std::fs::metadata(dest)?;
    if metadata.len() < 100 {
        std::fs::remove_file(dest)?;
        anyhow::bail!("Downloaded file too small, likely failed");
    }

    Ok(())
}

/// Create a simple fallback sprite (a yellow square) when download fails.
pub fn create_fallback_sprite(dest: &Path) -> Result<()> {
    use image::{Rgba, RgbaImage};

    let mut img = RgbaImage::new(96, 96);

    // Draw a simple Pikachu-esque shape (yellow circle with features)
    let yellow = Rgba([255, 220, 50, 255]);
    let black = Rgba([0, 0, 0, 255]);
    let red = Rgba([220, 50, 50, 255]);

    // Body (filled circle)
    for y in 0..96u32 {
        for x in 0..96u32 {
            let dx = x as f32 - 48.0;
            let dy = y as f32 - 52.0;
            if dx * dx + dy * dy < 35.0 * 35.0 {
                img.put_pixel(x, y, yellow);
            }
        }
    }

    // Eyes
    for y in 38..46 {
        for x in 36..42 {
            img.put_pixel(x, y, black);
        }
        for x in 54..60 {
            img.put_pixel(x, y, black);
        }
    }

    // Cheeks
    for y in 48..56 {
        for x in 28..36 {
            let dx = x as f32 - 32.0;
            let dy = y as f32 - 52.0;
            if dx * dx + dy * dy < 4.0 * 4.0 {
                img.put_pixel(x, y, red);
            }
        }
        for x in 60..68 {
            let dx = x as f32 - 64.0;
            let dy = y as f32 - 52.0;
            if dx * dx + dy * dy < 4.0 * 4.0 {
                img.put_pixel(x, y, red);
            }
        }
    }

    // Mouth
    for x in 44..52 {
        img.put_pixel(x, 54, black);
    }

    // Ears (triangles at top)
    for i in 0..15u32 {
        for j in 0..i {
            let x1 = 30 + j;
            let y1 = 20 - i;
            if x1 < 96 && y1 < 96 {
                img.put_pixel(x1, y1, yellow);
            }
            let x2 = 66 - j;
            if x2 < 96 {
                img.put_pixel(x2, y1, yellow);
            }
        }
    }

    img.save(dest)?;
    Ok(())
}
