/// GOOD character keys whose current element cannot be inferred from the character key alone.
pub const MULTI_ELEMENT_CHARACTERS: &[&str] = &["Traveler", "Manekin", "Manekina"];

/// Convert the Chinese element label shown in the character header to a GOOD element value.
pub fn good_element_from_zh(text: &str) -> Option<&'static str> {
    for (needle, element) in [
        ('火', "Pyro"),
        ('水', "Hydro"),
        ('雷', "Electro"),
        ('冰', "Cryo"),
        ('风', "Anemo"),
        ('岩', "Geo"),
        ('草', "Dendro"),
    ] {
        if text.contains(needle) {
            return Some(element);
        }
    }
    None
}

/// Resolve a captured multi-element character's current element from AvatarInfo.skill_depot_id.
///
/// These are content IDs used by the game for Traveler, Manekin, and Manekina variants.
/// Unknown depots deliberately return None instead of guessing an element.
pub fn good_element_from_skill_depot(skill_depot_id: u32) -> Option<&'static str> {
    match skill_depot_id {
        // Traveler (male 502-508, female 702-708).
        502 | 702 | 11702 | 11802 => Some("Pyro"),
        503 | 703 | 11703 | 11803 => Some("Hydro"),
        504 | 704 | 11706 | 11806 => Some("Anemo"),
        505 | 705 | 11705 | 11805 => Some("Cryo"),
        506 | 706 | 11707 | 11807 => Some("Geo"),
        507 | 707 | 11704 | 11804 => Some("Electro"),
        508 | 708 | 11708 | 11808 => Some("Dendro"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_every_traveler_manekin_and_manekina_depot() {
        for (element, depots) in [
            ("Pyro", &[502, 702, 11702, 11802][..]),
            ("Hydro", &[503, 703, 11703, 11803][..]),
            ("Anemo", &[504, 704, 11706, 11806][..]),
            ("Cryo", &[505, 705, 11705, 11805][..]),
            ("Geo", &[506, 706, 11707, 11807][..]),
            ("Electro", &[507, 707, 11704, 11804][..]),
            ("Dendro", &[508, 708, 11708, 11808][..]),
        ] {
            for depot in depots {
                assert_eq!(good_element_from_skill_depot(*depot), Some(element));
            }
        }
    }

    #[test]
    fn does_not_guess_unknown_depots() {
        assert_eq!(good_element_from_skill_depot(0), None);
        assert_eq!(good_element_from_skill_depot(99999), None);
    }

    #[test]
    fn resolves_all_chinese_element_labels() {
        for (text, element) in [
            ("火", "Pyro"),
            ("水元素", "Hydro"),
            ("雷/旅行者", "Electro"),
            ("冰", "Cryo"),
            ("风元素", "Anemo"),
            ("岩", "Geo"),
            ("草元素", "Dendro"),
        ] {
            assert_eq!(good_element_from_zh(text), Some(element));
        }
    }
}
