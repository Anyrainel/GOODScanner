use super::{
    state::{Lang, RefreshState, UiError, UiText},
    theme, widgets,
};
use crate::config::{Game, ToolTab};
use eframe::egui;
use std::{
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy)]
enum Source {
    Genshin,
    StarRail,
}
impl Source {
    fn for_page(game: Game, tab: ToolTab) -> Option<Self> {
        if tab == ToolTab::Credits {
            return None;
        }
        match (game, tab) {
            (Game::Genshin, _) => Some(Self::Genshin),
            (Game::StarRail, _) => Some(Self::StarRail),
        }
    }
    fn index(self) -> usize {
        match self {
            Self::Genshin => 0,
            Self::StarRail => 1,
        }
    }
    fn updated_at(self) -> Option<u64> {
        match self {
            Self::Genshin => genshin_scanner::game_data::cache_updated_at(),
            Self::StarRail => hsr_scanner::data_cache::cache_updated_at(),
        }
    }
    fn refresh(self, include_achievements: bool) -> anyhow::Result<()> {
        match self {
            Self::Genshin => {
                genshin_scanner::game_data::force_refresh()?;
                if include_achievements {
                    genshin_scanner::scanner::achievement::AchievementCatalog::force_refresh()?;
                }
                Ok(())
            },
            Self::StarRail => {
                hsr_scanner::data_cache::force_refresh()?;
                if include_achievements {
                    let references = hsr_scanner::data_cache::load_data_cache()?;
                    hsr_scanner::data_cache::load_achievement_data(&references, true)?;
                }
                Ok(())
            },
        }
    }
}

struct CachedData {
    refresh: RefreshState,
    updated_at: Option<u64>,
    checked: Option<Instant>,
    reader: Option<JoinHandle<Option<u64>>>,
}
impl Default for CachedData {
    fn default() -> Self {
        Self {
            refresh: RefreshState::Idle,
            updated_at: None,
            checked: None,
            reader: None,
        }
    }
}
impl CachedData {
    fn poll(&mut self) {
        let was_running = self.refresh.is_running();
        self.refresh.poll();
        if was_running && !self.refresh.is_running() {
            self.checked = None;
        }
        if self
            .reader
            .as_ref()
            .is_some_and(|reader| reader.is_finished())
        {
            self.updated_at = self.reader.take().unwrap().join().ok().flatten();
        }
    }
    fn read_age(&mut self, source: Source) {
        // Read metadata off the UI thread; the HSR cache contains the full data document.
        if self.reader.is_none()
            && self
                .checked
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(60))
        {
            self.checked = Some(Instant::now());
            self.reader = std::thread::Builder::new()
                .name("cache-age".into())
                .spawn(move || source.updated_at())
                .ok();
        }
    }
}

#[derive(Default)]
pub struct DataRefresh {
    caches: [CachedData; 2],
}
impl DataRefresh {
    pub fn poll(&mut self) {
        for cache in &mut self.caches {
            cache.poll();
        }
    }
    pub fn is_running(&self) -> bool {
        self.caches.iter().any(|cache| cache.refresh.is_running())
    }
    pub fn is_pending(&self) -> bool {
        self.is_running() || self.caches.iter().any(|cache| cache.reader.is_some())
    }
    pub fn heading(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        game: Game,
        tab: ToolTab,
        enabled: bool,
        include_achievements: bool,
    ) {
        let title = match tab {
            ToolTab::Capture => lang.t("抓包器", "Capture"),
            ToolTab::Scanner => lang.t("扫描器", "Scanner"),
            ToolTab::Manager => lang.t("管理器", "Manager"),
            ToolTab::Credits => lang.t("关于", "About"),
        };
        let Some(source) = Source::for_page(game, tab) else {
            ui.heading(title);
            return;
        };
        let cache = &mut self.caches[source.index()];
        cache.read_age(source);
        ui.scope(|ui| {
            // horizontal_wrapped starts with interact_size.y and centers taller
            // widgets around it. Allocate the full mixed text/button height upfront.
            ui.spacing_mut().interact_size.y = ui
                .spacing()
                .interact_size
                .y
                .max(ui.text_style_height(&egui::TextStyle::Heading))
                .max(
                    ui.text_style_height(&egui::TextStyle::Button)
                        + 2.0 * ui.spacing().button_padding.y,
                );
            ui.horizontal_wrapped(|ui| {
                ui.heading(title);
                ui.add_space(8.0);
                let busy = cache.refresh.is_running();
                if ui
                    .add_enabled(
                        !busy && enabled,
                        egui::Button::new(
                            egui::RichText::new(if busy {
                                lang.t("刷新中…", "Refreshing…")
                            } else {
                                lang.t("刷新数据", "Refresh data")
                            })
                            .size(12.0),
                        ),
                    )
                    .clicked()
                {
                    let hint = UiText::new(
                        "无法刷新游戏数据，请检查网络后重试。",
                        "Cannot refresh game data. Check your connection and retry.",
                    );
                    let thread_hint = hint.clone();
                    cache.refresh = match std::thread::Builder::new()
                        .name("game-data-refresh".into())
                        .spawn(move || {
                            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                source.refresh(include_achievements)
                            })) {
                                Ok(result) => result
                                    .map_err(|error| UiError::from_anyhow(thread_hint, &error)),
                                Err(error) => Err(UiError::from_panic(thread_hint, error.as_ref())),
                            }
                        }) {
                        Ok(handle) => RefreshState::Running(handle),
                        Err(error) => RefreshState::Failed(UiError::from_error(hint, error)),
                    };
                }
                ui.label(
                    egui::RichText::new(age_text(lang, cache.updated_at, now_secs()))
                        .size(12.0)
                        .color(theme::MUTED),
                )
                .on_hover_text(lang.t(
                    "游戏数据上次成功下载的时间；有多个缓存文件时显示最旧的一份。",
                    "Time since the last successful download; shows the oldest cached data file.",
                ));
                if let RefreshState::Failed(error) = &cache.refresh {
                    ui.menu_button(
                        egui::RichText::new(lang.t("刷新失败", "Refresh failed"))
                            .color(theme::ERROR)
                            .size(12.0),
                        |ui| {
                            ui.set_max_width(340.0);
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                            widgets::error_card(ui, lang, error);
                        },
                    );
                }
            });
        });
    }
    #[cfg(feature = "dev-tools")]
    pub(super) fn preview_age(&mut self, seconds: u64) {
        for cache in &mut self.caches {
            cache.updated_at = Some(now_secs().saturating_sub(seconds));
            cache.checked = Some(Instant::now());
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn age_text(lang: Lang, fetched: Option<u64>, now: u64) -> String {
    let Some(fetched) = fetched else {
        return lang.t("更新时间未知", "Age unknown").into();
    };
    let Some(age) = now.checked_sub(fetched) else {
        return lang.t("时间异常", "Clock mismatch").into();
    };
    if age < 60 {
        return lang.t("刚刚", "Just now").into();
    }
    let (number, zh, en) = if age < 3600 {
        (age / 60, "分钟前", "m ago")
    } else if age < 86400 {
        (age / 3600, "小时前", "h ago")
    } else {
        (age / 86400, "天前", "d ago")
    };
    format!("{number}{}", lang.t(zh, en))
}

#[cfg(test)]
mod feedback_tests {
    use super::*;

    #[test]
    fn failed_refresh_preserves_age_and_schedules_a_metadata_read() {
        let handle = std::thread::spawn(|| {
            Err(UiError::from_error(
                UiText::new("刷新失败", "Refresh failed"),
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused"),
            ))
        });
        while !handle.is_finished() {
            std::thread::yield_now();
        }
        let mut cache = CachedData {
            refresh: RefreshState::Running(handle),
            updated_at: Some(123),
            checked: Some(Instant::now()),
            reader: None,
        };
        cache.poll();
        assert!(matches!(cache.refresh, RefreshState::Failed(_)));
        assert_eq!(cache.updated_at, Some(123));
        assert!(cache.checked.is_none());
    }

    #[test]
    fn pages_share_the_refresh_state_for_their_actual_cache() {
        for game in [Game::Genshin, Game::StarRail] {
            assert_eq!(
                Source::for_page(game, ToolTab::Scanner).unwrap().index(),
                Source::for_page(game, ToolTab::Manager).unwrap().index(),
            );
            assert!(Source::for_page(game, ToolTab::Credits).is_none());
        }
        #[cfg(feature = "capture")]
        {
            assert_eq!(
                Source::for_page(Game::Genshin, ToolTab::Capture)
                    .unwrap()
                    .index(),
                Source::for_page(Game::Genshin, ToolTab::Scanner)
                    .unwrap()
                    .index(),
            );
            assert_eq!(
                Source::for_page(Game::StarRail, ToolTab::Capture)
                    .unwrap()
                    .index(),
                Source::for_page(Game::StarRail, ToolTab::Scanner)
                    .unwrap()
                    .index(),
            );
        }
    }
}
