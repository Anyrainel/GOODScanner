use eframe::egui;

use crate::config::Game;

use super::state::Lang;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameSwitcherLayout {
    pub genshin_width: f32,
    pub star_rail_width: f32,
}

/// Render the product-level game choice as two framed, equally sized buttons.
/// Both choices look clickable at rest; the selected game uses egui's native
/// selected-button treatment rather than relying on hover alone.
pub fn show(
    ui: &mut egui::Ui,
    lang: Lang,
    active_game: &mut Game,
    enabled: bool,
) -> GameSwitcherLayout {
    let gap = ui.spacing().item_spacing.x;
    let button_width = ((ui.available_width() - gap) / 2.0).max(1.0);
    let button_height = ui.spacing().interact_size.y.max(34.0);

    let mut genshin_width = 0.0;
    let mut star_rail_width = 0.0;
    ui.horizontal(|ui| {
        let genshin = ui.add_enabled(
            enabled,
            egui::Button::new(egui::RichText::new(lang.t("原神", "Genshin")).size(17.0))
                .selected(*active_game == Game::Genshin)
                .min_size(egui::vec2(button_width, button_height)),
        );
        genshin_width = genshin.rect.width();
        if genshin.clicked() {
            *active_game = Game::Genshin;
        }

        let star_rail = ui.add_enabled(
            enabled,
            egui::Button::new(egui::RichText::new(lang.t("星穹铁道", "Star Rail")).size(17.0))
                .selected(*active_game == Game::StarRail)
                .min_size(egui::vec2(button_width, button_height)),
        );
        star_rail_width = star_rail.rect.width();
        if star_rail.clicked() {
            *active_game = Game::StarRail;
        }
    });

    GameSwitcherLayout {
        genshin_width,
        star_rail_width,
    }
}
