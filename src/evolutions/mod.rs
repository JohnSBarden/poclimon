//! Evolution data and the species selection lock (#27).
//!
//! The tables in `data.rs` are generated from PokeAPI; see
//! `tools/gen-evolutions/gen.py` to regenerate them.

mod data;

pub use data::{BASE_SELECTABLE, EVOLUTIONS};

/// How a species evolves. Only `Level` is reachable in v1; the rest are
/// placeholders for later item, trade and friendship support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// At this canonical level (side conditions such as time of day ignored).
    Level(u8),
    Item,
    Trade,
    Friendship,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evolution {
    pub from: u16,
    pub to: u16,
    pub method: Method,
}

/// Species selectable without being unlocked, because nothing can unlock them
/// until item evolution exists. Remove when it ships (unlocks keep them for
/// anyone who already has them).
pub const TEMPORARILY_SELECTABLE: &[u32] = &[
    134, // Vaporeon
    135, // Jolteon
    136, // Flareon
];

/// Every evolution out of `id`, in Pokédex order of the target.
pub fn evolutions_from(id: u32) -> impl Iterator<Item = &'static Evolution> {
    let Ok(id) = u16::try_from(id) else {
        return EVOLUTIONS[..0].iter();
    };
    let start = EVOLUTIONS.partition_point(|e| e.from < id);
    let end = EVOLUTIONS.partition_point(|e| e.from <= id);
    EVOLUTIONS[start..end].iter()
}

/// Species `id` can evolve into by level at `level`. More than one means a
/// branch (Tyrogue, Wurmple, ...) and the player picks.
pub fn level_evolutions(id: u32, level: u32) -> Vec<u32> {
    evolutions_from(id)
        .filter(|e| matches!(e.method, Method::Level(at) if level >= u32::from(at)))
        .map(|e| u32::from(e.to))
        .collect()
}

/// Whether the player may add or swap in species `id`.
pub fn is_selectable(id: u32, unlocked: &[u32]) -> bool {
    u16::try_from(id).is_ok_and(|id16| BASE_SELECTABLE.binary_search(&id16).is_ok())
        || TEMPORARILY_SELECTABLE.contains(&id)
        || unlocked.contains(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creatures;

    #[test]
    fn every_id_is_in_the_pokedex() {
        for e in EVOLUTIONS {
            assert!(creatures::find_by_id(e.from.into()).is_some(), "{e:?}");
            assert!(creatures::find_by_id(e.to.into()).is_some(), "{e:?}");
        }
        for &id in BASE_SELECTABLE {
            assert!(creatures::find_by_id(id.into()).is_some(), "{id}");
        }
    }

    #[test]
    fn tables_are_sorted_for_lookup() {
        assert!(
            EVOLUTIONS
                .windows(2)
                .all(|w| (w[0].from, w[0].to) < (w[1].from, w[1].to))
        );
        assert!(BASE_SELECTABLE.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn no_self_evolution_or_cycles() {
        for e in EVOLUTIONS {
            assert_ne!(e.from, e.to);
            // Walk forward; a chain is at most 3 stages, so 8 steps means a cycle.
            let mut frontier = vec![u32::from(e.to)];
            for _ in 0..8 {
                frontier = frontier
                    .iter()
                    .flat_map(|&id| evolutions_from(id).map(|n| u32::from(n.to)))
                    .collect();
                assert!(!frontier.contains(&u32::from(e.from)), "cycle via {e:?}");
            }
            assert!(frontier.is_empty(), "chain from {e:?} never ends");
        }
    }

    #[test]
    fn regional_only_evolutions_are_excluded() {
        assert!(!EVOLUTIONS.iter().any(|e| (e.from, e.to) == (52, 863)));
        assert!(!EVOLUTIONS.iter().any(|e| (e.from, e.to) == (194, 980)));
    }

    #[test]
    fn canonical_levels() {
        assert_eq!(level_evolutions(1, 15), Vec::<u32>::new());
        assert_eq!(level_evolutions(1, 16), vec![2]); // Bulbasaur -> Ivysaur
        assert_eq!(level_evolutions(5, 36), vec![6]); // Charmeleon -> Charizard
        assert_eq!(level_evolutions(236, 20), vec![106, 107, 237]); // Tyrogue branches
    }

    #[test]
    fn eevee_and_pikachu_do_not_level_evolve_in_v1() {
        assert!(level_evolutions(133, 100).is_empty());
        assert!(level_evolutions(25, 100).is_empty());
    }

    #[test]
    fn selection_lock() {
        // Base forms, including every default starter.
        for id in [1, 4, 7, 25, 133] {
            assert!(is_selectable(id, &[]), "{id}");
        }
        // Evolves from a baby (Pichu -> Pikachu, Munchlax -> Snorlax).
        assert!(is_selectable(143, &[]));
        // Evolved forms are locked until unlocked.
        assert!(!is_selectable(2, &[]));
        assert!(!is_selectable(26, &[]));
        assert!(is_selectable(2, &[2]));
        // Eeveelutions that need items stay available for now.
        assert!(is_selectable(134, &[]));
        assert!(!is_selectable(197, &[]));
        assert!(!is_selectable(u32::MAX, &[]));
    }
}
