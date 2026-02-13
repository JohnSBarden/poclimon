pub mod fallback;

use anyhow::Result;
use std::path::Path;

/// Download a creature sprite from PokeAPI sprites repo.
/// Uses the "official artwork" PNG which is a clean, high-res image.
pub fn download_sprite(creature_id: u32, dest: &Path) -> Result<()> {
    let url = format!(
        "https://raw.githubusercontent.com/PokeAPI/sprites/master/sprites/pokemon/other/official-artwork/{}.png",
        creature_id
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
        anyhow::bail!("Downloaded file too small, likely failed");
    }

    Ok(())
}
