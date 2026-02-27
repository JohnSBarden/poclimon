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

/// Raw TOML config structure.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TomlConfig {
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub roster: RosterConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayConfig {
    #[serde(default = "default_scale")]
    pub scale: u32,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self { scale: default_scale() }
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
        "pikachu".to_string(),
        "eevee".to_string(),
    ]
}

/// Resolved game config with validated creature IDs.
#[derive(Debug, Clone)]
pub struct GameConfig {
    pub scale: u32,
    /// List of (creature_id, creature_name) for the active roster.
    pub roster: Vec<(u32, String)>,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            scale: 3,
            roster: vec![
                (1, "Bulbasaur".to_string()),
                (4, "Charmander".to_string()),
                (7, "Squirtle".to_string()),
                (25, "Pikachu".to_string()),
                (133, "Eevee".to_string()),
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

    /// Load from the default config path (~/.poclimon/config.toml).
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
        let creature = creatures::find_by_name(name).ok_or_else(|| {
            ConfigError::Validation(format!("Unknown creature: '{name}'"))
        })?;
        Ok(Self {
            scale: 3,
            roster: vec![(creature.id, creature.name.to_string())],
        })
    }

    /// Convert a parsed TOML config into a validated GameConfig.
    pub fn from_toml(toml: TomlConfig) -> Result<Self, ConfigError> {
        let creatures = &toml.roster.creatures;

        if creatures.len() > MAX_ACTIVE_CREATURES {
            return Err(ConfigError::Validation(
                format!("Maximum {MAX_ACTIVE_CREATURES} creatures allowed in roster"),
            ));
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
                    roster.push((c.id, c.name.to_string()));
                    continue;
                }
                return Err(ConfigError::Validation(format!("Unknown creature ID: {id}")));
            }
            // Try as name
            if let Some(c) = creatures::find_by_name(entry) {
                roster.push((c.id, c.name.to_string()));
            } else {
                return Err(ConfigError::Validation(format!("Unknown creature: '{entry}'")));
            }
        }

        Ok(Self {
            scale: toml.display.scale,
            roster,
        })
    }
}

/// Get the default config file path.
pub fn default_config_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".poclimon").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GameConfig::default();
        assert_eq!(config.scale, 3);
        assert_eq!(config.roster.len(), 5);
        assert_eq!(config.roster[0], (1, "Bulbasaur".to_string()));
        assert_eq!(config.roster[3], (25, "Pikachu".to_string()));
        assert_eq!(config.roster[4], (133, "Eevee".to_string()));
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
        assert_eq!(config.roster[0], (25, "Pikachu".to_string()));
        assert_eq!(config.roster[1], (133, "Eevee".to_string()));
    }

    #[test]
    fn test_parse_toml_with_ids() {
        let toml_str = r#"
[roster]
creatures = ["25", "1"]
"#;
        let toml_config: TomlConfig = toml::from_str(toml_str).unwrap();
        let config = GameConfig::from_toml(toml_config).unwrap();
        assert_eq!(config.roster[0], (25, "Pikachu".to_string()));
        assert_eq!(config.roster[1], (1, "Bulbasaur".to_string()));
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
        assert_eq!(config.roster.len(), 5);
        assert_eq!(config.roster[0].1, "Bulbasaur");
        assert_eq!(config.roster[3].1, "Pikachu");
    }

    #[test]
    fn test_from_creature_name() {
        let config = GameConfig::from_creature_name("eevee").unwrap();
        assert_eq!(config.roster.len(), 1);
        assert_eq!(config.roster[0], (133, "Eevee".to_string()));
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
        for name in &["vaporeon", "jolteon", "flareon", "articuno", "zapdos", "moltres"] {
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
        assert_eq!(config.roster[0].1, "Vaporeon");
        assert_eq!(config.roster[5].1, "Moltres");
    }
}
