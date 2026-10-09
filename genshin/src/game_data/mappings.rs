use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub(crate) struct MappingsFile {
    pub(crate) characters: Vec<CharacterEntry>,
    pub(crate) weapons: Vec<WeaponEntry>,
    #[serde(rename = "artifactSets")]
    pub(crate) artifact_sets: Vec<ArtifactSetEntry>,
}

impl MappingsFile {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.characters.is_empty() || self.weapons.is_empty() || self.artifact_sets.is_empty() {
            bail!("Shared Genshin scanner data has empty OCR mappings");
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct CharacterEntry {
    pub(crate) id: String,
    #[serde(alias = "names")]
    pub(crate) n: LocalizedNames,
    pub(crate) e: Option<String>,
    pub(crate) c3: Option<String>,
    pub(crate) c5: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct WeaponEntry {
    pub(crate) id: String,
    #[serde(alias = "names")]
    pub(crate) n: LocalizedNames,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct ArtifactSetEntry {
    pub(crate) id: String,
    #[serde(alias = "names")]
    pub(crate) n: LocalizedNames,
    #[serde(alias = "rarity")]
    pub(crate) r: Option<i32>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct LocalizedNames {
    pub(crate) zh: Option<String>,
}
