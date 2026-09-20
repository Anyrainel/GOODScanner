use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};

pub const SCANNER_EXPORT_PREFIX: &str = "good_export_";
pub const CAPTURE_EXPORT_PREFIX: &str = "genshin_export_";
pub const EXPORT_JSON_SUFFIX: &str = ".json";

/// Remove a file when present, while preserving any real filesystem failure.
///
/// Cache refreshers share this helper so a locked or unreadable cache cannot
/// be mistaken for a successful refresh.
pub(crate) fn remove_file_if_exists(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to remove cached file: {}", path.display()))
        },
    }
}

/// Delete previously generated timestamped exports that share `prefix`.
pub fn remove_previous_exports(output_dir: &Path, prefix: &str) -> Result<usize> {
    let mut removed = 0;
    for entry in fs::read_dir(output_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if is_generated_export_filename(file_name, prefix) {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn is_generated_export_filename(file_name: &str, prefix: &str) -> bool {
    if !file_name.starts_with(prefix) || !file_name.ends_with(EXPORT_JSON_SUFFIX) {
        return false;
    }

    let timestamp = &file_name[prefix.len()..file_name.len() - EXPORT_JSON_SUFFIX.len()];
    is_export_timestamp(timestamp)
}

fn is_export_timestamp(timestamp: &str) -> bool {
    let bytes = timestamp.as_bytes();
    if bytes.len() != 19 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        let expected_separator = matches!(idx, 4 | 7 | 13 | 16);
        if expected_separator {
            if *byte != b'-' {
                return false;
            }
        } else if idx == 10 {
            if *byte != b'_' {
                return false;
            }
        } else if !byte.is_ascii_digit() {
            return false;
        }
    }

    let month = parse_two_digits(&bytes[5..7]);
    let day = parse_two_digits(&bytes[8..10]);
    let hour = parse_two_digits(&bytes[11..13]);
    let minute = parse_two_digits(&bytes[14..16]);
    let second = parse_two_digits(&bytes[17..19]);

    (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 59
}

fn parse_two_digits(bytes: &[u8]) -> u8 {
    (bytes[0] - b'0') * 10 + (bytes[1] - b'0')
}

#[cfg(test)]
mod tests {
    use super::{is_generated_export_filename, CAPTURE_EXPORT_PREFIX, SCANNER_EXPORT_PREFIX};

    #[test]
    fn matches_generated_scanner_exports_only() {
        assert!(is_generated_export_filename(
            "good_export_2026-04-27_13-45-09.json",
            SCANNER_EXPORT_PREFIX
        ));
        assert!(!is_generated_export_filename(
            "good_export_2026-04-27_13-45.json",
            SCANNER_EXPORT_PREFIX
        ));
        assert!(!is_generated_export_filename(
            "good_export_latest.json",
            SCANNER_EXPORT_PREFIX
        ));
        assert!(!is_generated_export_filename(
            "good_export_2026-13-27_13-45-09.json",
            SCANNER_EXPORT_PREFIX
        ));
        assert!(!is_generated_export_filename(
            "genshin_export_2026-04-27_13-45-09.json",
            SCANNER_EXPORT_PREFIX
        ));
    }

    #[test]
    fn matches_generated_capture_exports_only() {
        assert!(is_generated_export_filename(
            "genshin_export_2026-04-27_13-45-09.json",
            CAPTURE_EXPORT_PREFIX
        ));
        assert!(!is_generated_export_filename(
            "genshin_export_2026-04-27_13-45.json",
            CAPTURE_EXPORT_PREFIX
        ));
        assert!(!is_generated_export_filename(
            "genshin_export_latest.json",
            CAPTURE_EXPORT_PREFIX
        ));
        assert!(!is_generated_export_filename(
            "genshin_export_2026-13-27_13-45-09.json",
            CAPTURE_EXPORT_PREFIX
        ));
        assert!(!is_generated_export_filename(
            "good_export_2026-04-27_13-45-09.json",
            CAPTURE_EXPORT_PREFIX
        ));
    }

    #[test]
    fn remove_previous_exports_deletes_matching_files_only() {
        let dir =
            std::env::temp_dir().join(format!("goodscanner-export-cleanup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let keep_latest = dir.join("good_export_2026-04-27_13-45-09.json");
        let keep_unrelated = dir.join("genshin_export_2026-04-27_13-45-09.json");
        let keep_manual = dir.join("good_export_latest.json");
        std::fs::write(&keep_latest, "{}").unwrap();
        std::fs::write(&keep_unrelated, "{}").unwrap();
        std::fs::write(&keep_manual, "{}").unwrap();

        let removed = super::remove_previous_exports(&dir, SCANNER_EXPORT_PREFIX).unwrap();
        assert_eq!(removed, 1);
        assert!(!keep_latest.exists());
        assert!(keep_unrelated.exists());
        assert!(keep_manual.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
