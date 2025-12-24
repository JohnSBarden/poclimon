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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimalConfig {
    pub name: String,
    pub kind: String,
    pub position: (u16, u16),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GameConfig {
    pub animals: Vec<AnimalConfig>,
    pub max_animals: usize,
    pub frame_delay_ms: u64,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            animals: vec![
                AnimalConfig {
                    name: "Whiskers".to_string(),
                    kind: "cat".to_string(),
                    position: (10, 5),
                },
                AnimalConfig {
                    name: "Buddy".to_string(),
                    kind: "dog".to_string(),
                    position: (30, 10),
                },
                AnimalConfig {
                    name: "Tweety".to_string(),
                    kind: "bird".to_string(),
                    position: (20, 15),
                },
            ],
            max_animals: 10,
            frame_delay_ms: 100,
        }
    }
}

impl GameConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        let config: GameConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}
