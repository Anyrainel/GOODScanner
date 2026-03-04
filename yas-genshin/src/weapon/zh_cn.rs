use std::collections::HashMap;

use edit_distance::edit_distance;
use lazy_static::lazy_static;

lazy_static! {
    static ref WEAPON_MAP: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();

        // 5-star weapons
        m.insert("天空之翼", "SkywardHarp");
        m.insert("阿莫斯之弓", "AmosBow");
        m.insert("风鹰剑", "AquilaFavonia");
        m.insert("天空之刃", "SkywardBlade");
        m.insert("磐岩结绿", "PrimordialJadeCutter");
        m.insert("斫峰之刃", "SummitShaper");
        m.insert("天空之傲", "SkywardPride");
        m.insert("狼的末路", "WolfsGravestone");
        m.insert("天空之脊", "SkywardSpine");
        m.insert("和璞鸢", "PrimordialJadeWingedSpear");
        m.insert("天空之卷", "SkywardAtlas");
        m.insert("四风原典", "LostPrayerToTheSacredWinds");
        m.insert("尘世之锁", "MemoryOfDust");
        m.insert("飞雷之弦振", "ThunderingPulse");
        m.insert("冬极白星", "PolarStar");
        m.insert("雾切之回光", "MistsplitterReforged");
        m.insert("波乱月白经津", "HaranGeppakuFutsu");
        m.insert("赤角石溃杵", "RedhornStonethresher");
        m.insert("松籁响起之时", "SongOfBrokenPines");
        m.insert("薙草之稻光", "EngulfingLightning");
        m.insert("息灾", "CalamityQueller");
        m.insert("护摩之杖", "StaffOfHoma");
        m.insert("神乐之真意", "KagurasVerity");
        m.insert("不灭月华", "EverlastingMoonglow");
        m.insert("若水", "AquaSimulacra");
        m.insert("猎人之径", "HuntersPath");
        m.insert("终末嗟叹之诗", "ElegyForTheEnd");
        m.insert("万叶之一刀", "FreedomSworn");
        m.insert("苍古自由之誓", "FreedomSworn");
        m.insert("圣显之钥", "KeyOfKhajNisut");
        m.insert("图莱杜拉的回忆", "TulaytullahsRemembrance");
        m.insert("千夜浮梦", "AThousandFloatingDreams");
        m.insert("裁叶萃光", "LightOfFoliarIncision");
        m.insert("碧落之珑", "JadefallsSplendor");
        m.insert("始基力场转换器", "TheFirstGreatMagic");
        m.insert("最初的大魔术", "TheFirstGreatMagic");
        m.insert("金流监督", "CashflowSupervision");
        m.insert("赦罪", "Absolution");
        m.insert("鹤鸣余音", "CranesEchoingCall");
        m.insert("裁断", "UrakuMisugiri");
        m.insert("有乐御簾切", "UrakuMisugiri");
        m.insert("白雨心相", "SplendorOfTranquilWaters");
        m.insert("静水流涌之辉", "SplendorOfTranquilWaters");
        m.insert("万世流涌大典", "TomeOfTheEternalFlow");
        m.insert("赤月之形", "CrimsonMoonsSemblance");
        m.insert("赤沙之杖", "StaffOfTheScarletSands");
        m.insert("驭浪的回忆", "SurfsUp");

        // 4-star weapons
        m.insert("匣里灭辰", "LionsRoar");
        m.insert("笛剑", "TheFlute");
        m.insert("祭礼剑", "SacrificialSword");
        m.insert("西风剑", "FavoniusSword");
        m.insert("黑剑", "TheBlackSword");
        m.insert("暗巷闪光", "TheAlleyFlash");
        m.insert("黑岩长剑", "BlackcliffLongsword");
        m.insert("试作斩岩", "PrototypeRancour");
        m.insert("腐殖之剑", "FesteringDesire");
        m.insert("降临之剑", "DescendingBlade");
        m.insert("匣里龙吟", "DragonsBane");
        m.insert("雨裁", "Rainslasher");
        m.insert("祭礼大剑", "SacrificialGreatsword");
        m.insert("西风大剑", "FavoniusGreatsword");
        m.insert("钟剑", "TheBell");
        m.insert("螭骨剑", "SerpentSpine");
        m.insert("黑岩斩刀", "BlackcliffSlasher");
        m.insert("试作古华", "PrototypeArchaic");
        m.insert("白影剑", "Whiteblind");
        m.insert("决斗之枪", "Deathmatch");
        m.insert("西风长枪", "FavoniusLance");
        m.insert("流月针", "CrescentPike");
        m.insert("试作星镰", "PrototypeStarglitter");
        m.insert("黑岩刺枪", "BlackcliffPole");
        m.insert("流浪乐章", "TheWidsith");
        m.insert("祭礼残章", "SacrificialFragments");
        m.insert("西风秘典", "FavoniusCodex");
        m.insert("匣里日月", "SolarPearl");
        m.insert("黑岩绯玉", "BlackcliffAgate");
        m.insert("试作金珀", "PrototypeAmber");
        m.insert("弓藏", "Rust");
        m.insert("祭礼弓", "SacrificialBow");
        m.insert("西风猎弓", "FavoniusWarbow");
        m.insert("绝弦", "TheStringless");
        m.insert("黑岩战弓", "BlackcliffWarbow");
        m.insert("试作澹月", "PrototypeCrescent");
        m.insert("钢轮弓", "CompoundBow");

        // 3-star weapons
        m.insert("黎明神剑", "HarbingerOfDawn");
        m.insert("冷刃", "CoolSteel");
        m.insert("飞天御剑", "SkyriderSword");
        m.insert("铁影阔剑", "IronSting");
        m.insert("以理服人", "DebateClub");
        m.insert("沐浴龙血的剑", "BloodtaintedGreatsword");
        m.insert("白铁大剑", "WhiteIronGreatsword");
        m.insert("黑缨枪", "BlackTassel");
        m.insert("白缨枪", "WhiteTassel");
        m.insert("翡玉法球", "EmeraldOrb");
        m.insert("讨龙英杰谭", "ThrillingTalesOfDragonSlayers");
        m.insert("弹弓", "Slingshot");
        m.insert("鸦羽弓", "RavenBow");
        m.insert("反曲弓", "RecurveBow");

        // 1-2 star weapons
        m.insert("无锋剑", "DullBlade");
        m.insert("银剑", "SilverSword");
        m.insert("训练大剑", "WasterGreatsword");
        m.insert("佣兵重剑", "OldMercsPal");
        m.insert("新手长枪", "BeginnersProtector");
        m.insert("铁尖枪", "IronPoint");
        m.insert("学徒笔记", "ApprenticesNotes");
        m.insert("口袋魔导书", "PocketGrimoire");
        m.insert("猎弓", "HuntersBow");
        m.insert("历练的猎弓", "SeasonedHuntersBow");

        m
    };
}

/// Look up a Chinese weapon name and return the corresponding GOOD format key.
pub fn weapon_from_zh_cn(name: &str) -> Option<&'static str> {
    WEAPON_MAP.get(name).copied()
}

/// Find the closest matching weapon name using edit distance for OCR error tolerance.
/// Returns the GOOD format key of the closest match, or `None` if no match is close enough.
pub fn find_closest_weapon(name: &str) -> Option<&'static str> {
    let mut min_distance = usize::MAX;
    let mut best_match: Option<&'static str> = None;

    for (&zh_name, &good_name) in WEAPON_MAP.iter() {
        let dist = edit_distance(name, zh_name);
        if dist < min_distance {
            min_distance = dist;
            best_match = Some(good_name);
        }
    }

    // Only accept matches with a small enough edit distance.
    // Threshold: at most 2 character edits for short names, scale for longer names.
    let threshold = (name.chars().count() / 3).max(1);
    if min_distance <= threshold {
        best_match
    } else {
        None
    }
}

/// Convert a Chinese weapon name to its GOOD format key.
/// First tries an exact match, then falls back to fuzzy matching.
pub fn weapon_name_to_good(name: &str) -> Option<&'static str> {
    weapon_from_zh_cn(name).or_else(|| find_closest_weapon(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert_eq!(weapon_from_zh_cn("天空之翼"), Some("SkywardHarp"));
        assert_eq!(weapon_from_zh_cn("狼的末路"), Some("WolfsGravestone"));
        assert_eq!(weapon_from_zh_cn("无锋剑"), Some("DullBlade"));
        assert_eq!(weapon_from_zh_cn("不存在的武器"), None);
    }

    #[test]
    fn test_fuzzy_match() {
        // Simulating a slight OCR error
        assert_eq!(weapon_name_to_good("天空之翼"), Some("SkywardHarp"));
    }

    #[test]
    fn test_weapon_name_to_good() {
        assert_eq!(weapon_name_to_good("护摩之杖"), Some("StaffOfHoma"));
        assert_eq!(weapon_name_to_good("猎弓"), Some("HuntersBow"));
    }
}
