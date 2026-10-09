//! Hosted game data, with the same two-hour refresh policy as Genshin.
use crate::{
    model::ReferenceSnapshot, packet_reference::PacketReferences, reference::ReferenceCache,
    HsrError, HsrResult, LocalizedText,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::Path,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DATA_CACHE_URL: &str = "https://hsr.ggartifact.com/good/hsr_scanner_data.json";
pub const ACHIEVEMENT_IDS_URL: &str = "https://hsr.ggartifact.com/good/hsr_achievement_ids.json";
pub const DATA_CACHE_DIRECTORY: &str = "data/hsr";
const DATA_FILE: &str = "hsr_scanner_data.json";
const ACHIEVEMENT_FILE: &str = "hsr_achievement_ids.json";
// Old combined caches, including the briefly separated ocr/capture caches,
// carry achievement IDs and formatVersion 1. These derived files are ignored;
// the new filenames and formatVersion 2 establish one shared inventory cache.
const TTL: u64 = 2 * 3600;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
static CACHE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharedDataDocument {
    pub format_version: u32,
    pub snapshot: ReferenceSnapshot,
    pub packet: PacketReferences,
}

impl SharedDataDocument {
    pub fn validate(&self) -> HsrResult<ReferenceCache> {
        if self.format_version != 2
            || self.snapshot.provider != "gilore.ggstarrail-reference"
            || self.snapshot.revision != self.packet.source_revision
            || !self.snapshot.achievement_ids.is_empty()
        {
            return Err(error(
                "HSR-REF-FORMAT",
                "unsupported format or inconsistent source revision",
            ));
        }
        if self.packet.main.len() != self.snapshot.relic_main_affixes.len()
            || self.snapshot.relic_main_affixes.iter().any(|a| {
                !self
                    .packet
                    .main
                    .iter()
                    .any(|p| p.group == a.group_id && p.property == a.property_id)
            })
        {
            return Err(error(
                "HSR-REF-INCOMPLETE",
                "packet main affixes disagree with reference progression",
            ));
        }
        let cache = ReferenceCache::from_snapshot(self.snapshot.clone())?;
        cache.validate_live_complete_profile()?;
        cache.with_packet_references(self.packet.clone())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiskCache<T> {
    fetched_at: u64,
    data: T,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AchievementDocument {
    format_version: u32,
    source_revision: String,
    achievement_ids: Vec<u32>,
}

impl AchievementDocument {
    fn attach(&self, references: &ReferenceCache) -> HsrResult<ReferenceCache> {
        if self.format_version != 1 || self.source_revision != references.revision() {
            return Err(error(
                "HSR-REF-ACHIEVEMENT-REVISION",
                "achievement IDs and inventory reference revisions disagree",
            ));
        }
        references
            .clone()
            .with_achievement_ids(self.achievement_ids.clone())
    }
}

fn error(code: &'static str, detail: impl Into<String>) -> HsrError {
    HsrError::new(code, LocalizedText::new(
        "无法更新星穹铁道游戏数据。请检查网络连接，然后点击“刷新游戏数据”重试。",
        "Star Rail game data could not be updated. Check your connection, then select Refresh game data."), detail)
}

pub fn load_data_cache() -> HsrResult<ReferenceCache> {
    match load_from_url(Path::new(DATA_CACHE_DIRECTORY), DATA_CACHE_URL, false) {
        Ok(cache) => Ok(cache),
        Err(error) => match crate::load_embedded_gilore_reference() {
            Ok(cache) => {
                yas::log_warn!(
                    "无法下载星穹铁道游戏数据，将使用内置参考数据。完整错误详情: {}",
                    "Star Rail game data could not be downloaded; using the built-in reference. Full error details: {}",
                    error
                );
                Ok(cache.without_achievements())
            },
            Err(_) => Err(error),
        },
    }
}

pub fn force_refresh() -> HsrResult<()> {
    load_from_url(Path::new(DATA_CACHE_DIRECTORY), DATA_CACHE_URL, true).map(|_| ())
}

/// Only achievement capture calls this; inventory operations never download IDs.
pub fn load_achievement_data(
    references: &ReferenceCache,
    force: bool,
) -> HsrResult<ReferenceCache> {
    let result = load_document::<AchievementDocument, _>(
        Path::new(DATA_CACHE_DIRECTORY),
        ACHIEVEMENT_FILE,
        ACHIEVEMENT_IDS_URL,
        force,
        |data| data.attach(references),
    );
    match result {
        Ok(cache) => Ok(cache),
        Err(failure) if !force => {
            let embedded = crate::load_embedded_gilore_reference()?;
            if embedded.revision() != references.revision() {
                return Err(failure);
            }
            yas::log_warn!(
                "无法下载星穹铁道成就数据，将使用内置参考数据。完整错误详情: {}",
                "Star Rail achievement data could not be downloaded; using the built-in reference. Full error details: {}",
                failure
            );
            references
                .clone()
                .with_achievement_ids(embedded.achievement_ids().collect())
        },
        Err(failure) => Err(failure),
    }
}

/// Metadata for the UI. Reading it does not load or refresh the reference data.
pub fn cache_updated_at() -> Option<u64> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Metadata {
        fetched_at: u64,
    }
    let file = fs::File::open(Path::new(DATA_CACHE_DIRECTORY).join(DATA_FILE)).ok()?;
    let metadata: Metadata = serde_json::from_reader(std::io::BufReader::new(file)).ok()?;
    (metadata.fetched_at > 0).then_some(metadata.fetched_at)
}

/// Also used by the offline HTTP integration harness. App callers always use the fixed HSR host.
pub fn load_from_url(root: &Path, url: &str, force: bool) -> HsrResult<ReferenceCache> {
    load_document::<SharedDataDocument, _>(
        root,
        DATA_FILE,
        url,
        force,
        SharedDataDocument::validate,
    )
}

fn load_document<T: DeserializeOwned + Serialize, R>(
    root: &Path,
    filename: &str,
    url: &str,
    force: bool,
    validate: impl Fn(&T) -> HsrResult<R>,
) -> HsrResult<R> {
    let _guard = CACHE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = root.join(filename);
    let cached = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<DiskCache<T>>(&bytes).ok())
        .and_then(|disk| {
            validate(&disk.data)
                .ok()
                .map(|cache| (disk.fetched_at, cache))
        });
    if !force {
        if cached
            .as_ref()
            .is_some_and(|(fetched, _)| *fetched > 0 && *fetched <= now && now - fetched < TTL)
        {
            return Ok(cached.unwrap().1);
        }
    }
    yas::log_info!(
        "正在下载星穹铁道游戏数据...",
        "Downloading Star Rail game data..."
    );
    let fetched = fetch::<T>(url).and_then(|data| validate(&data).map(|cache| (data, cache)));
    let (data, cache) = match fetched {
        Ok(result) => result,
        Err(failure) => {
            if !force {
                if let Some((_, cache)) = cached {
                    yas::log_warn!(
                        "无法更新星穹铁道游戏数据，将使用本地缓存。完整错误详情: {}",
                        "Star Rail game data could not be updated; using the local cache. Full error details: {}",
                        failure);
                    return Ok(cache);
                }
            }
            return Err(failure);
        },
    };
    // Never discard a working cache until the complete replacement has validated.
    fs::create_dir_all(root)
        .map_err(|e| error("HSR-REF-CACHE-WRITE", format!("{}: {e}", root.display())))?;
    let bytes = serde_json::to_vec(&DiskCache {
        fetched_at: now,
        data,
    })
    .map_err(|e| error("HSR-REF-CACHE-WRITE", e.to_string()))?;
    let temp = root.join(format!("{filename}.{}.tmp", std::process::id()));
    fs::write(&temp, bytes)
        .and_then(|_| fs::rename(&temp, &path))
        .map_err(|e| error("HSR-REF-CACHE-WRITE", format!("{}: {e}", path.display())))?;
    yas::log_info!("星穹铁道游戏数据已更新", "Star Rail game data updated");
    Ok(cache)
}

fn fetch<T: DeserializeOwned>(url: &str) -> HsrResult<T> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| error("HSR-REF-DOWNLOAD", e.to_string()))?;
    let response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| error("HSR-REF-DOWNLOAD", format!("url={url}; {e}")))?;
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| error("HSR-REF-DOWNLOAD", format!("url={url}; {e}")))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error(
            "HSR-REF-SIZE",
            format!("url={url}; reference exceeds {MAX_BYTES} bytes"),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|e| error("HSR-REF-JSON", format!("url={url}; {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    fn document() -> serde_json::Value {
        let mut embedded: serde_json::Value =
            serde_json::from_slice(include_bytes!("../assets/gilore_reference_v1.json")).unwrap();
        embedded["snapshot"]
            .as_object_mut()
            .unwrap()
            .remove("achievementIds");
        serde_json::json!({"formatVersion": 2, "snapshot": embedded["snapshot"],
            "packet": serde_json::from_slice::<serde_json::Value>(include_bytes!("../assets/packet_affixes.json")).unwrap()})
    }

    fn server(status: u16, body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/good/hsr_scanner_data.json",
            listener.local_addr().unwrap()
        );
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        (url, handle)
    }

    #[test]
    fn inventory_cache_is_shared_and_achievement_ids_are_optional_and_revision_checked() {
        let root = std::env::temp_dir().join(format!(
            "hsr-mode-cache-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let initial = document();
        // A legacy combined cache is deliberately ignored at the new boundary.
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("hsr_data_cache.json"), b"legacy cache").unwrap();
        let (url, task) = server(200, serde_json::to_vec(&initial).unwrap());
        let references = load_from_url(&root, &url, false).unwrap();
        task.join().unwrap();
        assert_eq!(references.achievement_count(), 0);
        assert!(!root.join(ACHIEVEMENT_FILE).exists());
        // Scanner, manager and capture all reuse this file without another request.
        assert!(load_from_url(&root, &url, false).is_ok());
        let inventory_bytes = fs::read(root.join(DATA_FILE)).unwrap();
        let ids = serde_json::json!({"formatVersion": 1, "sourceRevision": references.revision(), "achievementIds": [4010101]});
        let (url, task) = server(200, serde_json::to_vec(&ids).unwrap());
        let loaded =
            load_document::<AchievementDocument, _>(&root, ACHIEVEMENT_FILE, &url, false, |data| {
                data.attach(&references)
            })
            .unwrap();
        task.join().unwrap();
        assert!(loaded.has_achievement(4010101));
        assert_eq!(fs::read(root.join(DATA_FILE)).unwrap(), inventory_bytes);
        let good_ids = fs::read(root.join(ACHIEVEMENT_FILE)).unwrap();
        for invalid in [
            serde_json::json!({"formatVersion": 1, "sourceRevision": "wrong", "achievementIds": [4010101]}),
            serde_json::json!({"formatVersion": 1, "sourceRevision": references.revision(), "achievementIds": []}),
            serde_json::json!({"formatVersion": 1, "sourceRevision": references.revision(), "achievementIds": [4010101, 4010101]}),
        ] {
            let (url, task) = server(200, serde_json::to_vec(&invalid).unwrap());
            assert!(load_document::<AchievementDocument, _>(
                &root,
                ACHIEVEMENT_FILE,
                &url,
                true,
                |data| data.attach(&references)
            )
            .is_err());
            task.join().unwrap();
            assert_eq!(fs::read(root.join(ACHIEVEMENT_FILE)).unwrap(), good_ids);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn http_cache_refresh_updates_without_a_build_and_preserves_last_good_data() {
        let root = std::env::temp_dir().join(format!(
            "hsr-remote-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join(DATA_FILE);
        let mut next = document();
        let (url, task) = server(200, serde_json::to_vec(&next).unwrap());
        assert!(load_from_url(&root, &url, false)
            .unwrap()
            .character(1508)
            .is_some());
        task.join().unwrap();
        // The server is gone: a fresh cache must not make an HTTP request.
        assert!(load_from_url(&root, &url, false).is_ok());

        next["snapshot"]["revision"] = "next-game-data-revision".into();
        next["packet"]["sourceRevision"] = "next-game-data-revision".into();
        let mut character = next["snapshot"]["characters"][0].clone();
        character["gameId"] = 1999.into();
        character["key"] = "1999".into();
        next["snapshot"]["characters"]
            .as_array_mut()
            .unwrap()
            .push(character);
        // A packet table change must come from the same download too.
        next["packet"]["sub"][0]["base"] = 9.5.into();
        let (url, task) = server(200, serde_json::to_vec(&next).unwrap());
        let updated = load_from_url(&root, &url, true).unwrap();
        assert!(updated.character(1999).is_some());
        assert_eq!(updated.packet_references().unwrap().sub[0].base, 9.5);
        task.join().unwrap();
        let good_bytes = fs::read(&path).unwrap();
        for payload in [
            b"<html>not data</html>".to_vec(),
            {
                let mut incomplete = next.clone();
                incomplete["packet"]["sub"] = serde_json::json!([]);
                serde_json::to_vec(&incomplete).unwrap()
            },
            {
                let mut legacy = next.clone();
                legacy["formatVersion"] = 1.into();
                serde_json::to_vec(&legacy).unwrap()
            },
            {
                let mut combined = next.clone();
                combined["snapshot"]["achievementIds"] = serde_json::json!([4010101]);
                serde_json::to_vec(&combined).unwrap()
            },
        ] {
            let (url, task) = server(200, payload);
            assert!(load_from_url(&root, &url, true).is_err());
            task.join().unwrap();
            assert_eq!(fs::read(&path).unwrap(), good_bytes);
        }
        let mut disk: serde_json::Value = serde_json::from_slice(&good_bytes).unwrap();
        disk["fetchedAt"] = 0.into();
        fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
        let (url, task) = server(503, vec![]);
        assert!(load_from_url(&root, &url, false)
            .unwrap()
            .character(1999)
            .is_some());
        task.join().unwrap();
        // Corrupted local data may never substitute for a failed cold download.
        fs::write(&path, b"broken").unwrap();
        let (url, task) = server(503, vec![]);
        assert!(load_from_url(&root, &url, false).is_err());
        task.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
