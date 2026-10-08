use eframe::egui;

use super::state::Lang;

/// Which set of credits to display.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CreditSet {
    /// GGScannerOCR: OCR-based scanning credits (no capture libs).
    Scanner,
    /// Packet-capture credits only.
    Capture,
    /// GGScanner: scanner + capture attributions.
    Full,
    /// Honkai: Star Rail scanner, capture, and manager references.
    StarRail,
}

/// Shared About page; every attribution is retained in its game column.
pub fn show(ui: &mut egui::Ui, l: Lang, set: CreditSet) {
    let width = (ui.available_width() - ui.spacing().scroll.allocated_width() - 4.0).max(1.0);
    egui::Frame::none().inner_margin(2.0).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(width);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal_wrapped(|ui| {
                    ui.heading(super::shell::product_name());
                    ui.label(
                        egui::RichText::new(genshin_scanner::updater::current_version_display())
                            .color(super::theme::MUTED),
                    );
                    ui.hyperlink_to("GitHub", "https://github.com/Anyrainel/GOODScanner");
                });
                ui.add_space(8.0);
                ui.strong(l.t("开源致谢", "Credits"));
                if width >= 640.0 && set != CreditSet::Capture {
                    ui.columns(2, |columns| {
                        genshin_credits(&mut columns[0], l, set);
                        star_rail_credits(&mut columns[1], l, set);
                    });
                } else {
                    genshin_credits(ui, l, set);
                    if set != CreditSet::Capture {
                        star_rail_credits(ui, l, set);
                    }
                }
                ui.add_space(8.0);
                ui.separator();
                licenses(ui, l);
            });
    });
}

fn genshin_credits(ui: &mut egui::Ui, l: Lang, set: CreditSet) {
    ui.label(
        egui::RichText::new(l.t("原神", "Genshin"))
            .strong()
            .color(super::theme::ACCENT),
    );
    if matches!(
        set,
        CreditSet::Scanner | CreditSet::Full | CreditSet::StarRail
    ) {
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
}

fn star_rail_credits(ui: &mut egui::Ui, l: Lang, set: CreditSet) {
    ui.label(
        egui::RichText::new(l.t("星穹铁道", "Star Rail"))
            .strong()
            .color(super::theme::ACCENT),
    );
    if matches!(
        set,
        CreditSet::StarRail | CreditSet::Full | CreditSet::Scanner
    ) {
        #[cfg(feature = "capture")]
        entry(
            ui,
            l,
            "Reliquary",
            "IceDynamix contributors",
            "https://github.com/IceDynamix/reliquary",
            l.t(
                "星穹铁道网络数据解密与角色、库存协议类型",
                "Star Rail network decryption and character/inventory protocol types",
            ),
        );
        entry(
            ui,
            l,
            "Reliquary Archiver",
            "IceDynamix contributors",
            "https://github.com/IceDynamix/reliquary-archiver",
            l.t(
                "星穹铁道导出格式与技能映射参考 (MIT)",
                "Reference for Star Rail export formats and skill mappings (MIT)",
            ),
        );
        entry(
            ui,
            l,
            "Fribbels Star Rail Optimizer",
            "Fribbels contributors",
            "https://github.com/fribbels/hsr-optimizer",
            l.t(
                "星穹铁道导出格式的互操作参考 (MIT)",
                "Interoperability reference for Star Rail export formats (MIT)",
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
}

fn licenses(ui: &mut egui::Ui, l: Lang) {
    egui::CollapsingHeader::new(l.t("许可证与版权声明", "Licenses & copyright notices")).show(
        ui,
        |ui| {
            let mut notices = include_str!("../../../THIRD_PARTY_NOTICES.md");
            ui.add(
                egui::TextEdit::multiline(&mut notices)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            let mut hsr_notices = include_str!("../../../experimental/hsr/THIRD_PARTY_NOTICES.md");
            ui.add(
                egui::TextEdit::multiline(&mut hsr_notices)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            #[cfg(feature = "capture")]
            {
                let mut license = include_str!("../../../experimental/hsr/LICENSE-reliquary.txt");
                ui.add(
                    egui::TextEdit::multiline(&mut license)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            }
            let mut archiver_license =
                include_str!("../../../experimental/hsr/LICENSE-reliquary-archiver.txt");
            ui.add(
                egui::TextEdit::multiline(&mut archiver_license)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
        },
    );
}

fn entry(ui: &mut egui::Ui, _l: Lang, name: &str, author: &str, url: &str, description: &str) {
    let width = ui.available_width();
    egui::Frame::none()
        .fill(super::theme::SURFACE)
        .stroke(egui::Stroke::new(1.0, super::theme::BORDER))
        .rounding(8.0)
        .inner_margin(12.0)
        .outer_margin(2.0)
        .show(ui, |ui| {
            ui.set_width((width - 28.0).max(1.0));
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to(egui::RichText::new(name).strong().size(14.0), url);
                ui.label(
                    egui::RichText::new(author)
                        .size(12.0)
                        .color(super::theme::MUTED),
                );
            });
            ui.add(egui::Label::new(egui::RichText::new(description).size(12.0)).wrap());
        });
}
