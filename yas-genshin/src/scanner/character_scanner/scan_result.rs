#[derive(Debug, Clone)]
pub struct GenshinCharacterScanResult {
    /// Character name (GOOD key format, e.g. "KamisatoAyaka")
    pub name: String,
    /// Element (e.g. "Cryo")
    pub element: String,
    /// Character level 1-90
    pub level: i32,
    /// Ascension phase 0-6
    pub ascension: i32,
    /// Constellation count 0-6
    pub constellation: i32,
    /// Base talent level for normal attack 1-15
    pub talent_auto: i32,
    /// Base talent level for elemental skill 1-15
    pub talent_skill: i32,
    /// Base talent level for elemental burst 1-15
    pub talent_burst: i32,
}
