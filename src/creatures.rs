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

/// Zero-pad a National Pokédex ID to 4 digits, as used in PMDCollab sprite paths.
///
/// # Examples
/// ```
/// assert_eq!(poclimon::creatures::padded_id(25), "0025");
/// assert_eq!(poclimon::creatures::padded_id(146), "0146");
/// ```
pub fn padded_id(id: u32) -> String {
    format!("{:04}", id)
}

/// All creatures available in PoCLImon.
///
/// IDs correspond to National Pokédex numbers. Sprites are sourced from
/// the PMDCollab SpriteCollab repository and cached locally on first use.
pub const ROSTER: &[CreatureDef] = &[
    // Gen 1 starters
    CreatureDef { id: 1,   name: "Bulbasaur"  },
    CreatureDef { id: 4,   name: "Charmander" },
    CreatureDef { id: 7,   name: "Squirtle"   },
    // Electric mouse
    CreatureDef { id: 25,  name: "Pikachu"    },
    // Eevee and its evolutions
    CreatureDef { id: 133, name: "Eevee"      },
    CreatureDef { id: 134, name: "Vaporeon"   },
    CreatureDef { id: 135, name: "Jolteon"    },
    CreatureDef { id: 136, name: "Flareon"    },
    // Legendary birds
    CreatureDef { id: 144, name: "Articuno"   },
    CreatureDef { id: 145, name: "Zapdos"     },
    CreatureDef { id: 146, name: "Moltres"    },
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
    fn test_roster_has_eleven_creatures() {
        assert_eq!(ROSTER.len(), 11);
    }

    #[test]
    fn test_original_five_present() {
        assert!(find_by_id(1).is_some());   // Bulbasaur
        assert!(find_by_id(4).is_some());   // Charmander
        assert!(find_by_id(7).is_some());   // Squirtle
        assert!(find_by_id(25).is_some());  // Pikachu
        assert!(find_by_id(133).is_some()); // Eevee
    }

    #[test]
    fn test_eevee_evolutions_present() {
        assert_eq!(find_by_id(134).unwrap().name, "Vaporeon");
        assert_eq!(find_by_id(135).unwrap().name, "Jolteon");
        assert_eq!(find_by_id(136).unwrap().name, "Flareon");
    }

    #[test]
    fn test_legendary_birds_present() {
        assert_eq!(find_by_id(144).unwrap().name, "Articuno");
        assert_eq!(find_by_id(145).unwrap().name, "Zapdos");
        assert_eq!(find_by_id(146).unwrap().name, "Moltres");
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
        assert_eq!(find_by_name("vaporeon").unwrap().id, 134);
        assert_eq!(find_by_name("ZAPDOS").unwrap().id, 145);
        assert!(find_by_name("Mewtwo").is_none());
    }

    #[test]
    fn test_padded_id() {
        assert_eq!(padded_id(1), "0001");
        assert_eq!(padded_id(25), "0025");
        assert_eq!(padded_id(133), "0133");
        assert_eq!(padded_id(146), "0146");
    }
}
