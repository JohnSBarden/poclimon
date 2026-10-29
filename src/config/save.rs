//! Signed progress save (`~/.config/poclimon/save.toml`).
//!
//! Progress (roster, levels, XP, unlocked species) lives apart from the
//! freely editable settings file and carries an HMAC-SHA256 signature, so
//! editing it by hand is detected. This is tamper-evidence, not DRM: anyone
//! who rebuilds the binary can bypass it (see issue #28 for the threat model).
//!
//! The signature covers a canonical serialization of the data rather than the
//! file text, so whitespace and comment edits don't invalidate a save.

use super::{ConfigError, SlotEntry, write_atomic};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs;
use std::path::{Path, PathBuf};

pub const SAVE_VERSION: u32 = 1;

/// Key used when the build has no `POCLIMON_SAVE_KEY`. It's public, so dev
/// saves offer no protection; they're kept in a separate file and release
/// builds never accept them.
const DEV_KEY: &[u8] = b"poclimon-dev-key-not-a-secret!!!";

/// Release key as 64 hex chars, injected at compile time from a CI secret.
/// `build.rs` rejects malformed values.
const RELEASE_KEY_HEX: Option<&str> = option_env!("POCLIMON_SAVE_KEY");

/// Which key this build signs with.
struct SigningKey {
    id: &'static str,
    bytes: Vec<u8>,
}

fn signing_key() -> SigningKey {
    match RELEASE_KEY_HEX {
        Some(hex) => SigningKey {
            id: "r1",
            bytes: decode_hex(hex).expect("build.rs validates POCLIMON_SAVE_KEY"),
        },
        None => SigningKey {
            id: "dev",
            bytes: DEV_KEY.to_vec(),
        },
    }
}

/// Whether this build signs with the public dev key.
pub fn is_dev_build() -> bool {
    RELEASE_KEY_HEX.is_none()
}

/// The save file for a settings file: `<dir>/poclimon/save.toml` next to it
/// (`save.dev.toml` in dev builds, so development never touches a real save).
pub fn save_path_for(settings_path: &Path) -> PathBuf {
    let dir = match settings_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let name = if is_dev_build() {
        "save.dev.toml"
    } else {
        "save.toml"
    };
    dir.join("poclimon").join(name)
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Progress {
    /// Species the player may add or swap in beyond the base forms (#27).
    #[serde(default)]
    pub unlocked: Vec<u32>,
}

/// Progress as stored on disk.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SaveData {
    pub version: u32,
    pub key_id: String,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub progress: Progress,
    #[serde(default)]
    pub slot: Vec<SlotEntry>,
}

impl SaveData {
    pub fn new(slots: Vec<SlotEntry>, unlocked: Vec<u32>) -> Self {
        let mut data = Self {
            version: SAVE_VERSION,
            key_id: signing_key().id.to_string(),
            signature: String::new(),
            progress: Progress { unlocked },
            slot: slots,
        };
        data.signature = data.compute_signature(&signing_key().bytes);
        data
    }

    /// Fixed-order text of every signed field. `name` is left out: it's
    /// re-derived from the Pokédex id on load.
    fn canonical(&self) -> String {
        let mut out = format!(
            "poclimon-save\nversion={}\nkey_id={}\nunlocked=",
            self.version, self.key_id
        );
        let unlocked: Vec<String> = self.progress.unlocked.iter().map(u32::to_string).collect();
        out.push_str(&unlocked.join(","));
        out.push('\n');
        for s in &self.slot {
            out.push_str(&format!(
                "slot id={} slot_id={} level={} xp={}\n",
                s.id, s.slot_id, s.level, s.xp
            ));
        }
        out
    }

    fn compute_signature(&self, key: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
        mac.update(self.canonical().as_bytes());
        encode_hex(&mac.finalize().into_bytes())
    }

    /// Whether the signature matches this build's key.
    fn verify(&self) -> bool {
        let key = signing_key();
        if self.key_id != key.id {
            return false;
        }
        let Some(expected) = decode_hex(&self.signature) else {
            return false;
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&key.bytes).expect("any key length");
        mac.update(self.canonical().as_bytes());
        // Constant-time comparison.
        mac.verify_slice(&expected).is_ok()
    }

    fn to_toml(&self) -> Result<String, ConfigError> {
        let body = toml::to_string(self)?;
        Ok(format!(
            "# PoCLImon progress. Edited by hand, it fails verification and isn't loaded.\n\
             # Settings live in poclimon.toml, which is safe to edit.\n\n{body}"
        ))
    }
}

/// Why a save file couldn't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Parsed, but the signature doesn't match: edited outside the game.
    Tampered,
    /// Unreadable or not a save at all.
    Damaged,
}

/// What `load` found.
#[derive(Debug, PartialEq)]
pub enum LoadOutcome {
    /// A valid save.
    Loaded(SaveData),
    /// The save was rejected (and moved aside); this is the backup instead.
    Restored(SaveData, Problem),
    /// The save was rejected and there was no usable backup.
    Lost(Problem),
    /// No save file at all. `first_ever` is false when a backup or a rejected
    /// file shows a save once existed, which bars legacy import.
    Missing { first_ever: bool },
}

fn read_verified(path: &Path) -> Result<SaveData, Problem> {
    let text = fs::read_to_string(path).map_err(|_| Problem::Damaged)?;
    let data: SaveData = toml::from_str(&text).map_err(|_| Problem::Damaged)?;
    if data.verify() {
        Ok(data)
    } else {
        Err(Problem::Tampered)
    }
}

fn backup_path(path: &Path) -> PathBuf {
    sibling(path, ".bak")
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Whether any `<save>.rejected-*` file sits next to `path`.
fn has_rejected(path: &Path) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let prefix = format!("{}.rejected-", name.to_string_lossy());
    fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(&prefix))
    })
}

/// Load the save at `path`, falling back to its backup.
///
/// A rejected save is renamed to `<save>.rejected-<unix time>` so it's kept
/// for inspection but never loaded or overwritten.
pub fn load(path: &Path) -> LoadOutcome {
    let bak = backup_path(path);
    if !path.exists() {
        // A crash between backup rotation and the write can leave only `.bak`.
        if let Ok(data) = read_verified(&bak) {
            return LoadOutcome::Loaded(data);
        }
        let first_ever = !bak.exists() && !has_rejected(path);
        return LoadOutcome::Missing { first_ever };
    }

    let problem = match read_verified(path) {
        Ok(data) => return LoadOutcome::Loaded(data),
        Err(problem) => problem,
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = fs::rename(path, sibling(path, &format!(".rejected-{stamp}")));

    match read_verified(&bak) {
        Ok(data) => LoadOutcome::Restored(data, problem),
        Err(_) => LoadOutcome::Lost(problem),
    }
}

/// Sign and write `data` to `path`, first rotating the current save to `.bak`.
pub fn write(path: &Path, data: &SaveData) -> Result<(), ConfigError> {
    // Only a save that still verifies becomes the backup, so a bad file can't
    // displace the last good one.
    let bak = backup_path(path);
    if read_verified(path).is_ok() {
        write_atomic(&bak, &fs::read_to_string(path)?)?;
    }
    let text = data.to_toml()?;
    write_atomic(path, &text)?;
    // The first save also seeds the backup, so deleting just the save file
    // doesn't make the game look freshly installed (legacy import is refused
    // once any save has existed).
    if !bak.exists() {
        write_atomic(&bak, &text)?;
    }
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("poclimon-save-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("save.toml")
    }

    fn sample() -> SaveData {
        SaveData::new(
            vec![SlotEntry {
                id: 4,
                slot_id: 42,
                name: "Charmander".to_string(),
                level: 3,
                xp: 17,
            }],
            vec![4],
        )
    }

    fn edit(path: &Path, from: &str, to: &str) {
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains(from), "{from:?} not in save");
        fs::write(path, text.replacen(from, to, 1)).unwrap();
    }

    #[test]
    fn round_trip() {
        let path = scratch("round-trip");
        write(&path, &sample()).unwrap();
        assert_eq!(load(&path), LoadOutcome::Loaded(sample()));
    }

    #[test]
    fn whitespace_and_comment_edits_still_verify() {
        let path = scratch("cosmetic");
        write(&path, &sample()).unwrap();
        edit(&path, "level = 3", "level   =   3   # my starter");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("# hello\n\n{text}\n\n")).unwrap();
        assert_eq!(load(&path), LoadOutcome::Loaded(sample()));
    }

    #[test]
    fn each_progress_edit_is_rejected() {
        for (from, to) in [
            ("level = 3", "level = 36"),
            ("xp = 17", "xp = 9999"),
            ("id = 4", "id = 6"),
            ("unlocked = [4]", "unlocked = [4, 150]"),
        ] {
            let path = scratch("tamper");
            write(&path, &sample()).unwrap();
            edit(&path, from, to);
            assert_eq!(
                load(&path),
                LoadOutcome::Restored(sample(), Problem::Tampered),
                "{to}"
            );
        }
    }

    #[test]
    fn missing_signature_is_rejected() {
        let path = scratch("no-sig");
        write(&path, &sample()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let stripped: String = text
            .lines()
            .filter(|l| !l.starts_with("signature"))
            .map(|l| format!("{l}\n"))
            .collect();
        fs::write(&path, stripped).unwrap();
        assert_eq!(
            load(&path),
            LoadOutcome::Restored(sample(), Problem::Tampered)
        );
    }

    #[test]
    fn save_signed_with_another_key_is_rejected() {
        let path = scratch("other-key");
        let mut data = sample();
        data.key_id = if is_dev_build() { "r1" } else { "dev" }.to_string();
        data.signature = data.compute_signature(b"some other key");
        write_atomic(&path, &data.to_toml().unwrap()).unwrap();
        assert_eq!(load(&path), LoadOutcome::Lost(Problem::Tampered));
    }

    #[test]
    fn tampered_save_falls_back_to_backup_and_is_kept() {
        let path = scratch("fallback");
        let mut older = sample();
        older.slot[0].xp = 5;
        older = SaveData::new(older.slot, older.progress.unlocked);
        write(&path, &older).unwrap();
        write(&path, &sample()).unwrap(); // `older` rotates to .bak
        edit(&path, "level = 3", "level = 99");

        assert_eq!(load(&path), LoadOutcome::Restored(older, Problem::Tampered));
        assert!(!path.exists());
        assert!(has_rejected(&path), "rejected file not kept");
    }

    #[test]
    fn damaged_save_is_reported_as_damaged() {
        let path = scratch("damaged");
        fs::write(&path, "this is not toml [[[").unwrap();
        assert_eq!(load(&path), LoadOutcome::Lost(Problem::Damaged));
    }

    #[test]
    fn crash_between_rotation_and_write_loses_nothing() {
        let path = scratch("crash");
        write(&path, &sample()).unwrap();
        write(&path, &sample()).unwrap();
        // Simulate dying mid-write: temp file left behind, main file gone.
        fs::write(path.with_file_name(".save.toml.tmp"), "half a fi").unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(load(&path), LoadOutcome::Loaded(sample()));
    }

    #[test]
    fn missing_save_reports_whether_one_ever_existed() {
        let path = scratch("missing");
        assert_eq!(load(&path), LoadOutcome::Missing { first_ever: true });

        fs::write(sibling(&path, ".rejected-1"), "x").unwrap();
        assert_eq!(load(&path), LoadOutcome::Missing { first_ever: false });
    }

    #[test]
    fn dev_builds_use_a_separate_file() {
        let path = save_path_for(Path::new("/home/x/.config/poclimon.toml"));
        let expected = if is_dev_build() {
            "save.dev.toml"
        } else {
            "save.toml"
        };
        assert_eq!(path, Path::new("/home/x/.config/poclimon").join(expected));
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(
            decode_hex(&encode_hex(&[0, 15, 255])),
            Some(vec![0, 15, 255])
        );
        assert_eq!(decode_hex("abc"), None);
        assert_eq!(decode_hex("zz"), None);
    }
}
