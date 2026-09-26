//! Achievement name → id catalog from ggartifact `mapping_achievements.json`.
//!
//! Isolated from `MappingManager` (characters/weapons/sets). Cache path, TTL,
//! and stale-cache fallback match `mappings.json`.
//!
//! Current file shape is `{categories:[{id,n:{zh},achievements:[{id,n:{zh}}]}]}`.
//! The older flat `{achievements:[...]}` list is still accepted.

use std::collections::{HashMap, HashSet};
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
    #[serde(default)]
    categories: Vec<MappingCategory>,
    #[serde(default)]
    achievements: Vec<MappingAchievement>,
}

#[derive(Debug, Deserialize)]
struct MappingCategory {
    #[serde(default)]
    id: u32,
    n: LocalizedNames,
    #[serde(default)]
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

/// One in-game left-list category and the titles it contains.
#[derive(Debug, Clone)]
pub struct CatalogCategory {
    pub id: u32,
    pub name: String,
    /// Catalog order: `(id, original zh title)`.
    pub entries: Vec<(u32, String)>,
    titles: HashMap<String, Vec<u32>>,
}

impl CatalogCategory {
    pub fn expected_unique_titles(&self) -> usize {
        self.titles.len()
    }

    pub fn original_titles(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (_, name) in &self.entries {
            let key = normalize_text(name);
            if seen.insert(key) {
                out.push(name.clone());
            }
        }
        out
    }
}

/// How many of a category's catalog titles were recognized this scan.
#[derive(Debug, Clone)]
pub struct CategoryCoverage {
    pub name: String,
    pub expected: usize,
    pub matched: usize,
    pub missing: Vec<String>,
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
    pub categories: Vec<CatalogCategory>,
    category_index: HashMap<String, usize>,
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
        Self::from_cached(&cached, Vec::new())
    }

    fn load_from_cache() -> Result<Self> {
        let raw = fs::read_to_string(CATALOG_CACHE_PATH).with_context(|| {
            format!("achievement catalog cache missing: {CATALOG_CACHE_PATH}")
        })?;
        let data: MappingAchievementsFile = serde_json::from_str(&raw)
            .context("achievement catalog cache is not mapping_achievements.json")?;
        Self::from_mapping_file(data)
    }

    fn from_mapping_file(data: MappingAchievementsFile) -> Result<Self> {
        if !data.categories.is_empty() {
            let mut categories = Vec::new();
            let mut flat = Vec::new();
            for cat in data.categories {
                let Some(name) = cat.n.zh.filter(|s| !s.is_empty()) else {
                    continue;
                };
                let mut titles: HashMap<String, Vec<u32>> = HashMap::new();
                let mut entries = Vec::new();
                for a in cat.achievements {
                    let Some(title) = a.n.zh.filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    let key = normalize_text(&title);
                    titles.entry(key).or_default().push(a.id);
                    entries.push((a.id, title.clone()));
                    flat.push(CatalogEntry {
                        id: a.id,
                        name: title,
                        desc: String::new(),
                    });
                }
                categories.push(CatalogCategory {
                    id: cat.id,
                    name,
                    entries,
                    titles,
                });
            }
            if flat.is_empty() {
                bail!("achievement catalog cache has no Chinese names");
            }
            log_info!(
                "已加载 {} 个分类、{} 条成就",
                "Loaded {} categories, {} achievements",
                categories.len(),
                flat.len()
            );
            return Ok(Self::from_cached(&flat, categories));
        }

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
        Ok(Self::from_cached(&entries, Vec::new()))
    }

    fn from_cached(entries: &[CatalogEntry], categories: Vec<CatalogCategory>) -> Self {
        let mut title_desc = HashMap::new();
        let mut titles: HashMap<String, Vec<u32>> = HashMap::new();
        let mut descriptions: HashMap<String, Vec<u32>> = HashMap::new();
        let mut category_index = HashMap::new();

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
        for (i, cat) in categories.iter().enumerate() {
            let key = normalize_text(&cat.name);
            if !key.is_empty() {
                category_index.insert(key, i);
            }
        }

        Self {
            title_desc,
            titles,
            descriptions,
            categories,
            category_index,
        }
    }

    pub fn resolve_category(&self, name: &str) -> Option<&CatalogCategory> {
        let key = normalize_text(name);
        if key.is_empty() {
            return None;
        }
        if let Some(i) = self.category_index.get(&key) {
            return self.categories.get(*i);
        }
        let name_map: HashMap<String, String> = self
            .category_index
            .keys()
            .map(|k| (k.clone(), k.clone()))
            .collect();
        let matched = fuzzy_match_map(&key, &name_map)?;
        let i = self.category_index.get(&matched)?;
        self.categories.get(*i)
    }

    /// Catalog titles in this category that no OCR title matched.
    pub fn coverage(&self, category: &str, seen_titles: &[String]) -> Option<CategoryCoverage> {
        let cat = self.resolve_category(category)?;
        let mut matched_keys = HashSet::new();
        for title in seen_titles {
            for id in self.match_title_ids_in(title, Some(cat)) {
                if let Some((_, name)) = cat.entries.iter().find(|(eid, _)| *eid == id) {
                    matched_keys.insert(normalize_text(name));
                }
            }
        }
        let mut missing: Vec<String> = Vec::new();
        let mut missing_keys: HashSet<String> = HashSet::new();
        for (_, name) in &cat.entries {
            let key = normalize_text(name);
            if matched_keys.contains(&key) || !missing_keys.insert(key) {
                continue;
            }
            missing.push(name.clone());
        }
        Some(CategoryCoverage {
            name: cat.name.clone(),
            expected: cat.expected_unique_titles(),
            matched: matched_keys.len(),
            missing,
        })
    }

    pub fn ids_for_title(&self, title: &str) -> Vec<u32> {
        self.match_title_ids(title)
    }

    pub fn title_id_count(&self, title: &str) -> usize {
        self.ids_for_title(title).len()
    }

    pub fn title_id_count_in(&self, title: &str, category: Option<&str>) -> usize {
        let cat = category.and_then(|n| self.resolve_category(n));
        self.match_title_ids_in(title, cat).len()
    }

    pub fn len(&self) -> usize {
        self.titles.values().map(|ids| ids.len()).sum()
    }

    /// Resolve OCR title + subtitle to an achievement id.
    pub fn match_text(&self, title: &str, subtitle: &str) -> Option<u32> {
        self.match_text_in_category(title, subtitle, None)
    }

    pub fn match_text_in_category(
        &self,
        title: &str,
        subtitle: &str,
        category: Option<&str>,
    ) -> Option<u32> {
        let title = normalize_text(title);
        let subtitle = normalize_text(subtitle);
        if title.is_empty() && subtitle.is_empty() {
            return None;
        }
        let cat = category.and_then(|n| self.resolve_category(n));

        if !title.is_empty() && !subtitle.is_empty() {
            if let Some(id) = fuzzy_match_map(&format!("{title}-{subtitle}"), &self.title_desc) {
                return parse_id(&id);
            }
        }

        if !title.is_empty() {
            let ids = self.match_title_ids_in(&title, cat);
            if ids.len() == 1 {
                return Some(ids[0]);
            }
        }

        if !subtitle.is_empty() {
            let ids = self.match_title_ids_in(&subtitle, cat);
            if ids.len() == 1 {
                return Some(ids[0]);
            }
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

    /// Exact then fuzzy lookup of every catalog title, including staged groups.
    pub fn match_title_ids(&self, title: &str) -> Vec<u32> {
        self.match_title_ids_in(title, None)
    }

    fn match_title_ids_in(&self, title: &str, category: Option<&CatalogCategory>) -> Vec<u32> {
        let title = normalize_text(title);
        if title.is_empty() {
            return Vec::new();
        }
        let table = category.map(|c| &c.titles).unwrap_or(&self.titles);
        if let Some(ids) = table.get(&title) {
            let mut ids = ids.clone();
            ids.sort_unstable();
            return ids;
        }
        let title_as_map: HashMap<String, String> =
            table.keys().map(|k| (k.clone(), k.clone())).collect();
        if let Some(matched) = fuzzy_match_map(&title, &title_as_map) {
            let mut ids = table.get(&matched).cloned().unwrap_or_default();
            ids.sort_unstable();
            return ids;
        }
        Vec::new()
    }

    /// IDs for the card on screen. A series only shows its latest stage, and
    /// the catalog stores that series under the `·其一` title.
    pub fn ids_for_shown_title(
        &self,
        title: &str,
        category: Option<&str>,
    ) -> (Vec<u32>, Option<usize>) {
        let stage = shown_stage(title);
        let mut probes = vec![normalize_text(title)];
        if let Some(rewritten) = as_first_stage(title) {
            if rewritten != probes[0] {
                probes.push(rewritten);
            }
        }
        let cat = category.and_then(|name| self.resolve_category(name));
        for probe in probes {
            if probe.is_empty() {
                continue;
            }
            let mut ids = self.match_title_ids_in(&probe, cat);
            if ids.is_empty() && cat.is_some() {
                ids = self.match_title_ids_in(&probe, None);
            }
            if !ids.is_empty() {
                return (ids, stage);
            }
        }
        (Vec::new(), stage)
    }

    /// IDs completed by the card on screen.
    ///
    /// Later `其N` names hide the earlier ones. `冻原的枪匠·其三` therefore
    /// completes `·其一` and `·其二` even when `·其三` itself is not in the catalog.
    pub fn completed_ids_for_shown(&self, title: &str, done: bool) -> Vec<u32> {
        let shown = shown_stage(title);
        let stem = stage_stem(title);
        if let Some(n) = shown {
            if !stem.is_empty() {
                let mut out = Vec::new();
                for (name, ids) in &self.titles {
                    let Some(m) = shown_stage(name) else {
                        continue;
                    };
                    if stage_stem(name) != stem {
                        continue;
                    }
                    if m < n || (m == n && done) {
                        out.extend(ids.iter().copied());
                    }
                }
                out.sort_unstable();
                out.dedup();
                if !out.is_empty() {
                    return out;
                }
            }
        }
        let (ids, stage) = self.ids_for_shown_title(title, None);
        credit_completed(&ids, stage, done)
    }
}

/// How many IDs stored under one catalog title count as completed.
///
/// A done card counts the whole group. The `其一` / `其二` suffix is the
/// achievement's name, not the number of tiers: the list still says `其一`
/// after every tier of that name is done. A card that is not done counts
/// only the tiers before the one on screen.
pub fn credit_completed(ids: &[u32], stage: Option<usize>, done: bool) -> Vec<u32> {
    if ids.is_empty() {
        return Vec::new();
    }
    if done {
        return ids.to_vec();
    }
    let Some(k) = stage else {
        return Vec::new();
    };
    if !(1..=ids.len()).contains(&k) || k <= 1 {
        return Vec::new();
    }
    ids.iter().copied().take(k - 1).collect()
}

fn shown_stage(title: &str) -> Option<usize> {
    let chars: Vec<char> = normalize_text(title).chars().collect();
    if chars.len() < 2 || chars[chars.len() - 2] != '其' {
        return None;
    }
    stage_number(chars[chars.len() - 1])
}

fn as_first_stage(title: &str) -> Option<String> {
    let mut chars: Vec<char> = normalize_text(title).chars().collect();
    if chars.len() < 2 || chars[chars.len() - 2] != '其' {
        return None;
    }
    if stage_number(*chars.last()?)? == 1 {
        return None;
    }
    *chars.last_mut()? = '一';
    Some(chars.into_iter().collect())
}

fn stage_stem(title: &str) -> String {
    let chars: Vec<char> = normalize_text(title).chars().collect();
    if chars.len() >= 2 && chars[chars.len() - 2] == '其' && stage_number(chars[chars.len() - 1]).is_some()
    {
        return chars[..chars.len() - 2].iter().collect();
    }
    chars.into_iter().collect()
}

fn stage_number(c: char) -> Option<usize> {
    "一二三四五六七八九十".chars().position(|s| s == c).map(|i| i + 1)
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

fn cache_has_categories() -> bool {
    fs::read_to_string(CATALOG_CACHE_PATH)
        .map(|raw| raw.contains("\"categories\""))
        .unwrap_or(false)
}

fn fetch_if_needed() -> Result<()> {
    let meta = load_meta();
    let cache_exists = Path::new(CATALOG_CACHE_PATH).exists();
    if cache_exists && is_fresh(meta.last_fetch_time, CATALOG_TTL_SECS) && cache_has_categories() {
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
                let parsed: MappingAchievementsFile = serde_json::from_str(&body)
                    .context("downloaded mapping_achievements.json is invalid")?;
                if parsed.categories.is_empty() && parsed.achievements.is_empty() {
                    bail!("downloaded mapping_achievements.json has no achievements");
                }
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
        let cat = AchievementCatalog::from_mapping_file(data).unwrap();
        assert_eq!(cat.match_text("俯瞰风景", ""), Some(81000));
        assert_eq!(cat.match_text("动物园大亨", ""), None);
        assert_eq!(cat.ids_for_title("动物园大亨"), vec![80127, 80128]);
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

    #[test]
    fn parses_category_grouped_mapping() {
        let raw = r#"{"categories":[
            {"id":4,"n":{"zh":"蒙德·风与牧歌的城邦"},"achievements":[
                {"id":80011,"n":{"zh":"大地勘探·蒙德"}},
                {"id":80012,"n":{"zh":"千风拂去"}}
            ]},
            {"id":0,"n":{"zh":"天地万象"},"achievements":[
                {"id":80127,"n":{"zh":"动物园大亨"}},
                {"id":80128,"n":{"zh":"动物园大亨"}}
            ]}
        ]}"#;
        let data: MappingAchievementsFile = serde_json::from_str(raw).unwrap();
        let cat = AchievementCatalog::from_mapping_file(data).unwrap();
        assert_eq!(cat.categories.len(), 2);
        assert_eq!(cat.len(), 4);
        let mond = cat.resolve_category("蒙德风与牧歌的城邦").unwrap();
        assert_eq!(mond.expected_unique_titles(), 2);
        assert_eq!(
            cat.match_text_in_category("大地勘探蒙德", "", Some("蒙德·风与牧歌的城邦")),
            Some(80011)
        );
        assert_eq!(
            cat.match_text_in_category("动物园大亨", "", Some("天地万象")),
            None
        );
        let cov = cat
            .coverage("蒙德·风与牧歌的城邦", &["大地勘探·蒙德".into()])
            .unwrap();
        assert_eq!(cov.expected, 2);
        assert_eq!(cov.matched, 1);
        assert_eq!(cov.missing, vec!["千风拂去".to_string()]);
    }

    #[test]
    fn finished_series_credits_every_earlier_stage() {
        let cat = AchievementCatalog::from_entries(&[
            (80079, "鸣草丛中的捕手·其一", ""),
            (80080, "鸣草丛中的捕手·其一", ""),
            (80081, "鸣草丛中的捕手·其一", ""),
            (81000, "俯瞰风景", ""),
        ]);
        let (ids, stage) = cat.ids_for_shown_title("鸣草丛中的捕手·其一", None);
        assert_eq!(stage, Some(1));
        assert_eq!(
            cat.completed_ids_for_shown("鸣草丛中的捕手·其一", true),
            vec![80079, 80080, 80081]
        );
        assert_eq!(credit_completed(&ids, stage, false), Vec::<u32>::new());
        let gun = AchievementCatalog::from_entries(&[
            (86067, "冻原的枪匠·其一", ""),
            (86068, "冻原的枪匠·其二", ""),
        ]);
        assert_eq!(
            gun.completed_ids_for_shown("冻原的枪匠·其三", false),
            vec![86067, 86068]
        );
        assert_eq!(
            credit_completed(&[80127, 80128, 80129], None, true),
            vec![80127, 80128, 80129]
        );
        assert!(credit_completed(&[81000], None, false).is_empty());
    }
}
