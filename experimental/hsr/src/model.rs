use serde::{Deserialize, Serialize};

use crate::localization::BilingualName;

pub const EXPORT_SCHEMA: &str = "goodscanner.hsr";
pub const EXPORT_SCHEMA_VERSION: u32 = 3;
pub const REFERENCE_SCHEMA_VERSION: u32 = 1;
pub const OBSERVATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GearCategory {
    Relic,
    PlanarOrnament,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum GearSlot {
    Head,
    Hands,
    Body,
    Feet,
    PlanarSphere,
    LinkRope,
}

impl GearSlot {
    pub fn category(self) -> GearCategory {
        match self {
            Self::Head | Self::Hands | Self::Body | Self::Feet => GearCategory::Relic,
            Self::PlanarSphere | Self::LinkRope => GearCategory::PlanarOrnament,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CharacterReference {
    pub game_id: u32,
    pub key: String,
    pub name: BilingualName,
    pub rarity: u8,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LightConeReference {
    pub game_id: u32,
    pub key: String,
    pub name: BilingualName,
    pub rarity: u8,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GearReference {
    pub game_id: u32,
    pub key: String,
    pub name: BilingualName,
    /// Public GIlore asset path used only to distinguish otherwise identical
    /// visible relic definitions. Legacy normalized fixtures may omit it, but
    /// the verified GIlore adapter always supplies a nonempty value.
    #[serde(default)]
    pub icon_path: String,
    pub set_key: String,
    pub set_name: BilingualName,
    pub rarity: u8,
    pub category: GearCategory,
    pub slot: GearSlot,
    /// GIlore's public relic main-affix group. Legacy normalized fixtures do
    /// not contain progression metadata and deserialize this as zero, which
    /// deliberately makes authoritative value lookup unavailable.
    #[serde(default)]
    pub main_affix_group: u32,
    #[serde(default)]
    pub max_level: u8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatValueKind {
    Flat,
    Ratio,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatReference {
    pub key: String,
    pub name: BilingualName,
    #[serde(default)]
    pub value_kind: StatValueKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelicMainAffixReference {
    pub group_id: u32,
    pub property_id: String,
    pub max_level: u8,
    pub level_values: Vec<f64>,
}

/// Normalized, provider-owned reference snapshot. GIlore can implement the
/// provider contract later without changing observation or export code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReferenceSnapshot {
    pub schema_version: u32,
    pub provider: String,
    pub revision: String,
    pub characters: Vec<CharacterReference>,
    pub light_cones: Vec<LightConeReference>,
    pub gear_pieces: Vec<GearReference>,
    pub stats: Vec<StatReference>,
    #[serde(default)]
    pub relic_main_affixes: Vec<RelicMainAffixReference>,
    /// Public, account-independent numeric achievement IDs from GIlore.
    /// Inventory-only v1.1 bundles omit this field; v1.2 bundles populate it
    /// from `achievements.json`.
    #[serde(default)]
    pub achievement_ids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceKind {
    SanitizedFixture,
    ScreenCapture,
    PacketCapture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationEvidence {
    pub kind: EvidenceKind,
    /// Scanner/build revision for live observations, or a fixture revision.
    /// It never contains an account, device, or session identifier.
    pub revision: String,
    pub coverage: InventoryCoverage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageLevel {
    Complete,
    EquippedOnly,
    ShowcaseOnly,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryCoverage {
    pub characters: CoverageLevel,
    pub light_cones: CoverageLevel,
    pub relics: CoverageLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedCharacter {
    pub character_id: u32,
    pub level: u8,
    pub ascension: u8,
    pub eidolon: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedLightCone {
    pub light_cone_id: u32,
    pub level: u8,
    pub ascension: u8,
    pub superimposition: u8,
    /// Public character template ID, never a server avatar or account ID.
    pub equipped_character_id: Option<u32>,
    /// `None` means that the evidence source did not observe this state.
    pub lock: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedSubstat {
    /// Canonical language-neutral GIlore PropertyType (for example
    /// `CriticalChanceBase`). HSR has no audited numeric stat game ID.
    pub stat_key: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedGear {
    pub piece_id: u32,
    pub level: u8,
    pub main_stat_key: String,
    pub main_stat_value: f64,
    pub substats: Vec<ObservedSubstat>,
    /// Public character template ID, never a server avatar or account ID.
    pub equipped_character_id: Option<u32>,
    /// `None` means that the evidence source did not observe this state.
    pub lock: Option<bool>,
    /// This is the reversible discard-mark state, not salvage or deletion.
    pub discard: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationSnapshot {
    pub schema_version: u32,
    pub evidence: ObservationEvidence,
    pub characters: Vec<ObservedCharacter>,
    pub light_cones: Vec<ObservedLightCone>,
    pub gear: Vec<ObservedGear>,
}

/// Internal inventory companion used by tests and packet-capture adapters.
/// User-facing scanner and capture files are HSR-Scanner v4 via
/// [`crate::scanner_export`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsrInventoryExport {
    pub schema: String,
    pub schema_version: u32,
    pub source: ExportSource,
    pub reference: ExportReference,
    pub privacy: ExportPrivacy,
    pub characters: Vec<HsrCharacter>,
    pub light_cones: Vec<HsrLightCone>,
    pub relics: Vec<HsrRelic>,
    pub planar_ornaments: Vec<HsrPlanarOrnament>,
    /// Omitted means achievements were not observed. A present, complete empty
    /// snapshot authoritatively means that zero achievements were completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub achievements: Option<HsrAchievementSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HsrAchievementSnapshot {
    pub source: AchievementSource,
    pub coverage: AchievementCoverage,
    pub entries: Vec<HsrAchievementEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AchievementSource {
    pub kind: AchievementSourceKind,
    /// Parser/build revision only. It must not encode account, device, or
    /// session identity.
    pub revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AchievementSourceKind {
    PacketCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AchievementCoverage {
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HsrAchievementEntry {
    pub achievement_id: u32,
    pub status: AchievementStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AchievementStatus {
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSource {
    pub kind: EvidenceKind,
    pub revision: String,
    pub coverage: InventoryCoverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReference {
    pub schema_version: u32,
    pub provider: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPrivacy {
    pub account_identifiers_included: bool,
    pub raw_packet_data_included: bool,
    pub server_item_identifiers_included: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsrCharacter {
    pub local_id: String,
    pub key: String,
    pub game_id: u32,
    pub name: BilingualName,
    pub rarity: u8,
    pub path: String,
    pub level: u8,
    pub ascension: u8,
    pub eidolon: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsrLightCone {
    pub local_id: String,
    pub key: String,
    pub game_id: u32,
    pub name: BilingualName,
    pub rarity: u8,
    pub path: String,
    pub level: u8,
    pub ascension: u8,
    pub superimposition: u8,
    pub location_key: Option<String>,
    pub lock: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportStat {
    pub key: String,
    pub name: BilingualName,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsrRelic {
    #[serde(flatten)]
    pub gear: ExportGear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsrPlanarOrnament {
    #[serde(flatten)]
    pub gear: ExportGear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportGear {
    pub local_id: String,
    pub key: String,
    pub game_id: u32,
    pub name: BilingualName,
    pub set_key: String,
    pub set_name: BilingualName,
    pub rarity: u8,
    pub slot: GearSlot,
    pub level: u8,
    pub main_stat: ExportStat,
    pub substats: Vec<ExportStat>,
    pub location_key: Option<String>,
    pub lock: Option<bool>,
    pub discard: Option<bool>,
}
