//! Shared inventory reference for OCR, capture and manager; achievements load separately.
pub(crate) mod mappings;
pub mod types;

use self::mappings::MappingsFile;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub const DATA_URL: &str = "https://ggartifact.com/good/genshin_scanner_data.json";
const CACHE_PATH: &str = "data/genshin_scanner_data.json";
const TTL: u64 = 2 * 3600;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScannerData {
    pub format_version: u32,
    pub source_revision: String,
    pub(crate) mappings: MappingsFile,
    pub capture: types::DataCache,
}

impl ScannerData {
    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1
            || self.source_revision.is_empty()
            || self.source_revision != self.capture.git_hash
            || self.capture.version != 1
        {
            bail!("Invalid shared Genshin scanner data format or source revision");
        }
        self.mappings.validate()?;
        let data = &self.capture;
        if data.artifact_map.len() < 100
            || data.set_map.is_empty()
            || data.character_map.is_empty()
            || data.weapon_map.is_empty()
            || data.affix_map.is_empty()
            || data.property_map.is_empty()
            || data.skill_type_map.is_empty()
        {
            bail!("Shared Genshin scanner data has an incomplete capture catalog");
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiskCache {
    fetched_at: u64,
    data: ScannerData,
}

pub fn load() -> Result<ScannerData> {
    load_from_url(Path::new(CACHE_PATH), DATA_URL, false)
}

pub fn force_refresh() -> Result<()> {
    load_from_url(Path::new(CACHE_PATH), DATA_URL, true).map(|_| ())
}

pub fn cache_updated_at() -> Option<u64> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Metadata {
        fetched_at: u64,
    }
    let file = fs::File::open(CACHE_PATH).ok()?;
    let metadata: Metadata = serde_json::from_reader(std::io::BufReader::new(file)).ok()?;
    (metadata.fetched_at > 0).then_some(metadata.fetched_at)
}

// Remove obsolete mappings/capture caches without touching user data.
pub fn load_from_url(path: &Path, url: &str, force: bool) -> Result<ScannerData> {
    let _guard = LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(root) = path.parent() {
        for filename in [
            "mappings.json",
            "mappings_meta.json",
            "data_cache.json",
            "data_cache_meta.json",
        ] {
            crate::fs_utils::remove_file_if_exists(root.join(filename))?;
        }
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let cached = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<DiskCache>(&bytes).ok())
        .filter(|disk| disk.data.validate().is_ok());
    if !force
        && cached.as_ref().is_some_and(|disk| {
            disk.fetched_at > 0 && disk.fetched_at <= now && now - disk.fetched_at < TTL
        })
    {
        return Ok(cached.unwrap().data);
    }
    let fetched = (|| -> Result<ScannerData> {
        let response = crate::data_http::client()?
            .get(url)
            .send()?
            .error_for_status()?;
        let mut bytes = Vec::new();
        response.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("Shared Genshin scanner data exceeds size limit");
        }
        let data: ScannerData = serde_json::from_slice(&bytes)?;
        data.validate()?;
        Ok(data)
    })();
    let data = match fetched {
        Ok(data) => data,
        Err(error) => {
            if !force {
                if let Some(cached) = cached {
                    yas::log_warn!(
                        "无法更新原神游戏数据，将使用本地缓存。完整错误详情: {:#}",
                        "Genshin game data could not be updated; using the local cache. Full error details: {:#}",
                        error
                    );
                    return Ok(cached.data);
                }
            }
            return Err(error).context("Could not update shared Genshin scanner data");
        },
    };
    let disk = DiskCache {
        fetched_at: now,
        data,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(&disk)?)?;
    fs::rename(&temporary, path)?;
    Ok(disk.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    fn document() -> serde_json::Value {
        let artifacts: serde_json::Map<String, serde_json::Value> = (1..=100)
            .map(|id| {
                (
                    id.to_string(),
                    serde_json::json!({"set": "Test", "slot": "Flower", "rarity": 5}),
                )
            })
            .collect();
        serde_json::json!({"formatVersion": 1, "sourceRevision": "test-revision",
            "mappings": {"characters": [{"id": "Test", "n": {"zh": "测试"}}],
                "weapons": [{"id": "Test", "n": {"zh": "测试"}}],
                "artifactSets": [{"id": "Test", "n": {"zh": "测试"}}]},
            "capture": {"version": 1, "git_hash": "test-revision", "artifact_map": artifacts,
                "set_map": {"1": "Test"}, "character_map": {"1": "Test"},
                "weapon_map": {"1": {"name": "Test", "rarity": 5}},
                "affix_map": {"1": {"property": "CritRate", "value": 0.03}},
                "property_map": {"1": "CritRate"}, "skill_type_map": {"1": "Auto"}}})
    }

    fn server(body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/good/genshin_scanner_data.json",
            listener.local_addr().unwrap()
        );
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        (url, handle)
    }

    #[test]
    fn shared_cache_reuses_one_download_and_preserves_valid_data_on_refresh_failure() {
        let root = std::env::temp_dir().join(format!(
            "genshin-shared-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("mappings.json"), "legacy").unwrap();
        fs::write(root.join("data_cache.json"), "legacy").unwrap();
        let path = root.join("genshin_scanner_data.json");
        let (url, task) = server(serde_json::to_vec(&document()).unwrap());
        let loaded = load_from_url(&path, &url, false).unwrap();
        task.join().unwrap();
        assert!(!root.join("mappings.json").exists());
        assert!(!root.join("data_cache.json").exists());
        assert_eq!(loaded.capture.artifact_map.len(), 100);
        // The server has stopped: all modes reuse the same complete cached document.
        assert!(load_from_url(&path, &url, false).is_ok());
        assert!(!root.join("mapping_achievements.json").exists());
        let valid_bytes = fs::read(&path).unwrap();
        let mut wrong_revision = document();
        wrong_revision["sourceRevision"] = "wrong-revision".into();
        let mut incomplete = document();
        incomplete["capture"]["artifact_map"] = serde_json::json!({});
        for invalid in [
            b"invalid JSON".to_vec(),
            serde_json::to_vec(&wrong_revision).unwrap(),
            serde_json::to_vec(&incomplete).unwrap(),
        ] {
            let (url, task) = server(invalid);
            assert!(load_from_url(&path, &url, true).is_err());
            task.join().unwrap();
            assert_eq!(fs::read(&path).unwrap(), valid_bytes);
        }
        let mut disk: serde_json::Value = serde_json::from_slice(&valid_bytes).unwrap();
        disk["fetchedAt"] = 0.into();
        fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
        let (url, task) = server(b"invalid JSON".to_vec());
        assert!(load_from_url(&path, &url, false).is_ok());
        task.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
