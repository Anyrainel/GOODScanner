use std::collections::HashMap;
use log::debug;

/// OCR confusion pairs: (wrong char, correct char)
/// When OCR produces one character, try substituting the other.
const OCR_CONFUSIONS: &[(&str, &str)] = &[
    ("\u{8332}", "\u{5179}"), // 茲 → 兹
    ("\u{5179}", "\u{8332}"), // 兹 → 茲
];

/// Fuzzy match OCR text against a name→key map.
///
/// Matching strategy (in order):
/// 1. OCR confusion substitution → exact match
/// 2. Exact match on cleaned text
/// 3. Substring match (longest match wins, both directions)
/// 4. Levenshtein distance fallback (threshold: 30% of name length)
///
/// Port of `fuzzyMatchMap()` from GOODScanner/lib/constants.js
pub fn fuzzy_match_map(text: &str, map: &HashMap<String, String>) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    // Clean OCR text: normalize lookalike characters, then keep CJK, alphanumeric, and 「」.
    // OCR engines sometimes output:
    //   - Fullwidth characters (U+FF01..U+FF5E) instead of ASCII
    //   - Cyrillic lookalikes (е=U+0435 instead of e=U+0065, etc.)
    // These must be normalized to ASCII before filtering, or they'll be stripped.
    let cleaned: String = text
        .chars()
        .map(|c| {
            // Fullwidth ASCII variants (U+FF01..U+FF5E) → halfwidth (U+0021..U+007E)
            if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
                return char::from_u32(c as u32 - 0xFF01 + 0x0021).unwrap_or(c);
            }
            // Cyrillic lookalikes → ASCII Latin
            match c {
                '\u{0410}' | '\u{0430}' => 'a', // А/а
                '\u{0412}' | '\u{0432}' => 'B', // В/в
                '\u{0415}' | '\u{0435}' => 'e', // Е/е — very common OCR confusion
                '\u{041A}' | '\u{043A}' => 'K', // К/к
                '\u{041C}' | '\u{043C}' => 'M', // М/м
                '\u{041D}' | '\u{043D}' => 'H', // Н/н
                '\u{041E}' | '\u{043E}' => 'o', // О/о
                '\u{0420}' | '\u{0440}' => 'p', // Р/р
                '\u{0421}' | '\u{0441}' => 'c', // С/с
                '\u{0422}' | '\u{0442}' => 'T', // Т/т
                '\u{0423}' | '\u{0443}' => 'y', // У/у
                '\u{0425}' | '\u{0445}' => 'x', // Х/х
                _ => c,
            }
        })
        .filter(|c| {
            matches!(*c, '\u{4E00}'..='\u{9FFF}' | '\u{300C}' | '\u{300D}' | 'a'..='z' | 'A'..='Z' | '0'..='9')
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        return None;
    }

    // Try OCR confusion substitutions
    for &(from, to) in OCR_CONFUSIONS {
        if cleaned.contains(from) {
            let alt = cleaned.replace(from, to);
            if let Some(val) = map.get(&alt) {
                return Some(val.clone());
            }
        }
    }

    // Exact match
    if let Some(val) = map.get(&cleaned) {
        debug!("[fuzzy] exact match: cleaned={:?} → {:?}", cleaned, val);
        return Some(val.clone());
    }

    // Substring match: prefer "cleaned contains map key" (OCR added noise around real name)
    // over "map key contains cleaned" (OCR truncated the name).
    let mut best_match: Option<String> = None;
    let mut best_len: usize = 0;

    for (cn, val) in map.iter() {
        // cleaned contains the map key — OCR returned name + noise
        if cleaned.contains(cn.as_str()) && cn.len() > best_len {
            best_match = Some(val.clone());
            best_len = cn.len();
        }
    }
    if best_match.is_some() {
        debug!("[fuzzy] substring match (cleaned⊃key): cleaned={:?} → {:?}", cleaned, best_match);
        return best_match;
    }

    // Fallback: map key contains cleaned — OCR truncated the name
    for (cn, val) in map.iter() {
        if cn.contains(cleaned.as_str()) && cleaned.len() > best_len {
            best_match = Some(val.clone());
            best_len = cleaned.len();
        }
    }
    if best_match.is_some() {
        debug!("[fuzzy] reverse substring (key⊃cleaned): cleaned={:?} → {:?}", cleaned, best_match);
        return best_match;
    }

    // Levenshtein distance fallback — character-level comparison for CJK
    let cleaned_chars: Vec<char> = cleaned.chars().collect();
    let mut min_dist = usize::MAX;
    let mut best_debug_name = String::new();
    for (cn, val) in map.iter() {
        let cn_chars: Vec<char> = cn.chars().collect();
        let dist = edit_distance_chars(&cleaned_chars, &cn_chars);
        // 30% threshold, min 1 for short strings
        let threshold = std::cmp::max(1, cn_chars.len() * 3 / 10);
        if dist < min_dist && dist <= threshold {
            min_dist = dist;
            best_match = Some(val.clone());
            best_debug_name = cn.clone();
        }
    }

    if best_match.is_some() {
        debug!("[fuzzy] Levenshtein match: cleaned={:?} → {:?} (name={:?}, dist={}, map_size={})",
            cleaned, best_match, best_debug_name, min_dist, map.len());
    } else {
        debug!("[fuzzy] NO MATCH: cleaned={:?} (chars={}, map_size={})",
            cleaned, cleaned_chars.len(), map.len());
    }
    best_match
}

/// Character-level Levenshtein distance (important for CJK where
/// byte-level distance gives misleading results).
fn edit_distance_chars(a: &[char], b: &[char]) -> usize {
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let mut map = HashMap::new();
        map.insert("神里绫华".to_string(), "KamisatoAyaka".to_string());
        assert_eq!(
            fuzzy_match_map("神里绫华", &map),
            Some("KamisatoAyaka".to_string())
        );
    }

    #[test]
    fn test_substring_match() {
        let mut map = HashMap::new();
        map.insert("神里绫华".to_string(), "KamisatoAyaka".to_string());
        // OCR might produce extra noise characters around the name
        assert_eq!(
            fuzzy_match_map("·神里绫华", &map),
            Some("KamisatoAyaka".to_string())
        );
    }

    #[test]
    fn test_no_match() {
        let map = HashMap::new();
        assert_eq!(fuzzy_match_map("随机文本", &map), None);
    }

    #[test]
    fn test_levenshtein_short_set_name() {
        let mut map = HashMap::new();
        map.insert("教官".to_string(), "Instructor".to_string());
        map.insert("战狂".to_string(), "Berserker".to_string());
        map.insert("赌徒".to_string(), "Gambler".to_string());
        // OCR misreads "教官" as "教e" — should match via Levenshtein (distance 1)
        assert_eq!(
            fuzzy_match_map("教e", &map),
            Some("Instructor".to_string())
        );
    }

    #[test]
    fn test_fullwidth_normalization() {
        let mut map = HashMap::new();
        map.insert("教官".to_string(), "Instructor".to_string());
        // OCR might output fullwidth 'ｅ' (U+FF45) instead of regular 'e' (U+0065)
        // This should still match after normalization
        assert_eq!(
            fuzzy_match_map("教\u{FF45}", &map),
            Some("Instructor".to_string())
        );
    }

    #[test]
    fn test_empty_input() {
        let map = HashMap::new();
        assert_eq!(fuzzy_match_map("", &map), None);
    }

    /// Reproduce the exact runtime conditions with all 58 artifact set entries.
    /// This test verifies that "教e" matches "教官" even with the full map.
    #[test]
    fn test_levenshtein_with_full_artifact_set_map() {
        let mut map = HashMap::new();
        // All 58 artifact sets from mappings.json
        let entries: &[(&str, &str)] = &[
            ("风起之日", "ADayCarvedFromRisingWinds"),
            ("悠古的磐岩", "ArchaicPetra"),
            ("晨星与月的晓歌", "AubadeOfMorningstarAndMoon"),
            ("战狂", "Berserker"),
            ("冰风迷途的勇士", "BlizzardStrayer"),
            ("染血的骑士道", "BloodstainedChivalry"),
            ("勇士之心", "BraveHeart"),
            ("炽烈的炎之魔女", "CrimsonWitchOfFlames"),
            ("深林的记忆", "DeepwoodMemories"),
            ("守护之心", "DefendersWill"),
            ("沙上楼阁史话", "DesertPavilionChronicle"),
            ("来歆余响", "EchoesOfAnOffering"),
            ("绝缘之旗印", "EmblemOfSeveredFate"),
            ("深廊终曲", "FinaleOfTheDeepGalleries"),
            ("乐园遗落之花", "FlowerOfParadiseLost"),
            ("谐律异想断章", "FragmentOfHarmonicWhimsy"),
            ("赌徒", "Gambler"),
            ("饰金之梦", "GildedDreams"),
            ("冰之川与雪之砂", "GlacierAndSnowfield"),
            ("角斗士的终幕礼", "GladiatorsFinale"),
            ("黄金剧团", "GoldenTroupe"),
            ("沉沦之心", "HeartOfDepth"),
            ("华馆梦醒形骸记", "HuskOfOpulentDreams"),
            ("教官", "Instructor"),
            ("渡过烈火的贤人", "Lavawalker"),
            ("长夜之誓", "LongNightsOath"),
            ("被怜爱的少女", "MaidenBeloved"),
            ("逐影猎人", "MarechausseeHunter"),
            ("武人", "MartialArtist"),
            ("穹境示现之夜", "NightOfTheSkysUnveiling"),
            ("回声之林夜话", "NighttimeWhispersInTheEchoingWoods"),
            ("昔日宗室之仪", "NoblesseOblige"),
            ("水仙之梦", "NymphsDream"),
            ("黑曜秘典", "ObsidianCodex"),
            ("海染砗磲", "OceanHuedClam"),
            ("苍白之火", "PaleFlame"),
            ("祭水之人", "PrayersForDestiny"),
            ("祭火之人", "PrayersForIllumination"),
            ("祭雷之人", "PrayersForWisdom"),
            ("祭冰之人", "PrayersToSpringtime"),
            ("祭风之人", "PrayersToTheFirmament"),
            ("行者之心", "ResolutionOfSojourner"),
            ("逆飞的流星", "RetracingBolide"),
            ("学士", "Scholar"),
            ("烬城勇者绘卷", "ScrollOfTheHeroOfCinderCity"),
            ("追忆之注连", "ShimenawasReminiscence"),
            ("纺月的夜歌", "SilkenMoonsSerenade"),
            ("昔时之歌", "SongOfDaysPast"),
            ("千岩牢固", "TenacityOfTheMillelith"),
            ("流放者", "TheExile"),
            ("如雷的盛怒", "ThunderingFury"),
            ("平息鸣雷的尊者", "Thundersoother"),
            ("奇迹", "TinyMiracle"),
            ("未竟的遐思", "UnfinishedReverie"),
            ("辰砂往生录", "VermillionHereafter"),
            ("翠绿之影", "ViridescentVenerer"),
            ("花海甘露之光", "VourukashasGlow"),
            ("流浪大地的乐团", "WanderersTroupe"),
        ];
        for &(zh, id) in entries {
            map.insert(zh.to_string(), id.to_string());
        }

        // The exact scenario that fails at runtime
        assert_eq!(
            fuzzy_match_map("教e", &map),
            Some("Instructor".to_string()),
            "教e should match 教官 via Levenshtein with full 58-entry map"
        );

        // Also test with the colon stripped (as find_set_key_in_text does)
        assert_eq!(
            fuzzy_match_map("教e", &map),
            Some("Instructor".to_string()),
        );

        // Other short set names should also work
        assert_eq!(
            fuzzy_match_map("战e", &map),
            Some("Berserker".to_string()),
            "战e should match 战狂 via Levenshtein"
        );
    }
}
