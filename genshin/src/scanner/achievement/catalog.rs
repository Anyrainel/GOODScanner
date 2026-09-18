//! Achievement name/description → id catalog.
//!
//! Isolated from `MappingManager` (characters/weapons/sets). Fetches the
//! Chinese achievement JSON files from dvaJi/genshin-data, the same family of
//! tables cocogoat matches OCR text against.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use yas::{log_debug, log_info, log_warn};

use crate::scanner::common::fuzzy_match::fuzzy_match_map;

const CATALOG_URL: &str =
    "https://api.github.com/repos/dvaJi/genshin-data/contents/src/data/chinese-simplified/achievements";
const CATALOG_CACHE_PATH: &str = "data/achievements.json";
const CATALOG_META_PATH: &str = "data/achievements_meta.json";
const CATALOG_TTL_SECS: u64 = 24 * 3600;

/// Punctuation stripped before matching, same set as cocogoat `filter`.
const STRIP_CHARS: &[char] = &['…', '「', '」', '·', '，', '。', '.', '-', '—', ' '];

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct CatalogMeta {
    #[serde(rename = "lastFetchTime")]
    last_fetch_time: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CachedCatalog {
    achievements: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CatalogEntry {
    id: u32,
    name: String,
    desc: String,
}

#[derive(Debug, Deserialize)]
struct GitHubContent {
    name: String,
    download_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteCategoryFile {
    achievements: Vec<RemoteAchievement>,
}

#[derive(Debug, Deserialize)]
struct RemoteAchievement {
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
        let raw = fs::read_to_string(CATALOG_CACHE_PATH)
            .with_context(|| format!("achievement catalog cache missing: {CATALOG_CACHE_PATH}"))?;
        let cached: CachedCatalog =
            serde_json::from_str(&raw).context("achievement catalog cache is invalid JSON")?;
        Ok(Self::from_cached(&cached.achievements))
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
            title_desc.insert(format!("{name}-{desc}"), id.to_string());
            titles.entry(name).or_default().push(id);
            if !desc.is_empty() {
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
        self.title_desc.len()
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
                if ids.len() == 1 && !needs_subtitle_disambiguation(&subtitle) {
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
                if !needs_subtitle_disambiguation(&subtitle) {
                    return parse_id(&id);
                }
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

/// True when the description is needed to pick among staged achievements
/// (cocogoat: desc contains digits or `次`).
fn needs_subtitle_disambiguation(subtitle: &str) -> bool {
    !subtitle.is_empty()
        && (subtitle.chars().any(|c| c.is_ascii_digit()) || subtitle.contains('次'))
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
    match fs::read_to_string(CATALOG_META_PATH) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => CatalogMeta::default(),
    }
}

fn save_meta(meta: &CatalogMeta) -> Result<()> {
    if let Some(parent) = Path::new(CATALOG_META_PATH).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(CATALOG_META_PATH, serde_json::to_string(meta)?)?;
    Ok(())
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

    match download_catalog() {
        Ok(entries) => {
            if let Some(parent) = Path::new(CATALOG_CACHE_PATH).parent() {
                fs::create_dir_all(parent)?;
            }
            let json = serde_json::to_string(&CachedCatalog {
                achievements: entries,
            })?;
            fs::write(CATALOG_CACHE_PATH, json)?;
            save_meta(&CatalogMeta {
                last_fetch_time: now_secs(),
            })?;
            log_debug!("成就目录已更新", "Achievement catalog updated");
            Ok(())
        },
        Err(e) => {
            if cache_exists {
                log_warn!(
                    "无法下载最新成就目录；将使用本地缓存。完整错误详情: {:#}",
                    "The latest achievement catalog could not be downloaded; the local cache will be used. Full error details: {:#}",
                    e
                );
                Ok(())
            } else {
                Err(e).context(
                    "achievement catalog could not be downloaded and no local cache exists",
                )
            }
        },
    }
}

fn http_get(url: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("GOODScanner/achievement-catalog")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("HTTP client for achievement catalog could not be created")?;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("achievement catalog request failed: {url}"))?;
    if !response.status().is_success() {
        bail!(
            "achievement catalog fetch failed (HTTP {}) from {url}",
            response.status()
        );
    }
    response
        .text()
        .with_context(|| format!("achievement catalog response body could not be read: {url}"))
}

fn download_catalog() -> Result<Vec<CatalogEntry>> {
    let listing_body = http_get(CATALOG_URL)?;
    let listing: Vec<GitHubContent> =
        serde_json::from_str(&listing_body).context("GitHub achievement listing is not JSON")?;

    let mut entries = Vec::new();
    for item in listing {
        if !item.name.ends_with(".json") {
            continue;
        }
        let Some(url) = item.download_url else {
            continue;
        };
        let body = http_get(&url)?;
        let file: RemoteCategoryFile = serde_json::from_str(&body)
            .with_context(|| format!("achievement file is not JSON: {}", item.name))?;
        for a in file.achievements {
            entries.push(CatalogEntry {
                id: a.id,
                name: a.name,
                desc: a.desc,
            });
        }
    }

    if entries.is_empty() {
        bail!("downloaded achievement catalog is empty");
    }
    log_info!("已加载 {} 条成就", "Loaded {} achievements", entries.len());
    Ok(entries)
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
    fn normalize_strips_cocogoat_punctuation() {
        assert_eq!(normalize_text("「走吧，伙伴」……"), "走吧伙伴");
    }
}
