use eframe::egui;

use super::state::Lang;

/// Which set of credits to display.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CreditSet {
    /// GOODScanner: OCR-based scanning credits (no capture libs).
    Scanner,
    /// Packet-capture credits only.
    Capture,
    /// GOODCapture: scanner + capture attributions.
    Full,
    /// Honkai: Star Rail scanner, import, capture, and manager references.
    StarRail,
}

/// Render the credits / third-party attribution panel.
pub fn show(ui: &mut egui::Ui, l: Lang, set: CreditSet) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;

        ui.label(
            egui::RichText::new(l.t(
                "本软件使用了以下开源项目的代码，在此表示感谢。",
                "This software incorporates code from the following open-source projects.",
            ))
            .size(13.0),
        );
        ui.add_space(4.0);

        if matches!(set, CreditSet::Scanner | CreditSet::Full | CreditSet::StarRail) {
            entry(
                ui,
                l,
                "yas",
                "wormtql",
                "https://github.com/wormtql/yas",
                l.t(
                    "基础平台控制、屏幕捕获与 OCR（原始项目）",
                    "Base platform control, screen capture, and OCR (original project)",
                ),
            );

            entry(
                ui,
                l,
                "yas",
                "1803233552",
                "https://github.com/1803233552/yas",
                l.t(
                    "基础平台控制、屏幕捕获与 OCR（分支版本）",
                    "Base platform control, screen capture, and OCR (fork)",
                ),
            );
        }

        if matches!(set, CreditSet::Capture | CreditSet::Full) {
            entry(
                ui,
                l,
                "Irminsul",
                "Erik Gilling (konkers)",
                "https://github.com/konkers/irminsul",
                l.t(
                    "抓包扫描方案与数据导出逻辑 (MIT)",
                    "Packet capture scanning approach and data export logic (MIT)",
                ),
            );

            entry(
                ui,
                l,
                "auto-artifactarium",
                "IceDynamix",
                "https://github.com/konkers/auto-artifactarium",
                l.t(
                    "游戏数据包解密与协议解析 (MIT)",
                    "Game packet decryption and protocol parsing (MIT)",
                ),
            );
        }

        if matches!(set, CreditSet::Scanner | CreditSet::Full) {
            entry(
                ui,
                l,
                "Inventory Kamera",
                "Andrewthe13th",
                "https://github.com/Andrewthe13th/Inventory_Kamera",
                l.t(
                    "部分控制方法的灵感来源 (MIT)",
                    "Inspiration for some control methods (MIT)",
                ),
            );
        }

        if set == CreditSet::StarRail {
            #[cfg(feature = "capture")]
            entry(
                ui,
                l,
                "auto-reliquary",
                "Hashblen contributors",
                "https://github.com/hashblen/auto-reliquary",
                l.t(
                    "星穹铁道成就数据包解密与协议解析",
                    "Star Rail achievement packet decryption and protocol parsing",
                ),
            );
            entry(
                ui,
                l,
                "Reliquary",
                "IceDynamix contributors",
                "https://github.com/IceDynamix/reliquary",
                l.t(
                    "离线存档导入格式的互操作参考 (MIT)",
                    "Interoperability reference for offline archive imports (MIT)",
                ),
            );
            entry(
                ui,
                l,
                "Fribbels Star Rail Optimizer",
                "Fribbels contributors",
                "https://github.com/fribbels/hsr-optimizer",
                l.t(
                    "星穹铁道导入格式的互操作参考 (MIT)",
                    "Interoperability reference for Star Rail import formats (MIT)",
                ),
            );
            entry(
                ui,
                l,
                "YAS Star Rail",
                "wormtql",
                "https://github.com/wormtql/yas/tree/730d8845505c04c7f4c91709de076503401ff40c/yas-starrail",
                l.t(
                    "星穹铁道扫描行为的历史参考（未复制代码或资源）",
                    "Historical reference for Star Rail scanning behavior (no code or assets copied)",
                ),
            );
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(
            egui::RichText::new(l.t(
                "完整许可证文本请查看 THIRD_PARTY_NOTICES.md",
                "Full license texts are in THIRD_PARTY_NOTICES.md",
            ))
            .weak()
            .size(11.0),
        );
    });
}

fn entry(ui: &mut egui::Ui, l: Lang, name: &str, author: &str, url: &str, description: &str) {
    ui.group(|ui| {
        ui.label(egui::RichText::new(name).strong().size(14.0));
        ui.label(egui::RichText::new(format!("{}: {}", l.t("作者", "Author"), author)).size(12.0));
        ui.label(egui::RichText::new(description).size(12.0));
        ui.hyperlink_to(egui::RichText::new(url).size(11.0), url);
    });
}
