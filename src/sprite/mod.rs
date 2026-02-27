//! Sprite downloading and caching.
//!
//! Downloads sprite sheets and AnimData.xml from the PMDCollab SpriteCollab
//! repository on GitHub. Files are cached locally in ~/.poclimon/sprites/{id}/
//! so we only download once per creature.

pub mod fallback;

use crate::creatures;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// Base URL for raw files in the PMDCollab SpriteCollab repo.
const SPRITECOLLAB_BASE: &str =
    "https://raw.githubusercontent.com/PMDCollab/SpriteCollab/master/sprite";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// The animation names we need for our virtual pet.
const NEEDED_ANIMS: &[&str] = &["Idle", "Sleep", "Eat"];

/// Result type for [`download_all_sprites`]:
/// `(anim_data_path, downloaded_sheets, warning_messages)`.
type SpriteDownloadResult = (PathBuf, Vec<(String, PathBuf)>, Vec<String>);

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

/// Download a file from a URL to a local path.
/// Skips download if the file already exists.
fn download_file(url: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }

    // Write atomically to avoid leaving partial files on interruption.
    let tmp = dest.with_extension("part");
    let connect_timeout = HTTP_CONNECT_TIMEOUT.as_secs().to_string();
    let request_timeout = HTTP_REQUEST_TIMEOUT.as_secs().to_string();
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            &connect_timeout,
            "--max-time",
            &request_timeout,
            "-o",
        ])
        .arg(tmp.as_os_str())
        .arg(url)
        .output()?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("curl failed for {}: {}", url, String::from_utf8_lossy(&output.stderr));
    }

    let metadata = std::fs::metadata(&tmp)?;
    if metadata.len() < 50 {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("Downloaded file too small — likely a 404 or error");
    }
    std::fs::rename(&tmp, dest)?;

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
///
/// Returns `(anim_data_path, sheets, warnings)`.
/// - `sheets`: successfully downloaded `(anim_name, path)` pairs.
/// - `warnings`: non-fatal messages for any animation that couldn't be
///   downloaded (e.g., a missing sheet); the caller displays these via
///   the in-TUI notification system rather than writing to stderr.
pub fn download_all_sprites(creature_id: u32) -> Result<SpriteDownloadResult> {
    let cache_dir = sprite_cache_dir(creature_id)?;
    let pid = creatures::padded_id(creature_id);

    let anim_dest = cache_dir.join("AnimData.xml");
    let anim_url = format!("{}/{}/AnimData.xml", SPRITECOLLAB_BASE, pid);
    let anim_handle = thread::spawn(move || -> Result<PathBuf> {
        download_file(&anim_url, &anim_dest)?;
        Ok(anim_dest)
    });

    let mut sheet_handles = Vec::with_capacity(NEEDED_ANIMS.len());
    for &anim_name in NEEDED_ANIMS {
        let filename = format!("{}-Anim.png", anim_name);
        let dest = cache_dir.join(&filename);
        let url = format!("{}/{}/{}", SPRITECOLLAB_BASE, pid, filename);
        let name = anim_name.to_string();

        sheet_handles.push(thread::spawn(move || -> (String, Result<PathBuf>) {
            let result = download_file(&url, &dest).map(|_| dest);
            (name, result)
        }));
    }

    let anim_data_path = match anim_handle.join() {
        Ok(result) => result?,
        Err(_) => anyhow::bail!("AnimData download worker panicked"),
    };

    let mut sheets = Vec::new();
    let mut warnings = Vec::new();
    for handle in sheet_handles {
        match handle.join() {
            Ok((anim_name, Ok(path))) => sheets.push((anim_name, path)),
            Ok((anim_name, Err(e))) => {
                warnings.push(format!(
                    "Couldn't download {} for creature {}: {}",
                    anim_name, creature_id, e
                ));
            }
            Err(_) => warnings.push(format!(
                "Couldn't download sprite for creature {}: worker thread panicked",
                creature_id
            )),
        }
    }

    Ok((anim_data_path, sheets, warnings))
}
