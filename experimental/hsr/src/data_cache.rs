//! Hosted game data, with the same two-hour refresh policy as Genshin.
use crate::{
    model::ReferenceSnapshot, packet_reference::PacketReferences, reference::ReferenceCache,
    HsrError, HsrResult, LocalizedText,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::Path,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DATA_CACHE_URL: &str = "https://hsr.ggartifact.com/good/hsr_data_cache.json";
pub const DATA_CACHE_DIRECTORY: &str = "data/hsr";
const TTL: u64 = 2 * 3600;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
static CACHE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureDataDocument {
    pub format_version: u32,
    pub snapshot: ReferenceSnapshot,
    pub packet: PacketReferences,
}

impl CaptureDataDocument {
    pub fn validate(&self) -> HsrResult<ReferenceCache> {
        if self.format_version != 1
            || self.snapshot.provider != "gilore.ggstarrail-reference"
            || self.snapshot.revision != self.packet.source_revision
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
        if cache.achievement_count() == 0 {
            return Err(error(
                "HSR-REF-INCOMPLETE",
                "achievement reference is empty",
            ));
        }
        cache.with_packet_references(self.packet.clone())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiskCache {
    fetched_at: u64,
    data: CaptureDataDocument,
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
                Ok(cache)
            },
            Err(_) => Err(error),
        },
    }
}

pub fn force_refresh() -> HsrResult<()> {
    load_from_url(Path::new(DATA_CACHE_DIRECTORY), DATA_CACHE_URL, true).map(|_| ())
}

/// Also used by the offline HTTP integration harness. App callers always use the fixed HSR host.
pub fn load_from_url(root: &Path, url: &str, force: bool) -> HsrResult<ReferenceCache> {
    let _guard = CACHE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = root.join("hsr_data_cache.json");
    let cached = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<DiskCache>(&bytes).ok())
        .and_then(|disk| {
            disk.data
                .validate()
                .ok()
                .map(|cache| (disk.fetched_at, cache))
        });
    if !force {
        if let Some((fetched, cache)) = &cached {
            if *fetched > 0 && *fetched <= now && now - fetched < TTL {
                return Ok(cache.clone());
            }
        }
    }
    yas::log_info!(
        "正在下载星穹铁道游戏数据...",
        "Downloading Star Rail game data..."
    );
    let fetched = fetch(url).and_then(|data| data.validate().map(|cache| (data, cache)));
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
    let temp = root.join(format!("hsr_data_cache.{}.tmp", std::process::id()));
    fs::write(&temp, bytes)
        .and_then(|_| fs::rename(&temp, &path))
        .map_err(|e| error("HSR-REF-CACHE-WRITE", format!("{}: {e}", path.display())))?;
    yas::log_info!("星穹铁道游戏数据已更新", "Star Rail game data updated");
    Ok(cache)
}

fn fetch(url: &str) -> HsrResult<CaptureDataDocument> {
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
        let embedded: serde_json::Value =
            serde_json::from_slice(include_bytes!("../assets/gilore_reference_v1.json")).unwrap();
        serde_json::json!({"formatVersion": 1, "snapshot": embedded["snapshot"],
            "packet": serde_json::from_slice::<serde_json::Value>(include_bytes!("../assets/packet_affixes.json")).unwrap()})
    }

    fn server(status: u16, body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/good/hsr_data_cache.json",
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
    fn http_cache_refresh_updates_without_a_build_and_preserves_last_good_data() {
        let root = std::env::temp_dir().join(format!(
            "hsr-remote-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("hsr_data_cache.json");
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
        for payload in [b"<html>not data</html>".to_vec(), {
            let mut incomplete = next.clone();
            incomplete["packet"]["sub"] = serde_json::json!([]);
            serde_json::to_vec(&incomplete).unwrap()
        }] {
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
