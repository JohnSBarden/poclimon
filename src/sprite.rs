//! Sprite downloading and caching for PoCLImon.
//!
//! Downloads official artwork PNGs from the PokeAPI sprites repository.
//! Uses `curl` via `std::process::Command` to avoid pulling in a heavy HTTP
//! client dependency (like `reqwest`) for a simple CLI tool.

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Sprite cache that maps pokemon_id -> filesystem path of downloaded sprite.
pub struct SpriteCache {
    cache_dir: PathBuf,
    cached: HashMap<u32, PathBuf>,
}

impl SpriteCache {
    /// Create a new sprite cache rooted at the given directory.
    pub fn new(cache_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Self {
            cache_dir,
            cached: HashMap::new(),
        })
    }

    /// Get the cache directory path.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Get the sprite path for a Pokémon, downloading if necessary.
    /// Returns `Ok(path)` on success, or an error string for UI display.
    pub fn get_or_download(&mut self, pokemon_id: u32, name: &str) -> Result<PathBuf> {
        if let Some(path) = self.cached.get(&pokemon_id) {
            return Ok(path.clone());
        }

        let filename = format!("{}_{}.png", pokemon_id, name.to_lowercase());
        let sprite_path = self.cache_dir.join(&filename);

        if !sprite_path.exists() {
            download_sprite(pokemon_id, &sprite_path)?;
        }

        self.cached.insert(pokemon_id, sprite_path.clone());
        Ok(sprite_path)
    }
}

/// Download a Pokémon sprite from PokeAPI sprites repo.
///
/// Uses the "official artwork" PNG which is a clean, high-res image.
/// We shell out to `curl` rather than pulling in `reqwest` to keep
/// the dependency tree small for this CLI tool.
pub fn download_sprite(pokemon_id: u32, dest: &Path) -> Result<()> {
    let url = format!(
        "https://raw.githubusercontent.com/PokeAPI/sprites/master/sprites/pokemon/other/official-artwork/{}.png",
        pokemon_id
    );

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

    let metadata = std::fs::metadata(dest)?;
    if metadata.len() < 100 {
        std::fs::remove_file(dest)?;
        anyhow::bail!("Downloaded file too small, likely a 404");
    }

    Ok(())
}

/// Create a simple fallback sprite (a colored circle) when download fails.
pub fn create_fallback_sprite(dest: &Path) -> Result<()> {
    use image::{Rgba, RgbaImage};

    let mut img = RgbaImage::new(96, 96);
    let yellow = Rgba([255, 220, 50, 255]);
    let black = Rgba([0, 0, 0, 255]);

    // Body circle
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

    // Mouth
    for x in 44..52 {
        img.put_pixel(x, 54, black);
    }

    img.save(dest)?;
    Ok(())
}
