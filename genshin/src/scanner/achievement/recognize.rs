//! OCR a single achievement row and decide whether it is completed.
//!
//! Field split + matching follow cocogoat `scannerOnLine` /
//! `recognizeAchievement`. Completion uses date / `达成` / `n/m` progress,
//! the same signals cocogoat reads from the status and date crops.

use anyhow::Result;
use image::RgbImage;
use yas::ocr::ImageToText;

use super::catalog::AchievementCatalog;
use super::layout::{STATUS_BAND, SUBTITLE_BAND};
use super::split::{crop_px, crop_title};

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
    category: Option<&str>,
) -> Result<RecognizedRow> {
    let title = ocr_title(ocr, row)?;
    let subtitle = ocr_band(ocr, row, SUBTITLE_BAND)?;
    let status = ocr_band(ocr, row, STATUS_BAND)?;
    let id = catalog.match_text_in_category(&title, &subtitle, category);
    let done = is_done(&status, &status);
    Ok(RecognizedRow {
        id,
        title,
        subtitle,
        status: status.clone(),
        date: status,
        done,
    })
}

fn ocr_title(ocr: &dyn ImageToText<RgbImage>, row: &RgbImage) -> Result<String> {
    let Some(crop) = crop_title(row) else {
        return Ok(String::new());
    };
    let text = ocr.image_to_text(&crop, false)?;
    Ok(text.trim().to_string())
}

fn ocr_band(
    ocr: &dyn ImageToText<RgbImage>,
    row: &RgbImage,
    band: (f32, u32, f32, u32),
) -> Result<String> {
    let Some(crop) = crop_px(row, band.0, band.1, band.2, band.3) else {
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
    let has_separator = date_clean.contains('/') || date_clean.contains('-') || date_clean.contains('.');
    if !has_separator || date_clean.len() < 6 {
        return false;
    }
    date_clean.chars().filter(|c| c.is_ascii_digit()).count() >= 4
}

/// `4/20` and `0/1` are still in progress. They share the corner where a
/// finished card prints its date, so digit ink alone is not completion.
pub fn progress_is_open(status: &str) -> bool {
    parse_slash_pair(status).is_some_and(|(cur, max)| cur < max)
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
        assert!(!is_done("20 0/1", "2001"));
        assert!(progress_is_open("4/20"));
        assert!(!progress_is_open("达成"));
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
