use log::error;
use regex::Regex;

/// Parsed character data for GOOD format export
#[derive(Debug, Clone)]
pub struct GenshinCharacter {
    /// GOOD key format (e.g. "KamisatoAyaka")
    pub name: String,
    /// Element name (e.g. "Cryo")
    pub element: String,
    /// Character level 1-90
    pub level: i32,
    /// Ascension phase 0-6
    pub ascension: i32,
    /// Constellation count 0-6
    pub constellation: i32,
    /// Base talent levels (without constellation bonuses) 1-15
    pub talent_auto: i32,
    pub talent_skill: i32,
    pub talent_burst: i32,
}
