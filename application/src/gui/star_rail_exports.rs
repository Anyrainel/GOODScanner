//! Save each game's scan/capture stream before removing its older exports.
use hsr_scanner::{HsrError, HsrResult};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct CaptureFiles {
    pub paths: Vec<PathBuf>,
}

#[derive(Clone, Copy)]
enum ExportKind {
    Capture,
    Scan,
}

impl ExportKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Capture => "star_rail_capture_",
            Self::Scan => "star_rail_scan_",
        }
    }
    fn fail(self, detail: String) -> HsrError {
        HsrError::write_failed(
            match self {
                Self::Capture => "HSR-CAPTURE-EXPORT-FILES",
                Self::Scan => "HSR-SCAN-EXPORT-FILES",
            },
            detail,
        )
    }
}

fn export_stamp(kind: ExportKind) -> HsrResult<String> {
    Ok(format!(
        "{}_{:09}",
        genshin_scanner::cli::chrono_timestamp(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| kind.fail(e.to_string()))?
            .subsec_nanos()
    ))
}

pub fn write_scan_file(output_dir: &Path, export: &Value, only_latest: bool) -> HsrResult<PathBuf> {
    write_export_file(
        output_dir,
        &export_stamp(ExportKind::Scan)?,
        export,
        only_latest,
        ExportKind::Scan,
    )
}

pub fn write_capture_files(
    output_dir: &Path,
    export: &Value,
    only_latest: bool,
) -> HsrResult<CaptureFiles> {
    let stamp = export_stamp(ExportKind::Capture)?;
    write_capture_set(output_dir, &stamp, export, only_latest)
}

fn write_capture_set(
    output_dir: &Path,
    stamp: &str,
    export: &Value,
    only_latest: bool,
) -> HsrResult<CaptureFiles> {
    write_export_file(output_dir, stamp, export, only_latest, ExportKind::Capture)
        .map(|path| CaptureFiles { paths: vec![path] })
}

fn write_export_file(
    output_dir: &Path,
    stamp: &str,
    export: &Value,
    only_latest: bool,
    kind: ExportKind,
) -> HsrResult<PathBuf> {
    let bytes = serde_json::to_vec_pretty(export).map_err(|e| kind.fail(e.to_string()))?;
    let path = output_dir.join(format!("{}{stamp}.json", kind.prefix()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            kind.fail(format!(
                "path={}; cause={e}; older exports retained",
                path.display()
            ))
        })?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let cleanup = fs::remove_file(&path);
        return Err(kind.fail(format!(
            "path={}; cause={error}; incomplete-file cleanup={cleanup:?}; older exports retained",
            path.display()
        )));
    }
    drop(file);
    if only_latest {
        remove_older_exports(output_dir, stamp, kind).map_err(|e| {
            kind.fail(format!(
                "new exports saved: {}; older exports could not all be removed: {e}",
                path.display()
            ))
        })?;
    }
    Ok(path)
}

fn remove_older_exports(dir: &Path, current: &str, kind: ExportKind) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        // Never follow symlinks or recurse; only generated HSR export filenames.
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(stamp) = name.to_str().and_then(|name| export_timestamp(name, kind)) else {
            continue;
        };
        if stamp < current {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
fn export_timestamp(name: &str, kind: ExportKind) -> Option<&str> {
    let body = name.strip_suffix(".json")?;
    let prefixes: &[&str] = match kind {
        ExportKind::Scan => &["star_rail_scan_"],
        ExportKind::Capture => &[
            "star_rail_capture_",
            "star_rail_export_",
            "star_rail_fribbels_",
            "star_rail_achievements_",
        ],
    };
    let stamp = prefixes.iter().find_map(|p| body.strip_prefix(p))?;
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
        let old = dir.join("star_rail_capture_2026-09-01_10-00-00.json");
        fs::write(&old, b"previous complete export").unwrap();
        fs::create_dir(dir.join("star_rail_capture_2026-09-07_10-00-00_000000001.json")).unwrap();
        let export = serde_json::json!({"source": "HSR-Scanner", "version": 4});
        assert!(write_capture_set(&dir, "2026-09-07_10-00-00_000000001", &export, true).is_err());
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
            "star_rail_capture_2026-09-01_10-00-00.json",
            "star_rail_export_2026-09-01_10-00-00.json",
            "star_rail_fribbels_2026-09-01_10-00-00_000000001.json",
            "star_rail_achievements_2026-09-01_10-00-00_000000001.json",
        ];
        let keep = [
            "genshin_export_2026-09-01_10-00-00.json",
            "star_rail_scan_2026-09-01_10-00-00.json",
            "star_rail_export_latest.json",
            "star_rail_export_2026-02-30_10-00-00.json",
            "star_rail_export_2026-09-01_10-00-00.backup.json",
            "star_rail_capture_2026-09-07_10-00-00_000000001.json",
            "star_rail_fribbels_2026-09-08_10-00-00.json",
        ];
        for name in old.iter().chain(keep.iter()) {
            fs::write(dir.join(name), b"keep").unwrap();
        }
        fs::create_dir(dir.join("star_rail_export_2026-09-01_10-00-00_000000009.json")).unwrap();
        remove_older_exports(&dir, "2026-09-07_10-00-00_000000001", ExportKind::Capture).unwrap();
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
