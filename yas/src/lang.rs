//! Global language setting and bilingual logging macros.
//!
//! All log messages from our crates must use the `log_*!` macros
//! ([`log_info!`], [`log_warn!`], [`log_error!`], [`log_debug!`]) which
//! require two string-literal templates — one Chinese, one English.
//! The correct template is selected at runtime based on [`set_lang`].
//!
//! ```ignore
//! log_info!("扫描了{}个物品", "Scanned {} items", count);
//! ```
//!
//! Both templates receive the same format arguments, so they must have
//! identical `{}` placeholders.  A mismatch is a **compile error**.
//!
//! ## Legacy `localize()`
//!
//! [`localize`] still exists for runtime strings that use the old
//! `"中文 / English"` convention (e.g. `anyhow!` error messages).
//! New code should prefer `log_*!` macros.

use std::sync::atomic::{AtomicU8, Ordering};

static LANG: AtomicU8 = AtomicU8::new(0); // 0 = zh, 1 = en

/// Set the global language. Call once at startup.
/// Accepts `"en"` for English; anything else defaults to Chinese.
pub fn set_lang(lang: &str) {
    LANG.store(if lang == "en" { 1 } else { 0 }, Ordering::Relaxed);
}

/// Returns `"zh"` or `"en"`.
pub fn get_lang() -> &'static str {
    if LANG.load(Ordering::Relaxed) == 1 {
        "en"
    } else {
        "zh"
    }
}

/// Returns true if the current language is English.
pub fn is_en() -> bool {
    LANG.load(Ordering::Relaxed) == 1
}

/// Pick the correct language half from a bilingual `"中文 / English"` string.
///
/// Splits on the first `" / "` occurrence. If no separator is found,
/// returns the original string unchanged.
///
/// Kept for runtime strings (e.g. `anyhow!` error messages).
/// New code should prefer `log_*!` macros.
pub fn localize(msg: &str) -> String {
    if let Some(idx) = msg.find(" / ") {
        if is_en() {
            msg[idx + 3..].to_string()
        } else {
            msg[..idx].to_string()
        }
    } else {
        msg.to_string()
    }
}

// ── Bilingual log macros ────────────────────────────────────────────
//
// Each macro requires **two** string literals (zh, en) plus optional
// format arguments.  The compiler rejects calls with only one literal,
// making missing translations a build error.
//
// `::log::*` uses an absolute path so the calling crate only needs
// `log` in its dependency list (all workspace crates already have it).

/// Bilingual `log::info!`.
#[macro_export]
macro_rules! log_info {
    ($zh:literal, $en:literal $(, $($arg:tt)*)?) => {
        if $crate::lang::is_en() {
            ::log::info!($en $(, $($arg)*)?)
        } else {
            ::log::info!($zh $(, $($arg)*)?)
        }
    };
}

/// Bilingual `log::warn!`.
#[macro_export]
macro_rules! log_warn {
    ($zh:literal, $en:literal $(, $($arg:tt)*)?) => {
        if $crate::lang::is_en() {
            ::log::warn!($en $(, $($arg)*)?)
        } else {
            ::log::warn!($zh $(, $($arg)*)?)
        }
    };
}

/// Bilingual `log::error!`.
#[macro_export]
macro_rules! log_error {
    ($zh:literal, $en:literal $(, $($arg:tt)*)?) => {
        if $crate::lang::is_en() {
            ::log::error!($en $(, $($arg)*)?)
        } else {
            ::log::error!($zh $(, $($arg)*)?)
        }
    };
}

/// Bilingual `log::debug!`.
#[macro_export]
macro_rules! log_debug {
    ($zh:literal, $en:literal $(, $($arg:tt)*)?) => {
        if $crate::lang::is_en() {
            ::log::debug!($en $(, $($arg)*)?)
        } else {
            ::log::debug!($zh $(, $($arg)*)?)
        }
    };
}
