use crate::creature::CreatureSlot;
use crate::creatures;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MAX_ACTIVE_CREATURES: usize = 6;

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
#[derive(Debug, Serialize, Deserialize, Clone)]
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
    #[serde(default)]
    pub roster: RosterConfig,
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
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            scale: 3,
            roster: vec![
                RosterSlot {
                    creature_id: 1,
                    name: "Bulbasaur".to_string(),
                    slot_id: rand::random::<u64>(),
                    level: 1,
                    xp: 0,
                },
                RosterSlot {
                    creature_id: 4,
                    name: "Charmander".to_string(),
                    slot_id: rand::random::<u64>(),
                    level: 1,
                    xp: 0,
                },
                RosterSlot {
                    creature_id: 7,
                    name: "Squirtle".to_string(),
                    slot_id: rand::random::<u64>(),
                    level: 1,
                    xp: 0,
                },
                RosterSlot {
                    creature_id: 25,
                    name: "Pikachu".to_string(),
                    slot_id: rand::random::<u64>(),
                    level: 1,
                    xp: 0,
                },
                RosterSlot {
                    creature_id: 133,
                    name: "Eevee".to_string(),
                    slot_id: rand::random::<u64>(),
                    level: 1,
                    xp: 0,
                },
            ],
        }
    }
}

impl GameConfig {
    /// Load and validate config from a TOML file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        let toml_config: TomlConfig = toml::from_str(&content)?;
        Self::from_toml(toml_config)
    }

    /// Load from the default config path (~/.config/poclimon.toml).
    /// Creates a default config file if it doesn't exist.
    pub fn load_default() -> Result<Self, ConfigError> {
        let path = default_config_path();
        if path.exists() {
            Self::load(path)
        } else {
            // Create the default config file so the user can discover and edit it
            let default_toml = TomlConfig::default();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let content = toml::to_string_pretty(&default_toml)?;
            fs::write(&path, &content)?;
            Ok(Self::default())
        }
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
                slot_id: rand::random::<u64>(),
                level: 1,
                xp: 0,
            }],
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
                    rand::random::<u64>()
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
                scale: toml.display.scale,
                roster,
            });
        }

        // Old format: fall back to `roster.creatures`.
        let creatures = &toml.roster.creatures;

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
                        slot_id: rand::random::<u64>(),
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
                    slot_id: rand::random::<u64>(),
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
            scale: toml.display.scale,
            roster,
        })
    }

    /// Save the current roster state back to the TOML file.
    ///
    /// Writes the new `[[slot]]` format, preserving `[display]` settings.
    /// Silently succeeds if there are no slots.
    pub fn save(path: &Path, scale: u32, slots: &[&CreatureSlot]) -> Result<(), ConfigError> {
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

        // Build a minimal TOML document manually so we get the right structure.
        // We only write [display] and [[slot]] — no legacy [roster] section.
        let mut out = String::new();
        out.push_str("[display]\n");
        out.push_str(&format!("scale = {}\n", scale));

        for entry in &slot_entries {
            out.push('\n');
            out.push_str("[[slot]]\n");
            out.push_str(&format!("id = {}\n", entry.id));
            out.push_str(&format!("slot_id = {}\n", entry.slot_id));
            out.push_str(&format!("name = \"{}\"\n", entry.name));
            out.push_str(&format!("level = {}\n", entry.level));
            out.push_str(&format!("xp = {}\n", entry.xp));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, out)?;
        Ok(())
    }
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
