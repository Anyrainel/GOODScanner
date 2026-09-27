//! System CJK font discovery, shared by the GUI (egui) and dump annotations
//! (ab_glyph). Uses fontdb to scan system font locations and query by family
//! name — which covers fontconfig custom dirs, Flatpak and other distro
//! differences — with the old hardcoded candidate paths kept as a last
//! resort.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A located font file plus the face index inside it (non-zero for .ttc
/// collections).
#[derive(Clone, Debug)]
pub struct SystemFont {
    pub path: PathBuf,
    pub index: u32,
}

/// Preferred CJK families, best first. English and native names cover the
/// same fonts across distros and locales.
const CJK_FAMILIES: &[&str] = &[
    "Microsoft YaHei",
    "微软雅黑",
    "Noto Sans CJK SC",
    "Source Han Sans SC",
    "思源黑体",
    "WenQuanYi Micro Hei",
    "文泉驿微米黑",
    "WenQuanYi Zen Hei",
    "Noto Sans CJK",
    "SimSun",
    "宋体",
    "SimHei",
    "黑体",
    "Droid Sans Fallback",
];

static CJK_FONT: OnceLock<Option<SystemFont>> = OnceLock::new();

/// Find a system font suitable for CJK text. Cached after the first call.
pub fn find_cjk_font() -> Option<&'static SystemFont> {
    CJK_FONT.get_or_init(locate_cjk_font).as_ref()
}

fn locate_cjk_font() -> Option<SystemFont> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    for family in CJK_FAMILIES {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            ..Default::default()
        };
        let Some(id) = db.query(&query) else {
            continue;
        };
        let Some(face) = db.face(id) else {
            continue;
        };
        if let fontdb::Source::File(path) = &face.source {
            log_debug!(
                "CJK 字体: {} (index {}, family {:?})",
                "CJK font: {} (index {}, family {:?})",
                path.display(),
                face.index,
                family,
            );
            return Some(SystemFont {
                path: path.clone(),
                index: face.index,
            });
        }
    }

    for (path, index) in legacy_fallback_paths() {
        if Path::new(path).is_file() {
            log_debug!(
                "CJK 字体(兜底路径): {}",
                "CJK font (fallback path): {}",
                path,
            );
            return Some(SystemFont {
                path: PathBuf::from(path),
                index: *index,
            });
        }
    }

    log_warn!(
        "未找到可用的 CJK 字体，中文文本将无法渲染/标注。",
        "No usable CJK font was found; Chinese text cannot be rendered/annotated."
    );
    None
}

/// Direct paths for environments where the fontdb scan comes up empty.
fn legacy_fallback_paths() -> &'static [(&'static str, u32)] {    #[cfg(target_os = "windows")]
    {
        &[
            ("C:/Windows/Fonts/msyh.ttc", 0),   // Microsoft YaHei (微软雅黑)
            ("C:/Windows/Fonts/msyhbd.ttc", 0), // Microsoft YaHei Bold
            ("C:/Windows/Fonts/simsun.ttc", 0), // SimSun (宋体)
            ("C:/Windows/Fonts/simhei.ttf", 0), // SimHei (黑体)
        ]
    }
    #[cfg(not(target_os = "windows"))]
    {
        &[
            ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
            ("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc", 0),
            ("/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc", 0),
            ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
            ("/usr/share/fonts/gsfonts/DroidSansFallback.ttf", 0),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Must never panic; asserts a real font only when the environment has
    /// one (CI images may ship no CJK fonts).
    #[test]
    fn cjk_font_discovery_does_not_panic() {
        if let Some(font) = find_cjk_font() {
            assert!(font.path.is_file(), "discovered font must exist: {}", font.path.display());
        }
    }
}
