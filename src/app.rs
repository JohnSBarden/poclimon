//! Application state for PoCLImon.

use crate::config::{GameConfig, PokemonEntry};
use crate::sprite::SpriteCache;
use anyhow::Result;
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::path::PathBuf;

/// Per-Pokémon runtime state (stats, cached image).
pub struct PetState {
    pub entry: PokemonEntry,
    pub happiness: u32,
    pub hunger: u32,
    pub image: Option<StatefulProtocol>,
}

impl PetState {
    pub fn new(entry: PokemonEntry) -> Self {
        Self {
            entry,
            happiness: 50,
            hunger: 50,
            image: None,
        }
    }

    /// Feed this Pokémon: decreases hunger, slightly increases happiness.
    pub fn feed(&mut self) {
        self.hunger = self.hunger.saturating_sub(10);
        self.happiness = (self.happiness + 3).min(100);
    }

    /// Pet this Pokémon: increases happiness.
    pub fn pet(&mut self) {
        self.happiness = (self.happiness + 10).min(100);
    }
}

/// Main application state.
pub struct App {
    pub pets: Vec<PetState>,
    pub current_slot: usize,
    pub running: bool,
    pub status_message: Option<String>,
    pub sprite_cache: SpriteCache,
}

impl App {
    /// Create a new App from the given config.
    pub fn new(config: GameConfig) -> Result<Self> {
        let cache_dir = sprite_dir();
        let sprite_cache = SpriteCache::new(cache_dir)?;
        let pets: Vec<PetState> = config.roster.into_iter().map(PetState::new).collect();

        if pets.is_empty() {
            anyhow::bail!("Roster is empty — need at least one Pokémon");
        }

        Ok(Self {
            pets,
            current_slot: 0,
            running: true,
            status_message: None,
            sprite_cache,
        })
    }

    /// Get the current pet.
    pub fn current_pet(&self) -> &PetState {
        &self.pets[self.current_slot]
    }

    /// Get the current pet mutably.
    pub fn current_pet_mut(&mut self) -> &mut PetState {
        &mut self.pets[self.current_slot]
    }

    /// Total number of slots.
    pub fn slot_count(&self) -> usize {
        self.pets.len()
    }

    /// Switch to the next Pokémon in the roster.
    pub fn next_slot(&mut self) {
        self.current_slot = (self.current_slot + 1) % self.pets.len();
        self.status_message = None;
    }

    /// Switch to the previous Pokémon in the roster.
    pub fn prev_slot(&mut self) {
        if self.current_slot == 0 {
            self.current_slot = self.pets.len() - 1;
        } else {
            self.current_slot -= 1;
        }
        self.status_message = None;
    }

    /// Jump to a specific slot (0-indexed). Returns false if out of range.
    pub fn goto_slot(&mut self, slot: usize) -> bool {
        if slot < self.pets.len() {
            self.current_slot = slot;
            self.status_message = None;
            true
        } else {
            false
        }
    }

    /// Ensure the current pet's sprite is loaded, downloading if needed.
    pub fn ensure_sprite_loaded(&mut self, picker: &mut Picker) {
        let slot = self.current_slot;
        if self.pets[slot].image.is_some() {
            return;
        }

        let id = self.pets[slot].entry.id;
        let name = self.pets[slot].entry.name.clone();

        match self.sprite_cache.get_or_download(id, &name) {
            Ok(path) => {
                let result = image::ImageReader::open(&path)
                    .map_err(anyhow::Error::from)
                    .and_then(|r| r.decode().map_err(anyhow::Error::from));
                match result {
                    Ok(dyn_img) => {
                        self.pets[slot].image = Some(picker.new_resize_protocol(dyn_img));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Failed to decode sprite: {e}"));
                    }
                }
            }
            Err(e) => {
                // Try fallback
                let fallback_path = self.sprite_cache.cache_dir().join(format!("{id}_fallback.png"));
                if crate::sprite::create_fallback_sprite(&fallback_path).is_ok() {
                    if let Ok(dyn_img) = image::ImageReader::open(&fallback_path)
                        .map_err(anyhow::Error::from)
                        .and_then(|r| r.decode().map_err(anyhow::Error::from))
                    {
                        self.pets[slot].image = Some(picker.new_resize_protocol(dyn_img));
                    }
                }
                self.status_message = Some(format!("Download failed: {e} (using fallback)"));
            }
        }
    }
}

/// Get the sprite cache directory path.
fn sprite_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".poclimon")
        .join("sprites")
}
