//! Sprite downloading and caching.
//!
//! Downloads sprite sheets and AnimData.xml from the PMDCollab SpriteCollab
//! repository on GitHub. Files are cached locally in ~/.poclimon/sprites/{id}/
//! so we only download once per creature.

pub mod fallback;

use crate::creatures;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Base URL for raw files in the PMDCollab SpriteCollab repo.
const SPRITECOLLAB_BASE: &str =
    "https://raw.githubusercontent.com/PMDCollab/SpriteCollab/master/sprite";

/// The animation names we need for our virtual pet.
const NEEDED_ANIMS: &[&str] = &["Idle", "Sleep", "Eat"];

/// Get the cache directory for a creature's sprites.
/// Creates the directory if it doesn't exist.
pub fn sprite_cache_dir(creature_id: u32) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = home
        .join(".poclimon")
        .join("sprites")
        .join(creatures::padded_id(creature_id));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Download a file from a URL to a local path using curl.
/// Skips download if the file already exists.
fn download_file(url: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }

    let output = std::process::Command::new("curl")
        .args(["-sL", "-o"])
        .arg(dest.as_os_str())
        .arg(url)
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Verify the file isn't empty/tiny (likely a 404 page)
    let metadata = std::fs::metadata(dest)?;
    if metadata.len() < 50 {
        std::fs::remove_file(dest)?;
        anyhow::bail!("Downloaded file too small — likely a 404 or error");
    }

    Ok(())
}

/// Download the AnimData.xml for a creature.
/// Returns the path to the cached file.
pub fn download_anim_data(creature_id: u32) -> Result<PathBuf> {
    let cache_dir = sprite_cache_dir(creature_id)?;
    let dest = cache_dir.join("AnimData.xml");

    let pid = creatures::padded_id(creature_id);
    let url = format!("{}/{}/AnimData.xml", SPRITECOLLAB_BASE, pid);

    download_file(&url, &dest)?;
    Ok(dest)
}

/// Download a sprite sheet for a specific animation.
/// Returns the path to the cached PNG file.
pub fn download_sprite_sheet(creature_id: u32, anim_name: &str) -> Result<PathBuf> {
    let cache_dir = sprite_cache_dir(creature_id)?;
    let filename = format!("{}-Anim.png", anim_name);
    let dest = cache_dir.join(&filename);

    let pid = creatures::padded_id(creature_id);
    let url = format!("{}/{}/{}", SPRITECOLLAB_BASE, pid, filename);

    download_file(&url, &dest)?;
    Ok(dest)
}

/// Download all needed sprites for a creature (AnimData.xml + sprite sheets).
/// Returns (anim_data_path, Vec<(anim_name, sheet_path)>).
pub fn download_all_sprites(
    creature_id: u32,
) -> Result<(PathBuf, Vec<(String, PathBuf)>)> {
    let anim_data_path = download_anim_data(creature_id)?;

    let mut sheets = Vec::new();
    for &anim_name in NEEDED_ANIMS {
        match download_sprite_sheet(creature_id, anim_name) {
            Ok(path) => sheets.push((anim_name.to_string(), path)),
            Err(e) => {
                eprintln!(
                    "Warning: couldn't download {} for creature {}: {}",
                    anim_name, creature_id, e
                );
            }
        }
    }

    Ok((anim_data_path, sheets))
}
