//! OCR a single achievement row and decide whether it is completed.
//!
//! Field split + matching follow cocogoat `scannerOnLine` /
//! `recognizeAchievement`. Completion uses date / `达成` / `n/m` progress,
//! the same signals cocogoat reads from the status and date crops.

use anyhow::Result;
use image::RgbImage;
use yas::ocr::ImageToText;

use super::catalog::AchievementCatalog;
use super::layout::{DATE_FRAC, STATUS_FRAC, SUBTITLE_FRAC, TITLE_FRAC};
use super::split::crop_frac;

#[derive(Clone, Debug)]
pub struct RecognizedRow {
    pub id: Option<u32>,
    pub title: String,
    pub subtitle: String,
    pub status: String,
    pub date: String,
    pub done: bool,
}

pub fn recognize_row(
    ocr: &dyn ImageToText<RgbImage>,
    row: &RgbImage,
    catalog: &AchievementCatalog,
) -> Result<RecognizedRow> {
    let title = ocr_crop(ocr, row, TITLE_FRAC)?;
    let subtitle = ocr_crop(ocr, row, SUBTITLE_FRAC)?;
    let status = ocr_crop(ocr, row, STATUS_FRAC)?;
    let date = ocr_crop(ocr, row, DATE_FRAC)?;
    let id = catalog.match_text(&title, &subtitle);
    let done = is_done(&status, &date);
    Ok(RecognizedRow {
        id,
        title,
        subtitle,
        status,
        date,
        done,
    })
}

fn ocr_crop(
    ocr: &dyn ImageToText<RgbImage>,
    row: &RgbImage,
    frac: (f32, f32, f32, f32),
) -> Result<String> {
    let Some(crop) = crop_frac(row, frac) else {
        return Ok(String::new());
    };
    let text = ocr.image_to_text(&crop, false)?;
    Ok(text.trim().to_string())
}

/// Port of cocogoat's done heuristic:
/// - `达成` → done
/// - `current/max` with current < max → not done
/// - date shorter than 4 characters, or missing → not done
/// - otherwise a plausible date → done
pub fn is_done(status: &str, date: &str) -> bool {
    let status = status.replace(' ', "");
    if status.contains("达成") {
        return true;
    }

    if let Some((cur, max)) = parse_slash_pair(&status) {
        if cur < max {
            return false;
        }
        if cur > 0 && cur == max {
            // Progress complete; still require a date unless the UI only shows 达成.
        }
    }

    let date_clean: String = date
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '/' || *c == '-' || *c == '.')
        .collect();
    if date_clean.len() < 4 {
        return false;
    }
    date_clean.chars().filter(|c| c.is_ascii_digit()).count() >= 4
}

fn parse_slash_pair(text: &str) -> Option<(i32, i32)> {
    let mut parts = text.split('/');
    let a = parts.next()?.trim();
    let b = parts.next()?.trim();
    let cur: i32 = a
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    let max: i32 = b
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    Some((cur, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_when_status_is_achieved() {
        assert!(is_done("达成", ""));
        assert!(is_done("达成", "2024/01/02"));
    }

    #[test]
    fn not_done_when_progress_incomplete() {
        assert!(!is_done("3/10", "2024/01/02"));
        assert!(!is_done("0/1", ""));
    }

    #[test]
    fn done_when_date_looks_complete() {
        assert!(is_done("", "2024/05/01"));
        assert!(is_done("10/10", "2024-05-01"));
    }

    #[test]
    fn not_done_without_date() {
        assert!(!is_done("", ""));
        assert!(!is_done("???", "1"));
    }
}
