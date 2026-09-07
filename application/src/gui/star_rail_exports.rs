//! One capture produces a complete export set before any older set is removed.
use hsr_scanner::{HsrError, HsrResult};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct CaptureFiles {
    pub paths: Vec<PathBuf>,
}

pub fn write_capture_files(
    output_dir: &Path,
    gg: &hsr_scanner::HsrInventoryExport,
    scanner: Option<&serde_json::Value>,
    only_latest: bool,
) -> HsrResult<CaptureFiles> {
    let stamp = format!(
        "{}_{:09}",
        genshin_scanner::cli::chrono_timestamp(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| fail(e.to_string()))?
            .subsec_nanos()
    );
    write_capture_set(output_dir, &stamp, gg, scanner, only_latest)
}

fn write_capture_set(
    output_dir: &Path,
    stamp: &str,
    gg: &hsr_scanner::HsrInventoryExport,
    scanner: Option<&serde_json::Value>,
    only_latest: bool,
) -> HsrResult<CaptureFiles> {
    // Serialize every selected format before opening any destination.
    let mut outputs = Vec::new();
    if let Some(scanner) = scanner {
        outputs.push(("star_rail_fribbels_", serialize(scanner)?));
    }
    outputs.push(("star_rail_export_", serialize(gg)?));
    if let Some(achievements) = &gg.achievements {
        let ids: Vec<_> = achievements
            .entries
            .iter()
            .map(|a| a.achievement_id)
            .collect();
        outputs.push((
            "star_rail_achievements_",
            serialize(&serde_json::json!({"hsr_achievements": ids}))?,
        ));
    }
    let mut paths = Vec::new();
    for (prefix, bytes) in outputs {
        let path = output_dir.join(format!("{prefix}{stamp}.json"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                fail(format!(
                    "path={}; cause={e}; older exports retained",
                    path.display()
                ))
            })?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| {
                fail(format!(
                    "path={}; cause={e}; older exports retained",
                    path.display()
                ))
            })?;
        paths.push(path);
    }
    if only_latest {
        remove_older_exports(output_dir, stamp).map_err(|e| {
            fail(format!(
                "new exports saved: {}; older exports could not all be removed: {e}",
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
    }
    Ok(CaptureFiles { paths })
}
fn serialize(value: &impl serde::Serialize) -> HsrResult<Vec<u8>> {
    serde_json::to_vec_pretty(value).map_err(|e| fail(e.to_string()))
}
fn fail(detail: String) -> HsrError {
    HsrError::write_failed("HSR-CAPTURE-EXPORT-FILES", detail)
}

fn remove_older_exports(dir: &Path, current: &str) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        // Never follow symlinks or recurse; only generated HSR export filenames.
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(stamp) = name.to_str().and_then(export_timestamp) else {
            continue;
        };
        if stamp < current {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
fn export_timestamp(name: &str) -> Option<&str> {
    let body = name.strip_suffix(".json")?;
    let stamp = [
        "star_rail_export_",
        "star_rail_fribbels_",
        "star_rail_achievements_",
    ]
    .iter()
    .find_map(|p| body.strip_prefix(p))?;
    let bytes = stamp.as_bytes();
    if bytes.len() != 19 && bytes.len() != 29 {
        return None;
    }
    for (i, b) in bytes.iter().enumerate() {
        let valid = match i {
            4 | 7 | 13 | 16 => *b == b'-',
            10 | 19 => *b == b'_',
            _ => b.is_ascii_digit(),
        };
        if !valid {
            return None;
        }
    }
    let num = |i: usize| (bytes[i] - b'0') as u32 * 10 + (bytes[i + 1] - b'0') as u32;
    let year = stamp[..4].parse::<u32>().ok()?;
    let month = num(5);
    let day = num(8);
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        },
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    (year > 0 && (1..=days).contains(&day) && num(11) <= 23 && num(14) <= 59 && num(17) <= 59)
        .then_some(stamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_new_export_never_deletes_previous_export() {
        let dir = std::env::temp_dir().join(format!(
            "hsr-write-failure-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let old = dir.join("star_rail_export_2026-09-01_10-00-00.json");
        fs::write(&old, b"previous complete export").unwrap();
        fs::create_dir(dir.join("star_rail_export_2026-09-07_10-00-00_000000001.json")).unwrap();
        let refs = hsr_scanner::load_embedded_gilore_reference().unwrap();
        let achievements = hsr_scanner::build_achievement_snapshot([], "test", &refs).unwrap();
        let gg = hsr_scanner::build_achievement_only_export(achievements, &refs).unwrap();
        assert!(write_capture_set(&dir, "2026-09-07_10-00-00_000000001", &gg, None, true).is_err());
        assert_eq!(fs::read(old).unwrap(), b"previous complete export");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cleanup_is_hsr_only_and_keeps_current_set_future_files_and_user_files() {
        let dir = std::env::temp_dir().join(format!(
            "hsr-retention-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let old = [
            "star_rail_export_2026-09-01_10-00-00.json",
            "star_rail_fribbels_2026-09-01_10-00-00_000000001.json",
            "star_rail_achievements_2026-09-01_10-00-00_000000001.json",
        ];
        let keep = [
            "genshin_export_2026-09-01_10-00-00.json",
            "star_rail_export_latest.json",
            "star_rail_export_2026-02-30_10-00-00.json",
            "star_rail_export_2026-09-01_10-00-00.backup.json",
            "star_rail_export_2026-09-07_10-00-00_000000001.json",
            "star_rail_fribbels_2026-09-08_10-00-00.json",
        ];
        for name in old.iter().chain(keep.iter()) {
            fs::write(dir.join(name), b"keep").unwrap();
        }
        fs::create_dir(dir.join("star_rail_export_2026-09-01_10-00-00_000000009.json")).unwrap();
        remove_older_exports(&dir, "2026-09-07_10-00-00_000000001").unwrap();
        for name in old {
            assert!(!dir.join(name).exists());
        }
        for name in keep {
            assert_eq!(fs::read(dir.join(name)).unwrap(), b"keep");
        }
        assert!(dir
            .join("star_rail_export_2026-09-01_10-00-00_000000009.json")
            .is_dir());
        fs::remove_dir_all(dir).unwrap();
    }
}
