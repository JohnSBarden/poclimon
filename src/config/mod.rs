//! Configuration module for PoCLImon.
//!
//! Handles loading and saving the game config, including the Pokémon roster.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

/// Errors that can occur when loading configuration.
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse config: {0}")]
    Parse(#[from] serde_json::Error),
}

/// A single Pokémon entry in the roster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PokemonEntry {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub nickname: String,
}

impl PokemonEntry {
    /// Returns the display name (nickname if set, otherwise species name).
    pub fn display_name(&self) -> &str {
        if self.nickname.is_empty() {
            &self.name
        } else {
            &self.nickname
        }
    }
}

/// Top-level game configuration.
#[derive(Debug, Serialize, Deserialize)]
pub struct GameConfig {
    pub roster: Vec<PokemonEntry>,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            roster: vec![
                PokemonEntry { id: 25, name: "Pikachu".into(), nickname: String::new() },
                PokemonEntry { id: 4, name: "Charmander".into(), nickname: String::new() },
                PokemonEntry { id: 1, name: "Bulbasaur".into(), nickname: String::new() },
                PokemonEntry { id: 7, name: "Squirtle".into(), nickname: String::new() },
                PokemonEntry { id: 133, name: "Eevee".into(), nickname: String::new() },
                PokemonEntry { id: 39, name: "Jigglypuff".into(), nickname: String::new() },
            ],
        }
    }
}

impl GameConfig {
    /// Load config from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        let config: GameConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Save config to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}
