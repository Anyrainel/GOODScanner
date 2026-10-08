use super::{game_switcher, state::Lang, theme};
use crate::config::{Game, GameNavigation, ToolTab};
use eframe::egui;

pub const TITLEBAR_HEIGHT: f32 = 64.0;
pub const SIDEBAR_MIN_WIDTH: f32 = 148.0;

pub fn product_name() -> &'static str {
    if cfg!(feature = "capture") {
        "GGScanner"
    } else {
        "GGScannerOCR"
    }
}
pub fn site(game: Game) -> (&'static str, &'static str) {
    match game {
        Game::Genshin => ("GGArtifact", "https://ggartifact.com"),
        Game::StarRail => ("GGStarRail", "https://hsr.ggartifact.com"),
    }
}

pub struct Logos {
    pub genshin: egui::TextureHandle,
    pub star_rail: egui::TextureHandle,
}
impl Logos {
    pub fn load(ctx: &egui::Context) -> Self {
        fn load(ctx: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
            let rgba = image::load_from_memory(bytes)
                .expect("bundled game logo must be valid PNG")
                .into_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            ctx.load_texture(
                name,
                egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                egui::TextureOptions::LINEAR,
            )
        }
        Self {
            genshin: load(
                ctx,
                "genshin-site-logo",
                include_bytes!("../../../assets/genshin-logo.png"),
            ),
            star_rail: load(
                ctx,
                "star-rail-site-logo",
                include_bytes!("../../../assets/star-rail-logo.png"),
            ),
        }
    }
}

pub fn titlebar(
    ctx: &egui::Context,
    lang: &mut Lang,
    game: &mut Game,
    enabled: bool,
    logos: &Logos,
) {
    egui::TopBottomPanel::top("titlebar")
        .exact_height(TITLEBAR_HEIGHT)
        .frame(
            egui::Frame::none()
                .fill(theme::SURFACE)
                .inner_margin(egui::Margin::symmetric(12.0, 7.0)),
        )
        .show(ctx, |ui| {
            let drag = ui.interact(
                ui.max_rect(),
                ui.id().with("window-drag"),
                egui::Sense::click_and_drag(),
            );
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            if drag.double_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            } else if drag.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            ui.horizontal_centered(|ui| {
                ui.label(
                    egui::RichText::new("G")
                        .color(theme::ACCENT)
                        .size(23.0)
                        .strong(),
                );
                ui.label(egui::RichText::new(product_name()).size(16.0).strong());
                ui.add_space(16.0);
                ui.allocate_ui(egui::vec2(248.0, 36.0), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    game_switcher::show(ui, *lang, game, enabled, Some(logos));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::text_button(ui, "×", egui::vec2(36.0, 32.0), false)
                        .on_hover_text(lang.t("关闭", "Close"))
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if theme::text_button(
                        ui,
                        if maximized { "❐" } else { "□" },
                        egui::vec2(36.0, 32.0),
                        false,
                    )
                    .on_hover_text(lang.t("最大化 / 还原", "Maximize / Restore"))
                    .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    if theme::text_button(ui, "−", egui::vec2(36.0, 32.0), false)
                        .on_hover_text(lang.t("最小化", "Minimize"))
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                });
            });
        });
}

pub fn sidebar(ui: &mut egui::Ui, lang: &mut Lang, navigation: &mut GameNavigation) {
    let rect = ui.available_rect_before_wrap();
    let footer = egui::Rect::from_min_max(
        egui::pos2(rect.left(), (rect.bottom() - 124.0).max(rect.top())),
        rect.max,
    );
    let tabs = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), footer.top() - 12.0));
    super::layout::region(ui, tabs, "tool-tabs", |ui| {
        let width = tabs.width() - ui.spacing().scroll.allocated_width() - 4.0;
        egui::ScrollArea::vertical()
            .id_salt("navigation-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::none().inner_margin(2.0).show(ui, |ui| {
                    let width = (width - 4.0).max(1.0);
                    ui.set_width(width);
                    ui.visuals_mut().selection.bg_fill = theme::SELECTED;
                    ui.visuals_mut().selection.stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.hovered.bg_stroke =
                        egui::Stroke::new(1.0, theme::ACCENT);
                    ui.visuals_mut().widgets.active.bg_stroke =
                        egui::Stroke::new(1.0, theme::ACCENT);
                    ui.add_space(18.0);
                    let mut tab = navigation.active_tab();
                    let original = tab;
                    for (value, zh, en) in [
                        (ToolTab::Capture, "抓包器", "Capture"),
                        (ToolTab::Scanner, "扫描器", "Scanner"),
                        (ToolTab::Manager, "管理器", "Manager"),
                    ] {
                        if value == ToolTab::Capture && !cfg!(feature = "capture") {
                            continue;
                        }
                        let selected = tab == value;
                        let button = egui::SelectableLabel::new(
                            selected,
                            egui::RichText::new(lang.t(zh, en)).color(if selected {
                                theme::ACCENT
                            } else {
                                theme::MUTED
                            }),
                        );
                        if ui
                            .add_sized([width, 42.0], button)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            tab = value;
                        }
                    }
                    if tab != original {
                        navigation.select_tab(tab);
                    }
                });
            });
    });
    super::layout::region(ui, footer, "sidebar-footer", |ui| {
        egui::Frame::none().inner_margin(2.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            if theme::text_button(
                ui,
                lang.t("EN", "中"),
                egui::vec2(ui.available_width(), 30.0),
                false,
            )
            .on_hover_text(lang.t("Switch to English", "切换到中文"))
            .clicked()
            {
                *lang = match lang {
                    Lang::Zh => Lang::En,
                    Lang::En => Lang::Zh,
                };
            }
            if theme::text_button(
                ui,
                lang.t("关于", "About"),
                egui::vec2(ui.available_width(), 30.0),
                navigation.active_tab() == ToolTab::Credits,
            )
            .clicked()
            {
                navigation.select_tab(ToolTab::Credits);
            }
            let (label, url) = site(navigation.active_game);
            if theme::text_button(ui, label, egui::vec2(ui.available_width(), 30.0), false)
                .on_hover_text(url)
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        });
    });
}

pub fn window_resize(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let rect = ctx.screen_rect();
    let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let left = pointer.x < rect.left() + 5.0;
    let right = pointer.x > rect.right() - 5.0;
    let top = pointer.y < rect.top() + 5.0;
    let bottom = pointer.y > rect.bottom() - 5.0;
    let direction = match (left, right, top, bottom) {
        (true, _, true, _) => egui::ResizeDirection::NorthWest,
        (true, _, _, true) => egui::ResizeDirection::SouthWest,
        (_, true, true, _) => egui::ResizeDirection::NorthEast,
        (_, true, _, true) => egui::ResizeDirection::SouthEast,
        (true, _, _, _) => egui::ResizeDirection::West,
        (_, true, _, _) => egui::ResizeDirection::East,
        (_, _, true, _) => egui::ResizeDirection::North,
        (_, _, _, true) => egui::ResizeDirection::South,
        _ => return,
    };
    ctx.set_cursor_icon(match direction {
        egui::ResizeDirection::North | egui::ResizeDirection::South => {
            egui::CursorIcon::ResizeVertical
        },
        egui::ResizeDirection::East | egui::ResizeDirection::West => {
            egui::CursorIcon::ResizeHorizontal
        },
        egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => {
            egui::CursorIcon::ResizeNwSe
        },
        _ => egui::CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}
