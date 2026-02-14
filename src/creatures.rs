//! Built-in creature definitions.
//!
//! Each creature maps to a Pokémon with a National Dex ID, which is used
//! to find their sprites in the PMDCollab SpriteCollab repository.

/// A creature available in PoCLImon.
pub struct CreatureDef {
    /// National Pokédex ID (e.g., 25 for Pikachu)
    pub id: u32,
    /// Display name
    pub name: &'static str,
}

impl CreatureDef {
    /// Get the zero-padded ID string used in PMDCollab paths (e.g., "0025").
    #[allow(dead_code)]
    pub fn padded_id(&self) -> String {
        format!("{:04}", self.id)
    }
}

/// The default roster of creatures available in v0.0.1.
pub const ROSTER: &[CreatureDef] = &[
    CreatureDef { id: 1, name: "Bulbasaur" },
    CreatureDef { id: 4, name: "Charmander" },
    CreatureDef { id: 7, name: "Squirtle" },
    CreatureDef { id: 25, name: "Pikachu" },
    CreatureDef { id: 133, name: "Eevee" },
];

/// Find a creature by ID.
pub fn find_by_id(id: u32) -> Option<&'static CreatureDef> {
    ROSTER.iter().find(|c| c.id == id)
}

/// Find a creature by name (case-insensitive).
pub fn find_by_name(name: &str) -> Option<&'static CreatureDef> {
    let lower = name.to_lowercase();
    ROSTER.iter().find(|c| c.name.to_lowercase() == lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roster_has_five_creatures() {
        assert_eq!(ROSTER.len(), 5);
    }

    #[test]
    fn test_find_by_id() {
        assert_eq!(find_by_id(25).unwrap().name, "Pikachu");
        assert_eq!(find_by_id(1).unwrap().name, "Bulbasaur");
        assert!(find_by_id(999).is_none());
    }

    #[test]
    fn test_find_by_name() {
        assert_eq!(find_by_name("eevee").unwrap().id, 133);
        assert_eq!(find_by_name("Charmander").unwrap().id, 4);
        assert!(find_by_name("Mewtwo").is_none());
    }

    #[test]
    fn test_padded_id() {
        assert_eq!(find_by_id(1).unwrap().padded_id(), "0001");
        assert_eq!(find_by_id(25).unwrap().padded_id(), "0025");
        assert_eq!(find_by_id(133).unwrap().padded_id(), "0133");
    }
}
