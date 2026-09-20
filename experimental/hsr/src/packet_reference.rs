use crate::{reference::ReferenceCache, HsrError, HsrResult, LocalizedText};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MainAffix {
    pub group: u32,
    pub id: u32,
    pub property: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubAffix {
    pub group: u32,
    pub id: u32,
    pub property: String,
    pub base: f64,
    pub step: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PacketReferences {
    pub source_revision: String,
    pub main: Vec<MainAffix>,
    pub sub: Vec<SubAffix>,
    pub base_avatars: BTreeMap<u32, u32>,
}

impl PacketReferences {
    pub fn validate(&self, references: &ReferenceCache) -> HsrResult<()> {
        let invalid = || {
            HsrError::new(
                "HSR-REF-PACKET",
                LocalizedText::new(
                    "星穹铁道游戏数据不完整，请刷新游戏数据后重试。",
                    "Star Rail game data is incomplete. Refresh game data and retry.",
                ),
                "missing, duplicate, or invalid packet reference joins",
            )
        };
        if self.main.is_empty() || self.sub.is_empty() || self.base_avatars.is_empty() {
            return Err(invalid());
        }
        let mut keys = BTreeSet::new();
        for a in &self.main {
            if a.id == 0 || !keys.insert((a.group, a.id)) || references.stat(&a.property).is_none()
            {
                return Err(invalid());
            }
        }
        keys.clear();
        for a in &self.sub {
            if a.id == 0
                || !keys.insert((a.group, a.id))
                || references.stat(&a.property).is_none()
                || !a.base.is_finite()
                || a.base <= 0.0
                || !a.step.is_finite()
                || a.step < 0.0
            {
                return Err(invalid());
            }
        }
        for (id, base) in &self.base_avatars {
            if references.character(*id).is_none() || references.character(*base).is_none() {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
