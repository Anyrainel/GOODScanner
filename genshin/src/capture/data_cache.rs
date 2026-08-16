/// Downloads and caches `data_cache.json` from ggartifact.com.
///
/// Uses `data/data_cache_meta.json` for cache freshness tracking.
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use yas::{log_info, log_warn};

use super::data_types::DataCache;

const DATA_CACHE_URL: &str = "https://ggartifact.com/good/data_cache.json";
// The 7.0 data refresh currently publishes empty artifact_map/set_map objects.
// This commit is the last complete catalog and is immutable, so it can safely
// repair only those missing tables while current character/weapon/stat data is
// retained from DATA_CACHE_URL.
const ARTIFACT_CATALOG_FALLBACK_URL: &str = "https://raw.githubusercontent.com/Anyrainel/GenshinTools/a2a851f0cde65494a9699c86a5d6eeca8eb516ea/public/good/data_cache.json";
const DATA_CACHE_PATH: &str = "data/data_cache.json";
const DATA_CACHE_META_PATH: &str = "data/data_cache_meta.json";
const DATA_CACHE_TTL_SECS: u64 = 2 * 3600;
const MIN_VALID_ARTIFACT_ENTRIES: usize = 100;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct CacheMeta {
    #[serde(rename = "lastFetchTime")]
    last_fetch_time: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_meta() -> CacheMeta {
    if let Ok(content) = fs::read_to_string(DATA_CACHE_META_PATH) {
        if let Ok(meta) = serde_json::from_str::<CacheMeta>(&content) {
            return meta;
        }
    }
    CacheMeta::default()
}

fn write_meta(meta: &CacheMeta) {
    if let Some(parent) = Path::new(DATA_CACHE_META_PATH).parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(meta) {
        let _ = fs::write(DATA_CACHE_META_PATH, json);
    }
}

/// Delete cached files and re-download immediately.
pub fn force_refresh() -> Result<()> {
    let _ = fs::remove_file(DATA_CACHE_META_PATH);
    let _ = fs::remove_file(DATA_CACHE_PATH);
    load_data_cache().map(|_| ())
}

fn is_cache_fresh(last_fetch_time: u64, ttl_secs: u64) -> bool {
    last_fetch_time > 0 && (now_secs() - last_fetch_time) < ttl_secs
}

/// Fetch `data_cache.json` from remote if cache is stale, otherwise load from cache.
/// Returns the parsed `DataCache`.
pub fn load_data_cache() -> Result<DataCache> {
    fs::create_dir_all("data").ok();

    let cache_path = Path::new(DATA_CACHE_PATH);
    let meta = load_meta();

    if !cache_path.exists() || !is_cache_fresh(meta.last_fetch_time, DATA_CACHE_TTL_SECS) {
        log_info!(
            "正在下载抓包数据缓存...",
            "Downloading capture data cache..."
        );
        match fetch_remote() {
            Ok(data) => {
                // Validate and repair the remote cache before writing it. This
                // prevents a valid-looking empty artifact catalog from
                // replacing a usable local cache.
                let mut data_cache: DataCache = serde_json::from_str(&data)
                    .context("Failed to parse fetched data_cache.json")?;
                ensure_artifact_catalog(&mut data_cache)?;
                fs::write(cache_path, serde_json::to_vec(&data_cache)?)?;
                write_meta(&CacheMeta {
                    last_fetch_time: now_secs(),
                });
                log_info!("抓包数据缓存已更新", "Capture data cache updated");
            },
            Err(e) => {
                if cache_path.exists() {
                    log_warn!(
                        "下载抓包数据缓存失败（{}），使用本地缓存",
                        "Failed to fetch data cache ({}), using stale cache",
                        e
                    );
                } else {
                    anyhow::bail!(
                        "下载抓包数据缓存失败且无本地缓存 / Failed to fetch data cache and no local cache exists: {}",
                        e
                    );
                }
            },
        }
    }

    let content = fs::read_to_string(cache_path).context("Failed to read data_cache.json")?;
    let mut data_cache: DataCache =
        serde_json::from_str(&content).context("Failed to parse data_cache.json")?;
    if ensure_artifact_catalog(&mut data_cache)? {
        fs::write(cache_path, serde_json::to_vec(&data_cache)?)?;
    }
    Ok(data_cache)
}

fn fetch_remote() -> Result<String> {
    fetch_url(DATA_CACHE_URL)
}

fn fetch_url(url: &str) -> Result<String> {
    let resp =
        reqwest::blocking::get(url).with_context(|| format!("HTTP request to {url} failed"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {} from {}", status, url);
    }
    resp.text().context("Failed to read response body")
}

/// Repair a missing artifact catalog without replacing fresher cache tables.
///
/// Returns true when fallback entries were merged into `data_cache`.
fn ensure_artifact_catalog(data_cache: &mut DataCache) -> Result<bool> {
    if data_cache.artifact_map.len() >= MIN_VALID_ARTIFACT_ENTRIES && !data_cache.set_map.is_empty()
    {
        return Ok(false);
    }

    log_warn!(
        "抓包数据缓存缺少圣遗物目录（{} 件，{} 套），正在加载兼容目录",
        "Capture data cache is missing its artifact catalog ({} items, {} sets); loading compatibility catalog",
        data_cache.artifact_map.len(),
        data_cache.set_map.len(),
    );
    let fallback_json = fetch_url(ARTIFACT_CATALOG_FALLBACK_URL)
        .context("Failed to fetch compatibility artifact catalog")?;
    let fallback: DataCache = serde_json::from_str(&fallback_json)
        .context("Failed to parse compatibility artifact catalog")?;
    merge_artifact_catalog(data_cache, fallback)?;
    log_info!(
        "已加载兼容圣遗物目录（{} 件，{} 套）",
        "Loaded compatibility artifact catalog ({} items, {} sets)",
        data_cache.artifact_map.len(),
        data_cache.set_map.len(),
    );
    Ok(true)
}

fn merge_artifact_catalog(data_cache: &mut DataCache, fallback: DataCache) -> Result<()> {
    if fallback.artifact_map.len() < MIN_VALID_ARTIFACT_ENTRIES || fallback.set_map.is_empty() {
        anyhow::bail!(
            "Compatibility artifact catalog is incomplete ({} items, {} sets)",
            fallback.artifact_map.len(),
            fallback.set_map.len()
        );
    }

    // Current entries win so this remains forward-compatible when the primary
    // provider starts publishing artifact data again.
    for (id, artifact) in fallback.artifact_map {
        data_cache.artifact_map.entry(id).or_insert(artifact);
    }
    for (id, set_name) in fallback.set_map {
        data_cache.set_map.entry(id).or_insert(set_name);
    }

    if data_cache.artifact_map.len() < MIN_VALID_ARTIFACT_ENTRIES || data_cache.set_map.is_empty() {
        anyhow::bail!("Artifact catalog remains incomplete after compatibility merge");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::capture::data_types::{Artifact, ArtifactSlot};

    fn empty_cache() -> DataCache {
        DataCache {
            version: 1,
            git_hash: String::new(),
            affix_map: HashMap::new(),
            artifact_map: HashMap::new(),
            character_map: HashMap::new(),
            material_map: HashMap::new(),
            property_map: HashMap::new(),
            set_map: HashMap::new(),
            skill_type_map: HashMap::new(),
            weapon_map: HashMap::new(),
        }
    }

    fn artifact(set: &str) -> Artifact {
        Artifact {
            set: set.to_string(),
            slot: ArtifactSlot::Flower,
            rarity: 5,
        }
    }

    #[test]
    fn compatibility_catalog_repairs_empty_maps_and_preserves_current_entries() {
        let mut current = empty_cache();
        current.artifact_map.insert(1, artifact("Current Set"));
        current.set_map.insert(1, "Current Set".to_string());

        let mut fallback = empty_cache();
        for id in 1..=MIN_VALID_ARTIFACT_ENTRIES as u32 {
            fallback.artifact_map.insert(id, artifact("Fallback Set"));
        }
        fallback.set_map.insert(1, "Fallback Set".to_string());
        fallback.set_map.insert(2, "Older Set".to_string());

        merge_artifact_catalog(&mut current, fallback).unwrap();

        assert_eq!(
            current.artifact_map.get(&1).unwrap().set,
            "Current Set",
            "newer primary-cache entries must win"
        );
        assert_eq!(current.artifact_map.len(), MIN_VALID_ARTIFACT_ENTRIES);
        assert_eq!(current.set_map.get(&1).unwrap(), "Current Set");
        assert_eq!(current.set_map.get(&2).unwrap(), "Older Set");
    }

    #[test]
    fn compatibility_catalog_rejects_another_empty_snapshot() {
        let mut current = empty_cache();
        let error = merge_artifact_catalog(&mut current, empty_cache()).unwrap_err();
        assert!(error.to_string().contains("incomplete"));
    }
}
