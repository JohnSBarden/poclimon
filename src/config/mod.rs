use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse config: {0}")]
    Parse(#[from] serde_json::Error),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GameConfig {
    pub pokemon_id: u32,
    pub pokemon_name: String,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            pokemon_id: 25,
            pokemon_name: "Pikachu".to_string(),
        }
    }
}

impl GameConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        let config: GameConfig = serde_json::from_str(&content)?;
        Ok(config)
    }
}
