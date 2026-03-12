use regex::Regex;

/// Parse a number from OCR text. Returns the first integer found.
///
/// Port of `parseNumberFromText()` from GOODScanner/lib/ocr_utils.js
pub fn parse_number_from_text(text: &str) -> i32 {
    let re = Regex::new(r"(\d+)").unwrap();
    re.captures(text)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(0)
}

/// Parse "XX/YY" format and return the first number.
/// Falls back to extracting any number if the slash format isn't found.
///
/// Port of `parseSlashNumber()` from GOODScanner/lib/ocr_utils.js
pub fn parse_slash_number(text: &str) -> i32 {
    let re = Regex::new(r"(\d+)\s*/\s*(\d+)").unwrap();
    if let Some(caps) = re.captures(text) {
        caps[1].parse().unwrap_or(0)
    } else {
        parse_number_from_text(text)
    }
}

/// Parse "XX/YY" format and return both numbers.
/// Returns (current, max) or (0, 0) if parsing fails.
pub fn parse_slash_pair(text: &str) -> (i32, i32) {
    let re = Regex::new(r"(\d+)\s*/\s*(\d+)").unwrap();
    if let Some(caps) = re.captures(text) {
        let current: i32 = caps[1].parse().unwrap_or(0);
        let max: i32 = caps[2].parse().unwrap_or(0);
        (current, max)
    } else {
        (0, 0)
    }
}
