use image::Rgb;

/// Maps a Chinese character name to the GOOD format key.
///
/// Returns the GOOD key string if known, or an empty string for unrecognized names.
pub fn zh_cn_to_good_key(name: &str) -> &'static str {
    match name {
        "旅行者" => "Traveler",
        "神里绫华" => "KamisatoAyaka",
        "琴" => "Jean",
        "丽莎" => "Lisa",
        "芭芭拉" => "Barbara",
        "凯亚" => "Kaeya",
        "迪卢克" => "Diluc",
        "雷泽" => "Razor",
        "安柏" => "Amber",
        "温迪" => "Venti",
        "香菱" => "Xiangling",
        "北斗" => "Beidou",
        "行秋" => "Xingqiu",
        "魈" => "Xiao",
        "凝光" => "Ningguang",
        "可莉" => "Klee",
        "钟离" => "Zhongli",
        "菲谢尔" => "Fischl",
        "班尼特" => "Bennett",
        "达达利亚" => "Tartaglia",
        "诺艾尔" => "Noelle",
        "七七" => "Qiqi",
        "重云" => "Chongyun",
        "甘雨" => "Ganyu",
        "阿贝多" => "Albedo",
        "迪奥娜" => "Diona",
        "莫娜" => "Mona",
        "刻晴" => "Keqing",
        "砂糖" => "Sucrose",
        "辛焱" => "Xinyan",
        "罗莎莉亚" => "Rosaria",
        "胡桃" => "HuTao",
        "枫原万叶" => "KaedeharaKazuha",
        "烟绯" => "Yanfei",
        "宵宫" => "Yoimiya",
        "托马" => "Thoma",
        "优菈" => "Eula",
        "雷电将军" => "RaidenShogun",
        "早柚" => "Sayu",
        "珊瑚宫心海" => "SangonomiyaKokomi",
        "五郎" => "Gorou",
        "九条裟罗" => "KujouSara",
        "荒泷一斗" => "AratakiItto",
        "八重神子" => "YaeMiko",
        "鹿野院平藏" => "ShikanoinHeizou",
        "夜兰" => "Yelan",
        "绮良良" => "Kirara",
        "埃洛伊" => "Aloy",
        "申鹤" => "Shenhe",
        "云堇" => "YunJin",
        "久岐忍" => "KukiShinobu",
        "神里绫人" => "KamisatoAyato",
        "柯莱" => "Collei",
        "多莉" => "Dori",
        "提纳里" => "Tighnari",
        "妮露" => "Nilou",
        "赛诺" => "Cyno",
        "坎蒂丝" => "Candace",
        "纳西妲" => "Nahida",
        "莱依拉" => "Layla",
        "流浪者" => "Wanderer",
        "珐露珊" => "Faruzan",
        "瑶瑶" => "Yaoyao",
        "艾尔海森" => "Alhaitham",
        "迪希雅" => "Dehya",
        "米卡" => "Mika",
        "卡维" => "Kaveh",
        "白术" => "Baizhu",
        "琳妮特" => "Lynette",
        "林尼" => "Lyney",
        "菲米尼" => "Freminet",
        "那维莱特" => "Neuvillette",
        "莱欧斯利" => "Wriothesley",
        "夏洛蒂" => "Charlotte",
        "芙宁娜" => "Furina",
        "夏沃蕾" => "Chevreuse",
        "娜维娅" => "Navia",
        "嘉明" => "Gaming",
        "闲云" => "Xianyun",
        "千织" => "Chiori",
        "阿蕾奇诺" => "Arlecchino",
        "希格雯" => "Sigewinne",
        "赛索斯" => "Sethos",
        "克洛琳德" => "Clorinde",
        "艾梅莉埃" => "Emilie",
        // Characters from CHARACTER_NAMES not in original equip_from_zh_cn
        "卡齐娜" => "Kachina",
        "玛拉妮" => "Mualani",
        "基尼奇" => "Kinich",
        "希诺宁" => "Xilonen",
        "恰斯卡" => "Chasca",
        "玛薇卡" => "Mavuika",
        "茜特菈莉" => "Citlali",
        "梦见月瑞希" => "Mizuki",
        "瓦雷莎" => "Varesa",
        "丝柯克" => "Skirk",
        "伊涅芙" => "Iansan",
        "奇偶" => "QiQi",
        "菈乌玛" => "Lauma",
        "菲林斯" => "Phrynis",
        "奈芙尔" => "Naflah",
        "杜林" => "Durin",
        "哥伦比娅" => "Columba",
        "欧洛伦" => "Ororon",
        "蓝砚" => "LanYan",
        "伊安珊" => "Iansan",
        "伊法" => "Ifa",
        "塔利雅" => "Taliyah",
        "爱诺" => "Aino",
        "雅珂达" => "Yacoda",
        _ => "",
    }
}

/// Maps a Chinese element name to the English GOOD format element name.
pub fn element_from_zh_cn(element: &str) -> Option<&'static str> {
    match element {
        "火" => Some("Pyro"),
        "水" => Some("Hydro"),
        "雷" => Some("Electro"),
        "冰" => Some("Cryo"),
        "风" => Some("Anemo"),
        "岩" => Some("Geo"),
        "草" => Some("Dendro"),
        _ => None,
    }
}

/// Detects a Genshin Impact element from a pixel color.
///
/// Uses heuristic thresholds on the RGB channels to classify the color
/// into one of the seven elements. Returns `None` if no element matches.
pub fn element_from_color(color: &Rgb<u8>) -> Option<&'static str> {
    let [r, g, b] = color.0;

    let rf = r as f32;
    let gf = g as f32;
    let bf = b as f32;

    // Pyro: reddish (high R, low G and B)
    if rf > 180.0 && gf < 100.0 && bf < 100.0 {
        return Some("Pyro");
    }

    // Electro: purple (high R, low G, high B)
    if rf > 140.0 && gf < 100.0 && bf > 140.0 {
        return Some("Electro");
    }

    // Hydro: blue (low R, low-mid G, high B)
    if rf < 100.0 && gf < 150.0 && bf > 180.0 {
        return Some("Hydro");
    }

    // Cryo: light blue / cyan (mid-high R, high G, high B)
    if rf > 100.0 && rf < 200.0 && gf > 200.0 && bf > 220.0 {
        return Some("Cryo");
    }

    // Dendro: green (low R, high G, low-mid B)
    if rf < 120.0 && gf > 160.0 && bf < 120.0 {
        return Some("Dendro");
    }

    // Anemo: teal / green-cyan (low-mid R, high G, mid-high B)
    if rf < 130.0 && gf > 200.0 && bf > 150.0 && bf < 220.0 {
        return Some("Anemo");
    }

    // Geo: yellow / brown (high R, mid-high G, low B)
    if rf > 180.0 && gf > 140.0 && gf < 220.0 && bf < 100.0 {
        return Some("Geo");
    }

    None
}
