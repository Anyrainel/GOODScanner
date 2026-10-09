use crate::game_data::mappings::MappingsFile;
use anyhow::Result;
use std::collections::HashMap;

/// Constellation bonus info for a character
#[derive(Debug, Clone)]
pub struct ConstBonus {
    /// Which talent gets +3 at C3: "A" (auto), "E" (skill), or "Q" (burst)
    pub c3: Option<String>,
    /// Which talent gets +3 at C5: "A" (auto), "E" (skill), or "Q" (burst)
    pub c5: Option<String>,
}

/// Holds all name→GOOD key mappings loaded from remote/cached data.
///
/// Port of the mapping system from GOODScanner/lib/constants.js and
/// GOODScanner/lib/fetch_mappings.js
#[derive(Debug)]
pub struct MappingManager {
    /// Chinese character name → GOOD character key
    pub character_name_map: HashMap<String, String>,
    /// GOOD character key → canonical element from mappings.json
    pub character_element_map: HashMap<String, String>,
    /// GOOD character key → constellation talent bonus info
    pub character_const_bonus: HashMap<String, ConstBonus>,
    /// Chinese weapon name → GOOD weapon key
    pub weapon_name_map: HashMap<String, String>,
    /// Chinese artifact set name → GOOD set key
    pub artifact_set_map: HashMap<String, String>,
    /// GOOD set key → max rarity (4 or 5)
    pub artifact_set_max_rarity: HashMap<String, i32>,
}

/// Name override config for characters with customizable in-game names
pub struct NameOverrides {
    pub traveler_name: Option<String>,
    pub wanderer_name: Option<String>,
    pub manekin_name: Option<String>,
    pub manekina_name: Option<String>,
}

impl Default for NameOverrides {
    fn default() -> Self {
        Self {
            traveler_name: None,
            wanderer_name: None,
            manekin_name: None,
            manekina_name: None,
        }
    }
}

impl MappingManager {
    /// Fetch mappings if needed (cache expired or missing), then load and initialize.
    ///
    /// Port of `fetchMappingsIfNeeded()` + `initMappings()` from GOODScanner
    pub fn new(overrides: &NameOverrides) -> Result<Self> {
        Ok(Self::from_mappings_data(
            crate::game_data::load()?.mappings,
            overrides,
        ))
    }

    fn from_mappings_data(data: MappingsFile, overrides: &NameOverrides) -> Self {
        let mut character_name_map = HashMap::new();
        let mut character_element_map = HashMap::new();
        let mut character_const_bonus = HashMap::new();

        for entry in &data.characters {
            if let Some(zh_name) = &entry.n.zh {
                character_name_map.insert(zh_name.clone(), entry.id.clone());
            }
            if let Some(element) = &entry.e {
                character_element_map.insert(entry.id.clone(), element.clone());
            }
            if entry.c3.is_some() || entry.c5.is_some() {
                character_const_bonus.insert(
                    entry.id.clone(),
                    ConstBonus {
                        c3: entry.c3.clone(),
                        c5: entry.c5.clone(),
                    },
                );
            }
        }

        let mut weapon_name_map = HashMap::new();
        for entry in &data.weapons {
            if let Some(zh_name) = &entry.n.zh {
                weapon_name_map.insert(zh_name.clone(), entry.id.clone());
            }
        }

        let mut artifact_set_map = HashMap::new();
        let mut artifact_set_max_rarity = HashMap::new();
        for entry in &data.artifact_sets {
            if let Some(zh_name) = &entry.n.zh {
                artifact_set_map.insert(zh_name.clone(), entry.id.clone());
            }
            if let Some(rarity) = entry.r {
                artifact_set_max_rarity.insert(entry.id.clone(), rarity);
            }
        }

        // Apply user name overrides
        let name_overrides: &[(&Option<String>, &str)] = &[
            (&overrides.traveler_name, "Traveler"),
            (&overrides.wanderer_name, "Wanderer"),
            (&overrides.manekin_name, "Manekin"),
            (&overrides.manekina_name, "Manekina"),
        ];

        for (custom_name, id) in name_overrides {
            if let Some(name) = custom_name {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    character_name_map.insert(trimmed.to_string(), id.to_string());
                }
            }
        }

        Self {
            character_name_map,
            character_element_map,
            character_const_bonus,
            weapon_name_map,
            artifact_set_map,
            artifact_set_max_rarity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_mappings(raw: &str, overrides: &NameOverrides) -> MappingManager {
        let data: MappingsFile = serde_json::from_str(raw).unwrap();
        MappingManager::from_mappings_data(data, overrides)
    }

    #[test]
    fn loads_character_elements_from_mapping_cache_shape() {
        let mappings = fixture_mappings(
            r#"{
                "characters": [
                    {"id": "Cyno", "n": {"zh": "赛诺"}, "e": "electro"},
                    {"id": "Aino", "names": {"zh": "爱诺"}, "e": "hydro", "c3": "E"}
                ],
                "weapons": [],
                "artifactSets": []
            }"#,
            &NameOverrides::default(),
        );

        assert_eq!(
            mappings.character_name_map.get("赛诺").map(String::as_str),
            Some("Cyno")
        );
        assert_eq!(
            mappings
                .character_element_map
                .get("Cyno")
                .map(String::as_str),
            Some("electro")
        );
        assert_eq!(
            mappings
                .character_element_map
                .get("Aino")
                .map(String::as_str),
            Some("hydro")
        );
        assert_eq!(
            mappings
                .character_const_bonus
                .get("Aino")
                .and_then(|bonus| bonus.c3.as_deref()),
            Some("E")
        );
    }

    #[test]
    fn name_overrides_do_not_replace_element_metadata() {
        let mappings = fixture_mappings(
            r#"{
                "characters": [
                    {"id": "Traveler", "n": {"zh": "旅行者"}, "e": "anemo"}
                ],
                "weapons": [],
                "artifactSets": []
            }"#,
            &NameOverrides {
                traveler_name: Some("自定义旅行者".to_string()),
                ..NameOverrides::default()
            },
        );

        assert_eq!(
            mappings
                .character_name_map
                .get("自定义旅行者")
                .map(String::as_str),
            Some("Traveler")
        );
        assert_eq!(
            mappings
                .character_element_map
                .get("Traveler")
                .map(String::as_str),
            Some("anemo")
        );
    }
}
