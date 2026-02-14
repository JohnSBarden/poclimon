/// A detected/known eye region.
#[derive(Debug, Clone)]
pub struct EyeRegion {
    pub cx: u32,
    pub cy: u32,
    pub radius: u32,
}

/// Get eye positions for a creature by ID.
///
/// For the v0.0.1 roster, eye positions are manually calibrated against
/// the PokeAPI official-artwork sprites (475x475). Programmatic detection
/// on stylized artwork is unreliable (shading, ears, markings all create
/// false positives), so we use known positions for supported creatures.
///
/// Returns empty vec for unknown creatures (sleeping animation will still
/// work, just without the closed-eye effect).
pub fn get_eye_regions(creature_id: u32) -> Vec<EyeRegion> {
    match creature_id {
        // Bulbasaur: large red eyes, slight head tilt
        1 => vec![
            EyeRegion { cx: 188, cy: 258, radius: 14 },
            EyeRegion { cx: 268, cy: 248, radius: 14 },
        ],
        // Charmander: rounded eyes, slight head tilt
        4 => vec![
            EyeRegion { cx: 168, cy: 128, radius: 12 },
            EyeRegion { cx: 228, cy: 118, radius: 12 },
        ],
        // Squirtle: large reddish-brown eyes, three-quarter view
        7 => vec![
            EyeRegion { cx: 170, cy: 100, radius: 15 },
            EyeRegion { cx: 230, cy: 100, radius: 15 },
        ],
        // Pikachu: small dark oval eyes
        25 => vec![
            EyeRegion { cx: 140, cy: 165, radius: 10 },
            EyeRegion { cx: 194, cy: 158, radius: 10 },
        ],
        // Eevee: large round brown eyes
        133 => vec![
            EyeRegion { cx: 128, cy: 200, radius: 14 },
            EyeRegion { cx: 212, cy: 212, radius: 14 },
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roster_creatures_have_eyes() {
        for id in [1, 4, 7, 25, 133] {
            let eyes = get_eye_regions(id);
            assert_eq!(eyes.len(), 2, "Creature {} should have 2 eye regions", id);
        }
    }

    #[test]
    fn test_unknown_creature_no_eyes() {
        assert!(get_eye_regions(999).is_empty());
    }

    #[test]
    fn test_eyes_are_horizontally_paired() {
        for id in [1, 4, 7, 25, 133] {
            let eyes = get_eye_regions(id);
            let y_diff = (eyes[0].cy as i32 - eyes[1].cy as i32).unsigned_abs();
            assert!(y_diff < 50, "Eyes for creature {} should be roughly aligned (y_diff={})", id, y_diff);
            let x_diff = (eyes[0].cx as i32 - eyes[1].cx as i32).unsigned_abs();
            assert!(x_diff > 50, "Eyes for creature {} should be spread apart (x_diff={})", id, x_diff);
        }
    }
}
