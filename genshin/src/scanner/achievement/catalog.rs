//! Achievement name → id catalog from ggartifact `mapping_achievements.json`.
//!
//! Isolated from `MappingManager` (characters/weapons/sets). Cache path, TTL,
//! and stale-cache fallback match `mappings.json`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use yas::{log_debug, log_info, log_warn};

use crate::scanner::common::fuzzy_match::fuzzy_match_map;

const CATALOG_URL: &str = "https://ggartifact.com/good/mapping_achievements.json";
const CATALOG_CACHE_PATH: &str = "data/mapping_achievements.json";
const CATALOG_META_PATH: &str = "data/mapping_achievements_meta.json";
const CATALOG_TTL_SECS: u64 = 24 * 3600;

/// Punctuation stripped before matching, same set as cocogoat `filter`.
const STRIP_CHARS: &[char] = &['…', '「', '」', '·', '，', '。', '.', '-', '—', ' '];

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct CatalogMeta {
    #[serde(rename = "lastFetchTime")]
    last_fetch_time: u64,
}

#[derive(Debug, Deserialize)]
struct MappingAchievementsFile {
    achievements: Vec<MappingAchievement>,
}

#[derive(Debug, Deserialize)]
struct MappingAchievement {
    id: u32,
    n: LocalizedNames,
}

#[derive(Debug, Deserialize)]
struct LocalizedNames {
    zh: Option<String>,
}

struct CatalogEntry {
    id: u32,
    name: String,
    desc: String,
}

/// Lookup tables used by the OCR matcher (cocogoat `achievementTitles` /
/// `achievementEC` / `achievementSubs`).
#[derive(Debug, Clone)]
pub struct AchievementCatalog {
    /// Unique `title-desc` → id. This is the primary key; titles repeat across stages.
    pub title_desc: HashMap<String, String>,
    /// Title → ids (stringified). Unique titles can short-circuit matching.
    pub titles: HashMap<String, Vec<u32>>,
    /// Description → ids.
    pub descriptions: HashMap<String, Vec<u32>>,
}

impl AchievementCatalog {
    pub fn new() -> Result<Self> {
        fetch_if_needed()?;
        Self::load_from_cache()
    }

    pub fn force_refresh() -> Result<()> {
        crate::fs_utils::remove_file_if_exists(CATALOG_META_PATH)?;
        crate::fs_utils::remove_file_if_exists(CATALOG_CACHE_PATH)?;
        fetch_if_needed()?;
        Ok(())
    }

    pub fn from_entries(entries: &[(u32, &str, &str)]) -> Self {
        let cached: Vec<CatalogEntry> = entries
            .iter()
            .map(|(id, name, desc)| CatalogEntry {
                id: *id,
                name: (*name).to_string(),
                desc: (*desc).to_string(),
            })
            .collect();
        Self::from_cached(&cached)
    }

    fn load_from_cache() -> Result<Self> {
        let raw = fs::read_to_string(CATALOG_CACHE_PATH).with_context(|| {
            format!("achievement catalog cache missing: {CATALOG_CACHE_PATH}")
        })?;
        let data: MappingAchievementsFile = serde_json::from_str(&raw)
            .context("achievement catalog cache is not mapping_achievements.json")?;
        let entries: Vec<CatalogEntry> = data
            .achievements
            .into_iter()
            .filter_map(|a| {
                let name = a.n.zh?;
                if name.is_empty() {
                    None
                } else {
                    Some(CatalogEntry {
                        id: a.id,
                        name,
                        desc: String::new(),
                    })
                }
            })
            .collect();
        if entries.is_empty() {
            bail!("achievement catalog cache has no Chinese names");
        }
        log_info!("已加载 {} 条成就", "Loaded {} achievements", entries.len());
        Ok(Self::from_cached(&entries))
    }

    fn from_cached(entries: &[CatalogEntry]) -> Self {
        let mut title_desc = HashMap::new();
        let mut titles: HashMap<String, Vec<u32>> = HashMap::new();
        let mut descriptions: HashMap<String, Vec<u32>> = HashMap::new();

        for entry in entries {
            let name = normalize_text(&entry.name);
            let desc = normalize_text(&entry.desc);
            if name.is_empty() {
                continue;
            }
            let id = entry.id;
            titles.entry(name.clone()).or_default().push(id);
            if !desc.is_empty() {
                title_desc.insert(format!("{name}-{desc}"), id.to_string());
                descriptions.entry(desc).or_default().push(id);
            }
        }

        Self {
            title_desc,
            titles,
            descriptions,
        }
    }

    pub fn len(&self) -> usize {
        self.titles.values().map(|ids| ids.len()).sum()
    }

    /// Resolve OCR title + subtitle to an achievement id.
    ///
    /// Port of cocogoat `recognizeAchievement` matching, using this repo's
    /// fuzzy matcher instead of the browser `textBestmatch`.
    pub fn match_text(&self, title: &str, subtitle: &str) -> Option<u32> {
        let title = normalize_text(title);
        let subtitle = normalize_text(subtitle);
        if title.is_empty() && subtitle.is_empty() {
            return None;
        }

        if !title.is_empty() && !subtitle.is_empty() {
            if let Some(id) = fuzzy_match_map(&format!("{title}-{subtitle}"), &self.title_desc) {
                return parse_id(&id);
            }
        }

        if !title.is_empty() {
            if let Some(ids) = self.titles.get(&title) {
                if ids.len() == 1 {
                    return Some(ids[0]);
                }
            }
            let title_as_map: HashMap<String, String> = self
                .titles
                .iter()
                .filter(|(_, ids)| ids.len() == 1)
                .map(|(k, ids)| (k.clone(), ids[0].to_string()))
                .collect();
            if let Some(id) = fuzzy_match_map(&title, &title_as_map) {
                return parse_id(&id);
            }
        }

        if !subtitle.is_empty() {
            let desc_map: HashMap<String, String> = self
                .descriptions
                .iter()
                .filter(|(_, ids)| ids.len() == 1)
                .map(|(k, ids)| (k.clone(), ids[0].to_string()))
                .collect();
            if let Some(id) = fuzzy_match_map(&subtitle, &desc_map) {
                return parse_id(&id);
            }
        }

        None
    }
}

pub fn normalize_text(text: &str) -> String {
    text.chars()
        .filter(|c| !STRIP_CHARS.contains(c))
        .collect::<String>()
        .trim()
        .to_string()
}

fn parse_id(s: &str) -> Option<u32> {
    s.parse().ok()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_meta() -> CatalogMeta {
    let content = match fs::read_to_string(CATALOG_META_PATH) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CatalogMeta::default()
        },
        Err(error) => {
            log_warn!(
                "无法读取成就目录缓存状态；将重新检查远程数据。完整错误详情: {:#}",
                "Achievement catalog cache metadata could not be read; remote data will be checked again. Full error details: {:#}",
                error,
            );
            return CatalogMeta::default();
        },
    };
    match serde_json::from_str::<CatalogMeta>(&content) {
        Ok(meta) => meta,
        Err(error) => {
            log_warn!(
                "成就目录缓存状态文件已损坏；将重新检查远程数据。完整错误详情: {:#}",
                "Achievement catalog cache metadata is invalid; remote data will be checked again. Full error details: {:#}",
                error,
            );
            CatalogMeta::default()
        },
    }
}

fn save_meta(meta: &CatalogMeta) -> Result<()> {
    if let Some(parent) = Path::new(CATALOG_META_PATH).parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "achievement catalog cache directory could not be created: {}",
                parent.display()
            )
        })?;
    }
    let json = serde_json::to_string(meta)
        .context("achievement catalog cache metadata serialization failed")?;
    fs::write(CATALOG_META_PATH, json).with_context(|| {
        format!("achievement catalog cache metadata could not be written: {CATALOG_META_PATH}")
    })
}

fn is_fresh(last_fetch_time: u64, ttl_secs: u64) -> bool {
    last_fetch_time > 0 && (now_secs() - last_fetch_time) < ttl_secs
}

fn fetch_if_needed() -> Result<()> {
    let meta = load_meta();
    let cache_exists = Path::new(CATALOG_CACHE_PATH).exists();
    if cache_exists && is_fresh(meta.last_fetch_time, CATALOG_TTL_SECS) {
        return Ok(());
    }

    log_info!("正在获取成就目录...", "Fetching achievement catalog...");

    if let Some(parent) = Path::new(CATALOG_CACHE_PATH).parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "achievement catalog cache directory could not be created: {}",
                parent.display()
            )
        })?;
    }

    match reqwest::blocking::get(CATALOG_URL) {
        Ok(response) => {
            if response.status().is_success() {
                let body = response
                    .text()
                    .context("achievement catalog response body could not be read")?;
                let _: MappingAchievementsFile = serde_json::from_str(&body)
                    .context("downloaded mapping_achievements.json is invalid")?;
                fs::write(CATALOG_CACHE_PATH, &body).with_context(|| {
                    format!("achievement catalog cache could not be written: {CATALOG_CACHE_PATH}")
                })?;
                save_meta(&CatalogMeta {
                    last_fetch_time: now_secs(),
                })?;
                log_debug!("成就目录已更新", "Achievement catalog updated");
            } else if cache_exists {
                log_warn!(
                    "获取数据失败 (HTTP {})，使用本地缓存",
                    "Fetch failed (HTTP {}), using local cache",
                    response.status()
                );
            } else {
                bail!(
                    "获取成就目录失败 (HTTP {})，且无本地缓存。请检查网络连接。\n\
                     / Failed to fetch achievement catalog (HTTP {}), no local cache. Check your network connection.",
                    response.status(),
                    response.status()
                );
            }
        },
        Err(e) => {
            if cache_exists {
                log_warn!(
                    "无法下载最新成就目录；将使用本地缓存。完整错误详情: {:#}",
                    "The latest achievement catalog could not be downloaded; the local cache will be used. Full error details: {:#}",
                    e
                );
            } else {
                return Err(e).context(format!(
                    "achievement catalog could not be downloaded and no local cache exists; source: {CATALOG_URL}"
                ));
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AchievementCatalog {
        AchievementCatalog::from_entries(&[
            (
                80127,
                "动物园大亨",
                "使用道具「四方八方之网」捕获1只野外生物。",
            ),
            (
                80128,
                "动物园大亨",
                "使用道具「四方八方之网」捕获30只野外生物。",
            ),
            (81000, "俯瞰风景", "抵达庆云顶的制高点。"),
        ])
    }

    #[test]
    fn unique_title_matches_without_subtitle() {
        let cat = sample();
        assert_eq!(cat.match_text("俯瞰风景", ""), Some(81000));
    }

    #[test]
    fn staged_title_needs_subtitle() {
        let cat = sample();
        assert_eq!(
            cat.match_text("动物园大亨", "使用道具四方八方之网捕获30只野外生物"),
            Some(80128)
        );
        assert_eq!(
            cat.match_text("动物园大亨", "使用道具四方八方之网捕获1只野外生物"),
            Some(80127)
        );
    }

    #[test]
    fn ggartifact_title_only_skips_duplicate_names() {
        let raw = r#"{"achievements":[
            {"id":81000,"n":{"zh":"俯瞰风景"}},
            {"id":80127,"n":{"zh":"动物园大亨"}},
            {"id":80128,"n":{"zh":"动物园大亨"}}
        ]}"#;
        let data: MappingAchievementsFile = serde_json::from_str(raw).unwrap();
        let entries: Vec<CatalogEntry> = data
            .achievements
            .into_iter()
            .map(|a| CatalogEntry {
                id: a.id,
                name: a.n.zh.unwrap(),
                desc: String::new(),
            })
            .collect();
        let cat = AchievementCatalog::from_cached(&entries);
        assert_eq!(cat.match_text("俯瞰风景", ""), Some(81000));
        assert_eq!(cat.match_text("动物园大亨", ""), None);
    }

    #[test]
    fn normalize_strips_cocogoat_punctuation() {
        assert_eq!(normalize_text("「走吧，伙伴」……"), "走吧伙伴");
    }

    #[test]
    fn parses_ggartifact_mapping_shape() {
        let raw = r#"{"achievements":[{"id":81000,"n":{"zh":"俯瞰风景"}}]}"#;
        let data: MappingAchievementsFile = serde_json::from_str(raw).unwrap();
        assert_eq!(data.achievements[0].id, 81000);
        assert_eq!(data.achievements[0].n.zh.as_deref(), Some("俯瞰风景"));
    }
}
