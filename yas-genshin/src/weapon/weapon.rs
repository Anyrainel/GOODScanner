use regex::Regex;

#[derive(Debug, Clone)]
pub struct GenshinWeapon {
    pub name: String,
    pub level: i32,
    pub ascension: i32,
    pub refinement: i32,
    pub star: i32,
    pub lock: bool,
    pub equip: Option<String>,
}

/// Parse a level/ascension string in the format "Lv.XX/YY" or "XX/YY".
///
/// Returns `(level, ascension, ascended)` where:
/// - `level` is the current level
/// - `ascension` is the ascension phase (0-6)
/// - `ascended` is true when the weapon has been ascended but not yet leveled
///   (i.e., level < max_level at a boundary)
///
/// Ascension is derived from the max level:
///   20 → 0, 40 → 1, 50 → 2, 60 → 3, 70 → 4, 80 → 5, 90 → 6
pub fn parse_level_and_ascension(raw: &str) -> Option<(i32, i32, bool)> {
    let re = Regex::new(r"(?:Lv\.)?(\d+)/(\d+)").ok()?;
    let caps = re.captures(raw)?;

    let level: i32 = caps.get(1)?.as_str().parse().ok()?;
    let max_level: i32 = caps.get(2)?.as_str().parse().ok()?;

    let ascension = match max_level {
        20 => 0,
        40 => 1,
        50 => 2,
        60 => 3,
        70 => 4,
        80 => 5,
        90 => 6,
        _ => return None,
    };

    let ascended = level < max_level;

    Some((level, ascension, ascended))
}

/// Parse a refinement string in the format "精炼X阶" where X is a digit 1-5.
///
/// Returns the refinement rank (1-5).
pub fn parse_refinement(raw: &str) -> Option<i32> {
    let re = Regex::new(r"精炼(\d)阶").ok()?;
    let caps = re.captures(raw)?;

    let refinement: i32 = caps.get(1)?.as_str().parse().ok()?;
    if (1..=5).contains(&refinement) {
        Some(refinement)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_level_and_ascension() {
        // Basic Lv. format
        assert_eq!(parse_level_and_ascension("Lv.90/90"), Some((90, 6, false)));
        assert_eq!(parse_level_and_ascension("Lv.80/90"), Some((80, 6, true)));
        assert_eq!(parse_level_and_ascension("Lv.80/80"), Some((80, 5, false)));
        assert_eq!(parse_level_and_ascension("Lv.1/20"), Some((1, 0, true)));
        assert_eq!(parse_level_and_ascension("Lv.20/20"), Some((20, 0, false)));
        assert_eq!(parse_level_and_ascension("Lv.20/40"), Some((20, 1, true)));

        // Without Lv. prefix
        assert_eq!(parse_level_and_ascension("90/90"), Some((90, 6, false)));
        assert_eq!(parse_level_and_ascension("1/20"), Some((1, 0, true)));

        // Invalid
        assert_eq!(parse_level_and_ascension("Lv.90/100"), None);
        assert_eq!(parse_level_and_ascension("hello"), None);
    }

    #[test]
    fn test_parse_refinement() {
        assert_eq!(parse_refinement("精炼1阶"), Some(1));
        assert_eq!(parse_refinement("精炼5阶"), Some(5));
        assert_eq!(parse_refinement("精炼3阶"), Some(3));
        assert_eq!(parse_refinement("something"), None);
    }
}
