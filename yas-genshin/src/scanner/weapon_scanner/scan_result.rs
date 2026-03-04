#[derive(Debug, Hash, Clone, PartialEq, Eq)]
pub struct GenshinWeaponScanResult {
    pub name: String,
    pub level: String,        // "Lv.XX/YY" raw text
    pub refinement: String,   // "精炼X阶" raw text
    pub equip: String,        // equipped character raw text
    pub star: i32,
    pub lock: bool,
    pub index: usize,
}
