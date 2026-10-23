pub mod save;

use crate::creature::CreatureSlot;
use crate::creatures;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MAX_ACTIVE_CREATURES: usize = 6;

/// Bounds for `[display] scale`. 0 would produce zero-sized frames, and frame
/// memory grows with the square of the scale, so out-of-range values are clamped.
pub const MIN_SCALE: u32 = 1;
pub const MAX_SCALE: u32 = 8;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse config: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("Failed to serialize config: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("Validation error: {0}")]
    Validation(String),
}

/// A single slot entry in the new `[[slot]]` TOML format.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SlotEntry {
    pub id: u32,
    pub slot_id: u64,
    pub name: String,
    #[serde(default)]
    pub level: u32,
    #[serde(default)]
    pub xp: u32,
}

/// Raw TOML config structure.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TomlConfig {
    #[serde(default)]
    pub display: DisplayConfig,
    /// Legacy (pre-0.4) roster. `None` when the file has no `[roster]` table.
    #[serde(default)]
    pub roster: Option<RosterConfig>,
    /// New-format slot entries. When non-empty, these take precedence over
    /// `roster.creatures`.
    #[serde(default)]
    pub slot: Vec<SlotEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayConfig {
    #[serde(default = "default_scale")]
    pub scale: u32,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            scale: default_scale(),
        }
    }
}

fn default_scale() -> u32 {
    3
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RosterConfig {
    #[serde(default = "default_creatures")]
    pub creatures: Vec<String>,
}

impl Default for RosterConfig {
    fn default() -> Self {
        Self {
            creatures: default_creatures(),
        }
    }
}

fn default_creatures() -> Vec<String> {
    vec![
        "bulbasaur".to_string(),
        "charmander".to_string(),
        "squirtle".to_string(),
    ]
}

/// A slot in the resolved game roster, carrying persistence fields.
#[derive(Debug, Clone)]
pub struct RosterSlot {
    pub creature_id: u32,
    pub name: String,
    pub slot_id: u64,
    pub level: u32,
    pub xp: u32,
}

/// Resolved game config with validated creature IDs.
#[derive(Debug, Clone)]
pub struct GameConfig {
    pub scale: u32,
    /// Active roster entries.
    pub roster: Vec<RosterSlot>,
    /// Species unlocked beyond the base forms (signed with the save).
    pub unlocked: Vec<u32>,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            scale: 3,
            roster: vec![
                RosterSlot {
                    creature_id: 1,
                    name: "Bulbasaur".to_string(),
                    slot_id: new_slot_id(),
                    level: 1,
                    xp: 0,
                },
                RosterSlot {
                    creature_id: 4,
                    name: "Charmander".to_string(),
                    slot_id: new_slot_id(),
                    level: 1,
                    xp: 0,
                },
                RosterSlot {
                    creature_id: 7,
                    name: "Squirtle".to_string(),
                    slot_id: new_slot_id(),
                    level: 1,
                    xp: 0,
                },
                RosterSlot {
                    creature_id: 25,
                    name: "Pikachu".to_string(),
                    slot_id: new_slot_id(),
                    level: 1,
                    xp: 0,
                },
                RosterSlot {
                    creature_id: 133,
                    name: "Eevee".to_string(),
                    slot_id: new_slot_id(),
                    level: 1,
                    xp: 0,
                },
            ],
            unlocked: Vec::new(),
        }
    }
}

impl GameConfig {
    /// Load and validate a legacy-format config file (tests only; see `load_game`).
    #[cfg(test)]
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        let toml_config: TomlConfig = toml::from_str(&repair_oversized_slot_ids(&content))?;
        Self::from_toml(toml_config)
    }

    /// Load settings from `settings_path` and progress from the signed save
    /// next to it (see [`save::save_path_for`]), creating either if missing.
    ///
    /// A settings file that exists but can't be read or parsed is an error and
    /// is left untouched. A save that fails verification is moved aside and
    /// replaced by its backup (or a new game); the returned notice says so.
    ///
    /// Legacy rosters in the settings file (`[[slot]]` / `[roster]`, ≤0.5.x)
    /// are imported once, only if no save has ever existed, so a hand-made
    /// "legacy" file can't be used to launder edits into a signed save.
    pub fn load_game(settings_path: &Path) -> Result<LoadedGame, ConfigError> {
        let save_path = save::save_path_for(settings_path);
        let settings: TomlConfig = if settings_path.exists() {
            let content = fs::read_to_string(settings_path)?;
            toml::from_str(&repair_oversized_slot_ids(&content))?
        } else {
            write_atomic(settings_path, &render_settings(default_scale()))?;
            TomlConfig::default()
        };
        let scale = clamp_scale(settings.display.scale);
        let has_legacy = !settings.slot.is_empty() || settings.roster.is_some();

        let from_save = |data: save::SaveData| -> Result<Self, ConfigError> {
            if data.slot.is_empty() {
                return Ok(Self::default());
            }
            let mut config = Self::from_toml(TomlConfig {
                slot: data.slot,
                ..TomlConfig::default()
            })?;
            config.unlocked = data.progress.unlocked;
            Ok(config)
        };

        let mut notice = None;
        let mut needs_write = false;
        let mut config = match save::load(&save_path) {
            save::LoadOutcome::Loaded(data) => from_save(data)?,
            save::LoadOutcome::Restored(data, problem) => {
                notice = Some(problem_notice(problem, true));
                needs_write = true;
                from_save(data)?
            }
            save::LoadOutcome::Lost(problem) => {
                notice = Some(problem_notice(problem, false));
                needs_write = true;
                Self::default()
            }
            save::LoadOutcome::Missing { first_ever: true } if has_legacy => {
                let mut config = Self::from_toml(settings)?;
                // Grandfather every species the player already has (#27).
                config.unlocked = config.roster.iter().map(|r| r.creature_id).collect();
                config.unlocked.sort_unstable();
                config.unlocked.dedup();
                notice = Some("Moved your roster into the new signed save file".to_string());
                needs_write = true;
                // Dev builds leave the real settings file alone so a release
                // build can still import it.
                if !save::is_dev_build() {
                    write_atomic(settings_path, &render_settings(scale))?;
                }
                config
            }
            save::LoadOutcome::Missing { .. } => {
                needs_write = true;
                Self::default()
            }
        };
        config.scale = scale;

        if needs_write {
            config.write_save(&save_path)?;
        }
        Ok(LoadedGame {
            config,
            save_path,
            notice,
        })
    }

    /// Sign and write this config's roster and unlocks to `save_path`.
    fn write_save(&self, save_path: &Path) -> Result<(), ConfigError> {
        let entries = self
            .roster
            .iter()
            .map(|r| SlotEntry {
                id: r.creature_id,
                slot_id: r.slot_id,
                name: r.name.clone(),
                level: r.level,
                xp: r.xp,
            })
            .collect();
        save::write(
            save_path,
            &save::SaveData::new(entries, self.unlocked.clone()),
        )
    }

    /// Create a GameConfig from a single creature name (CLI override).
    pub fn from_creature_name(name: &str) -> Result<Self, ConfigError> {
        let creature = creatures::find_by_name(name)
            .ok_or_else(|| ConfigError::Validation(format!("Unknown creature: '{name}'")))?;
        Ok(Self {
            scale: 3,
            roster: vec![RosterSlot {
                creature_id: creature.id,
                name: creature.name.to_string(),
                slot_id: new_slot_id(),
                level: 1,
                xp: 0,
            }],
            unlocked: Vec::new(),
        })
    }

    /// Convert a parsed TOML config into a validated GameConfig.
    pub fn from_toml(toml: TomlConfig) -> Result<Self, ConfigError> {
        // New format: use `[[slot]]` entries if present.
        if !toml.slot.is_empty() {
            let slots = &toml.slot;

            if slots.len() > MAX_ACTIVE_CREATURES {
                return Err(ConfigError::Validation(format!(
                    "Maximum {MAX_ACTIVE_CREATURES} creatures allowed in roster"
                )));
            }

            let mut roster = Vec::new();
            for entry in slots {
                // Validate the ID exists.
                let creature = creatures::find_by_id(entry.id).ok_or_else(|| {
                    ConfigError::Validation(format!("Unknown creature ID: {}", entry.id))
                })?;

                // slot_id == 0 means the old format generated it; give it a fresh random ID.
                let slot_id = if entry.slot_id == 0 {
                    new_slot_id()
                } else {
                    entry.slot_id
                };

                // level defaults to 1 when missing/zero.
                let level = if entry.level == 0 { 1 } else { entry.level };

                roster.push(RosterSlot {
                    creature_id: creature.id,
                    name: creature.name.to_string(),
                    slot_id,
                    level,
                    xp: entry.xp,
                });
            }

            return Ok(Self {
                scale: clamp_scale(toml.display.scale),
                roster,
                unlocked: Vec::new(),
            });
        }

        // Old format: fall back to `roster.creatures`.
        let creatures = &toml
            .roster
            .map(|r| r.creatures)
            .unwrap_or_else(default_creatures);

        if creatures.len() > MAX_ACTIVE_CREATURES {
            return Err(ConfigError::Validation(format!(
                "Maximum {MAX_ACTIVE_CREATURES} creatures allowed in roster"
            )));
        }

        if creatures.is_empty() {
            return Err(ConfigError::Validation(
                "Roster must contain at least one creature".to_string(),
            ));
        }

        let mut roster = Vec::new();
        for entry in creatures {
            // Try as numeric ID first
            if let Ok(id) = entry.parse::<u32>() {
                if let Some(c) = creatures::find_by_id(id) {
                    roster.push(RosterSlot {
                        creature_id: c.id,
                        name: c.name.to_string(),
                        slot_id: new_slot_id(),
                        level: 1,
                        xp: 0,
                    });
                    continue;
                }
                return Err(ConfigError::Validation(format!(
                    "Unknown creature ID: {id}"
                )));
            }
            // Try as name
            if let Some(c) = creatures::find_by_name(entry) {
                roster.push(RosterSlot {
                    creature_id: c.id,
                    name: c.name.to_string(),
                    slot_id: new_slot_id(),
                    level: 1,
                    xp: 0,
                });
            } else {
                return Err(ConfigError::Validation(format!(
                    "Unknown creature: '{entry}'"
                )));
            }
        }

        Ok(Self {
            scale: clamp_scale(toml.display.scale),
            roster,
            unlocked: Vec::new(),
        })
    }

    /// Sign and write the live roster and `unlocked` to the save at `path`.
    pub fn save(path: &Path, slots: &[&CreatureSlot], unlocked: &[u32]) -> Result<(), ConfigError> {
        let slot_entries: Vec<SlotEntry> = slots
            .iter()
            .map(|s| SlotEntry {
                id: s.creature_id,
                slot_id: s.slot_id,
                name: s.creature_name.clone(),
                level: s.level,
                xp: s.xp,
            })
            .collect();
        save::write(path, &save::SaveData::new(slot_entries, unlocked.to_vec()))
    }
}

/// Settings and progress for a session, from [`GameConfig::load_game`].
pub struct LoadedGame {
    pub config: GameConfig,
    /// Where progress is saved during play.
    pub save_path: PathBuf,
    /// Something the player should be told on start (restored backup, import).
    pub notice: Option<String>,
}

fn problem_notice(problem: save::Problem, restored: bool) -> String {
    let what = match problem {
        save::Problem::Tampered => "Your save was modified outside PoCLImon and was not loaded",
        save::Problem::Damaged => "Your save file was damaged",
    };
    let then = if restored {
        "restored the last backup"
    } else {
        "no backup was usable, so this is a new game"
    };
    format!("{what}; {then}. The old file was kept next to it as *.rejected-<time>")
}

/// A random, non-zero slot id that fits in a TOML integer (i64).
///
/// TOML can't represent u64 values above `i64::MAX`: they're written fine but
/// fail to parse on the next load. 0 is reserved as "regenerate on load".
pub fn new_slot_id() -> u64 {
    loop {
        let id = rand::random::<u64>() & i64::MAX as u64;
        if id != 0 {
            return id;
        }
    }
}

/// Rewrite `slot_id = N` lines whose value doesn't fit in a TOML integer.
///
/// Versions up to 0.5.1 generated slot ids from the full u64 range, so most
/// saves contain at least one id above `i64::MAX`, which the TOML parser
/// rejects. Slot ids only need to be unique within the roster, so the
/// oversized ones are masked into range (and are written back that way on
/// the next save). Every other line is passed through unchanged.
fn repair_oversized_slot_ids(content: &str) -> std::borrow::Cow<'_, str> {
    let fix_line = |line: &str| -> Option<String> {
        let (key, value) = line.split_once('=')?;
        if key.trim() != "slot_id" {
            return None;
        }
        let digits = value.trim();
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // Fits already (or not a u64 at all): leave it to the parser.
        let n: u64 = digits.parse().ok().filter(|&n| n > i64::MAX as u64)?;
        Some(format!("{key}= {}", n & i64::MAX as u64))
    };

    if !content.lines().any(|l| fix_line(l).is_some()) {
        return std::borrow::Cow::Borrowed(content);
    }
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        match fix_line(line) {
            Some(fixed) => out.push_str(&fixed),
            None => out.push_str(line),
        }
        out.push('\n');
    }
    std::borrow::Cow::Owned(out)
}

fn clamp_scale(scale: u32) -> u32 {
    scale.clamp(MIN_SCALE, MAX_SCALE)
}

/// The settings file. Progress lives in the signed save, not here.
fn render_settings(scale: u32) -> String {
    format!(
        "# PoCLImon settings. Safe to edit; your Pokémon are in poclimon/save.toml.\n\
         [display]\n\
         scale = {scale}  # sprite size multiplier, {MIN_SCALE}-{MAX_SCALE}\n"
    )
}

/// Replace `path` with `contents` so that a crash at any point leaves either
/// the old file or the new one, never a truncated mix.
///
/// Writes a hidden sibling temp file, fsyncs it, then renames it over `path`
/// (rename is atomic within a directory).
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;

    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "poclimon.toml".to_string());
    let tmp = dir.join(format!(".{file_name}.tmp"));

    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;

    // Persist the rename itself. Best effort: not every platform can open a
    // directory for syncing.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Get the default config file path.
pub fn default_config_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("poclimon.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GameConfig::default();
        assert_eq!(config.scale, 3);
        assert_eq!(config.roster.len(), 5);
        assert_eq!(config.roster[0].creature_id, 1);
        assert_eq!(config.roster[0].name, "Bulbasaur");
        assert_eq!(config.roster[3].creature_id, 25);
        assert_eq!(config.roster[3].name, "Pikachu");
        assert_eq!(config.roster[4].creature_id, 133);
        assert_eq!(config.roster[4].name, "Eevee");
    }

    #[test]
    fn test_parse_toml_basic() {
        let toml_str = r#"
[display]
scale = 8

[roster]
creatures = ["pikachu", "eevee"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.scale, 8);
        assert_eq!(config.roster.len(), 2);
        assert_eq!(config.roster[0].creature_id, 25);
        assert_eq!(config.roster[0].name, "Pikachu");
        assert_eq!(config.roster[1].creature_id, 133);
        assert_eq!(config.roster[1].name, "Eevee");
    }

    #[test]
    fn test_parse_toml_with_ids() {
        let toml_str = r#"
[roster]
creatures = ["25", "1"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.roster[0].creature_id, 25);
        assert_eq!(config.roster[0].name, "Pikachu");
        assert_eq!(config.roster[1].creature_id, 1);
        assert_eq!(config.roster[1].name, "Bulbasaur");
    }

    #[test]
    fn test_validation_max_six() {
        // Build a roster with 7 entries (exceeds the limit of 6).
        let toml_str = r#"
[roster]
creatures = ["pikachu","eevee","bulbasaur","charmander","squirtle","vaporeon","jolteon"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let result = GameConfig::from_toml(toml_config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Maximum 6"));
    }

    #[test]
    fn test_validation_empty_roster() {
        let toml_str = r#"
[roster]
creatures = []
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let result = GameConfig::from_toml(toml_config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least one"));
    }

    #[test]
    fn test_validation_unknown_creature() {
        let toml_str = r#"
[roster]
creatures = ["mewtwo"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let result = GameConfig::from_toml(toml_config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown creature"));
    }

    #[test]
    fn test_default_toml_config() {
        let toml_config = TomlConfig::default();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.scale, 3);
        assert_eq!(config.roster.len(), 3);
        assert_eq!(config.roster[0].name, "Bulbasaur");
        assert_eq!(config.roster[1].name, "Charmander");
        assert_eq!(config.roster[2].name, "Squirtle");
    }

    #[test]
    fn test_from_creature_name() {
        let config = GameConfig::from_creature_name("eevee").unwrap();
        assert_eq!(config.roster.len(), 1);
        assert_eq!(config.roster[0].creature_id, 133);
        assert_eq!(config.roster[0].name, "Eevee");
    }

    #[test]
    fn test_from_creature_name_unknown() {
        let result = GameConfig::from_creature_name("mewtwo");
        assert!(result.is_err());
    }

    #[test]
    fn test_six_creatures_ok() {
        let toml_str = r#"
[roster]
creatures = ["pikachu", "eevee", "bulbasaur", "charmander", "squirtle", "pikachu"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.roster.len(), 6);
    }

    #[test]
    fn test_new_creatures_by_name() {
        for name in &[
            "vaporeon", "jolteon", "flareon", "articuno", "zapdos", "moltres",
        ] {
            let config = GameConfig::from_creature_name(name).unwrap();
            assert_eq!(config.roster.len(), 1);
        }
    }

    #[test]
    fn test_new_creatures_by_id() {
        let toml_str = r#"
[roster]
creatures = ["134", "135", "136", "144", "145", "146"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.roster.len(), 6);
        assert_eq!(config.roster[0].name, "Vaporeon");
        assert_eq!(config.roster[5].name, "Moltres");
    }

    // ── New format tests ──────────────────────────────────────────────────

    #[test]
    fn test_new_format_loads_level_xp() {
        let toml_str = r#"
[display]
scale = 3

[[slot]]
id = 25
slot_id = 8675309
name = "Pikachu"
level = 3
xp = 42

[[slot]]
id = 1
slot_id = 1234567
name = "Bulbasaur"
level = 1
xp = 0
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.roster.len(), 2);
        assert_eq!(config.roster[0].creature_id, 25);
        assert_eq!(config.roster[0].name, "Pikachu");
        assert_eq!(config.roster[0].slot_id, 8675309);
        assert_eq!(config.roster[0].level, 3);
        assert_eq!(config.roster[0].xp, 42);
        assert_eq!(config.roster[1].creature_id, 1);
        assert_eq!(config.roster[1].slot_id, 1234567);
        assert_eq!(config.roster[1].level, 1);
        assert_eq!(config.roster[1].xp, 0);
    }

    #[test]
    fn test_old_format_still_loads() {
        let toml_str = r#"
[roster]
creatures = ["pikachu", "eevee"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.roster.len(), 2);
        assert_eq!(config.roster[0].name, "Pikachu");
        assert_eq!(config.roster[1].name, "Eevee");
        // Old format gets level 1 and xp 0.
        assert_eq!(config.roster[0].level, 1);
        assert_eq!(config.roster[0].xp, 0);
    }

    #[test]
    fn test_old_format_slot_id_nonzero() {
        let toml_str = r#"
[roster]
creatures = ["pikachu"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_ne!(config.roster[0].slot_id, 0);
    }

    #[test]
    fn test_slot_entry_toml_roundtrip() {
        let entry = SlotEntry {
            id: 25,
            slot_id: 9999,
            name: "Pikachu".to_string(),
            level: 5,
            xp: 100,
        };
        // Serialize to TOML and back.
        let serialized = toml::to_string(&entry).unwrap();
        let deserialized: SlotEntry = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.id, 25);
        assert_eq!(deserialized.slot_id, 9999);
        assert_eq!(deserialized.name, "Pikachu");
        assert_eq!(deserialized.level, 5);
        assert_eq!(deserialized.xp, 100);
    }

    // ── Save safety ───────────────────────────────────────────────────────

    /// A fresh, empty directory unique to this test.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "poclimon_cfg_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_load_game_rejects_corrupt_settings_and_leaves_them_untouched() {
        let dir = test_dir("corrupt");
        let path = dir.join("poclimon.toml");
        let broken = "[display]\nscale = 3\n\n[[slot]]\nid = 25\nslot_id = 1\nname = \"Pika";
        fs::write(&path, broken).unwrap();

        let result = GameConfig::load_game(&path);

        assert!(result.is_err(), "corrupt config must not load as defaults");
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
    }

    fn ids(c: &GameConfig) -> Vec<u32> {
        c.roster.iter().map(|r| r.creature_id).collect()
    }

    #[test]
    fn test_load_game_creates_settings_and_save() {
        let dir = test_dir("missing");
        let path = dir.join("nested").join("poclimon.toml");

        let game = GameConfig::load_game(&path).unwrap();

        assert!(path.exists());
        assert!(game.save_path.exists());
        assert_eq!(game.config.roster.len(), 5);
        assert!(game.notice.is_none());
        // What was written must load back to the same roster.
        let reloaded = GameConfig::load_game(&path).unwrap();
        assert_eq!(ids(&reloaded.config), ids(&game.config));
    }

    const LEGACY: &str = "[display]\nscale = 2\n\n[[slot]]\nid = 25\nslot_id = 9\n\
                          name = \"Pikachu\"\nlevel = 4\nxp = 10\n\n[[slot]]\nid = 25\n\
                          slot_id = 8\nname = \"Pikachu\"\nlevel = 1\nxp = 0\n";

    #[test]
    fn test_legacy_roster_is_imported_and_grandfathered() {
        let dir = test_dir("legacy");
        let path = dir.join("poclimon.toml");
        fs::write(&path, LEGACY).unwrap();

        let game = GameConfig::load_game(&path).unwrap();

        assert_eq!(ids(&game.config), vec![25, 25]);
        assert_eq!(game.config.roster[0].level, 4);
        assert_eq!(game.config.unlocked, vec![25]);
        assert_eq!(game.config.scale, 2);
        assert!(game.notice.is_some());
        assert!(matches!(
            save::load(&game.save_path),
            save::LoadOutcome::Loaded(_)
        ));
        if save::is_dev_build() {
            assert_eq!(fs::read_to_string(&path).unwrap(), LEGACY);
        } else {
            assert!(!fs::read_to_string(&path).unwrap().contains("[[slot]]"));
        }
    }

    #[test]
    fn test_legacy_import_refused_once_a_save_has_existed() {
        let dir = test_dir("launder");
        let path = dir.join("poclimon.toml");
        let game = GameConfig::load_game(&path).unwrap();
        // Player deletes the save and writes a "legacy" roster to launder levels.
        fs::remove_file(&game.save_path).unwrap();
        fs::write(&path, LEGACY).unwrap();

        let again = GameConfig::load_game(&path).unwrap();

        assert!(again.config.roster.iter().all(|r| r.level == 1));
        assert_ne!(ids(&again.config), vec![25, 25]);
    }

    #[test]
    fn test_tampered_save_restores_backup_with_notice() {
        let dir = test_dir("tampered");
        let path = dir.join("poclimon.toml");
        let game = GameConfig::load_game(&path).unwrap();
        let mut slot = CreatureSlot::new(4, "Charmander".to_string());
        slot.level = 2;
        GameConfig::save(&game.save_path, &[&slot], &[]).unwrap();
        slot.level = 3;
        GameConfig::save(&game.save_path, &[&slot], &[]).unwrap();
        let text = fs::read_to_string(&game.save_path).unwrap();
        fs::write(&game.save_path, text.replace("level = 3", "level = 50")).unwrap();

        let again = GameConfig::load_game(&path).unwrap();

        assert_eq!(again.config.roster[0].level, 2, "backup not restored");
        assert!(again.notice.unwrap().contains("modified outside"));
    }

    #[test]
    fn test_write_atomic_replaces_content_and_leaves_no_temp_file() {
        let dir = test_dir("atomic");
        let path = dir.join("poclimon.toml");
        fs::write(&path, "old").unwrap();

        write_atomic(&path, "new").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "poclimon.toml")
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn test_save_roundtrips_through_load() {
        let dir = test_dir("roundtrip");
        let path = dir.join("poclimon.toml");
        let mut slot = CreatureSlot::new(25, "Pikachu".to_string());
        slot.slot_id = 42;
        slot.level = 4;
        slot.xp = 17;

        GameConfig::save(&path, &[&slot], &[25]).unwrap();
        let save::LoadOutcome::Loaded(data) = save::load(&path) else {
            panic!("save didn't verify");
        };

        assert_eq!(data.slot.len(), 1);
        assert_eq!(data.slot[0].slot_id, 42);
        assert_eq!(data.slot[0].level, 4);
        assert_eq!(data.slot[0].xp, 17);
        assert_eq!(data.progress.unlocked, vec![25]);
    }

    #[test]
    fn test_new_slot_ids_fit_in_a_toml_integer() {
        // TOML integers are i64; a u64 above i64::MAX can be written but not read back.
        for _ in 0..10_000 {
            let id = new_slot_id();
            assert!(
                id <= i64::MAX as u64,
                "slot id {id} won't parse back from TOML"
            );
            assert_ne!(id, 0, "0 is the 'regenerate me' sentinel");
        }
    }

    #[test]
    fn test_oversized_slot_id_from_older_versions_still_loads() {
        // Saves written before this fix can hold u64 slot ids above i64::MAX.
        let dir = test_dir("oversized");
        let path = dir.join("poclimon.toml");
        fs::write(
            &path,
            "[display]\nscale = 3\n\n[[slot]]\nid = 4\nslot_id = 11956443417021264552\n\
             name = \"Charmander\"\nlevel = 2\nxp = 49\n\n[[slot]]\nid = 7\n\
             slot_id = 6946265537283148646\nname = \"Squirtle\"\nlevel = 3\nxp = 5\n",
        )
        .unwrap();

        let config = GameConfig::load(&path).unwrap();

        assert_eq!(config.roster.len(), 2);
        assert_eq!(config.roster[0].creature_id, 4);
        assert_eq!(config.roster[0].level, 2);
        assert_eq!(config.roster[0].xp, 49);
        assert!(config.roster[0].slot_id <= i64::MAX as u64);
        assert_ne!(config.roster[0].slot_id, config.roster[1].slot_id);
        // Ids that already fit are kept as-is.
        assert_eq!(config.roster[1].slot_id, 6946265537283148646);
        assert_eq!(config.roster[1].level, 3);
    }

    #[test]
    fn test_scale_zero_is_clamped_to_minimum() {
        let toml_config: TomlConfig =
            toml::from_str("[display]\nscale = 0\n\n[roster]\ncreatures = [\"pikachu\"]\n")
                .unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.scale, MIN_SCALE);
    }

    #[test]
    fn test_huge_scale_is_clamped_to_maximum() {
        let toml_config: TomlConfig =
            toml::from_str("[display]\nscale = 500\n\n[roster]\ncreatures = [\"pikachu\"]\n")
                .unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.scale, MAX_SCALE);
    }

    #[test]
    fn test_new_format_slot_id_zero_regenerated() {
        // slot_id = 0 in the file should be treated as "missing" and regenerated.
        let toml_str = r#"
[[slot]]
id = 25
slot_id = 0
name = "Pikachu"
level = 1
xp = 0
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_ne!(config.roster[0].slot_id, 0);
    }
}
