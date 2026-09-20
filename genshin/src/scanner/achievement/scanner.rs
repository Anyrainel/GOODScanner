use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{bail, Result};
use image::{GenericImageView, RgbImage};
use rayon::prelude::*;
use yas::ocr::ImageToText;
use yas::{log_debug, log_info, log_warn};

use super::catalog::{normalize_text, AchievementCatalog};
use super::config::GoodAchievementScannerConfig;
use super::layout::{
    CATEGORY_FIRST_Y, CATEGORY_STEP_Y, CATEGORY_VISIBLE, CATEGORY_X, IDLE_FRAMES_BEFORE_STOP,
    LIST_WHEEL_TICKS, MAX_CATEGORIES, OVERVIEW_FIRST_CATEGORY_POS, PAIMON_ACHIEVEMENT_POS,
    PAIMON_MENU_DELAY,
};
use super::recognize::recognize_row;
use super::split::{
    crop_row, detect_list_rect, detect_selected_category_rect, split_row_bands, PixelRect,
};
use crate::scanner::common::coord_scaler::CoordScaler;
use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::ocr_pool::SharedOcrPools;
use crate::scanner::common::progress::ProgressFn;

pub struct GoodAchievementScanner {
    config: GoodAchievementScannerConfig,
    catalog: Arc<AchievementCatalog>,
}

impl GoodAchievementScanner {
    pub fn new(
        config: GoodAchievementScannerConfig,
        catalog: Arc<AchievementCatalog>,
    ) -> Result<Self> {
        Ok(Self { config, catalog })
    }

    /// Open the achievement list from any game screen, then walk left-side
    /// categories. Returns sorted unique completed achievement IDs.
    ///
    /// List motion follows cocogoat CaptureScanner: capture → split rows →
    /// OCR → click + 11 wheel ticks → stop after several frames with no new
    /// rows. Category motion clicks the next left-list row; after the 8th
    /// visible row the game auto-scrolls, so that same slot is clicked again
    /// until the right-hand titles are all ones already seen.
    pub fn scan(
        &self,
        ctrl: &mut GenshinGameController,
        pools: &SharedOcrPools,
        progress_fn: Option<&ProgressFn<'_>>,
    ) -> Result<Vec<u32>> {
        log_info!("[achievement] 开始扫描成就", "[achievement] starting scan");

        let cancel = ctrl.cancel_token();
        ctrl.focus_game_window();
        if cancel.check_rmb() {
            bail!("cancelled");
        }
        self.open_achievement_screen(ctrl)?;

        let mut completed: BTreeSet<u32> = BTreeSet::new();
        let mut seen_rows: BTreeSet<String> = BTreeSet::new();
        let mut seen_categories: BTreeSet<String> = BTreeSet::new();
        let started = SystemTime::now();
        let mut category_slot = 0usize;

        let report = |completed_n: usize, current: &str| {
            if let Some(f) = progress_fn {
                f(completed_n, completed_n.max(1), current, "achievements");
            }
        };

        for category_index in 0..MAX_CATEGORIES {
            if cancel.check_rmb() {
                bail!("cancelled");
            }
            if self.config.max_count > 0 && completed.len() >= self.config.max_count {
                break;
            }

            let selected_name = self.read_selected_category(ctrl, pools);
            if let Some(name) = selected_name.as_deref() {
                let key = normalize_text(name);
                if category_index > 0 && !key.is_empty() && seen_categories.contains(&key) {
                    log_info!(
                        "[achievement] 分类「{}」已扫描过，已到列表末尾",
                        "[achievement] category '{}' already scanned; end of list",
                        name
                    );
                    break;
                }
                if !key.is_empty() {
                    seen_categories.insert(key);
                }
                log_info!(
                    "[achievement] 扫描分类: {}",
                    "[achievement] scanning category: {}",
                    name
                );
            } else {
                log_info!(
                    "[achievement] 扫描分类 #{}",
                    "[achievement] scanning category #{}",
                    category_index + 1
                );
            }

            let rows_before = seen_rows.len();
            self.scan_open_list(ctrl, pools, &mut completed, &mut seen_rows, &report)?;
            if self.config.max_count > 0 && completed.len() >= self.config.max_count {
                break;
            }
            // After the last category, clicking slot 8 wraps or stays put.
            // Right-hand titles then repeat what we already recognized.
            if category_index > 0 && seen_rows.len() == rows_before {
                log_info!(
                    "[achievement] 右侧条目均已识别，已到最后一个分类",
                    "[achievement] right-hand titles already recognized; last category reached"
                );
                break;
            }

            if category_slot + 1 < CATEGORY_VISIBLE {
                category_slot += 1;
            }
            self.click_category_slot(ctrl, category_slot);
        }

        let ids: Vec<u32> = completed.into_iter().collect();
        let elapsed = started.elapsed().unwrap_or_default();
        log_info!(
            "[achievement] 完成: {} 个已达成成就，用时 {:.1}s",
            "[achievement] done: {} completed achievements in {:.1}s",
            ids.len(),
            elapsed.as_secs_f64()
        );
        Ok(ids)
    }

    /// Return to the overworld, open the Paimon menu, enter 成就, then open
    /// the first category list the rest of the scan walks.
    fn open_achievement_screen(&self, ctrl: &mut GenshinGameController) -> Result<()> {
        let cancel = ctrl.cancel_token();
        ctrl.focus_game_window();
        if self.list_is_open(ctrl)? {
            log_info!(
                "[achievement] 成就列表已打开",
                "[achievement] achievement list already open"
            );
            return Ok(());
        }

        ctrl.return_to_main_ui(8);
        if cancel.check_rmb() {
            bail!("cancelled");
        }

        for attempt in 0..3 {
            if cancel.check_rmb() {
                bail!("cancelled");
            }
            ctrl.focus_game_window();
            if self.list_is_open(ctrl)? {
                log_info!(
                    "[achievement] 成就列表已打开，第{}次尝试",
                    "[achievement] achievement list opened on attempt {}",
                    attempt + 1
                );
                return Ok(());
            }

            for _ in 0..3 {
                if !ctrl.is_likely_main_world() {
                    break;
                }
                ctrl.key_press(enigo::Key::Escape);
                yas::utils::sleep(PAIMON_MENU_DELAY);
            }

            ctrl.click_at(PAIMON_ACHIEVEMENT_POS.0, PAIMON_ACHIEVEMENT_POS.1);
            yas::utils::sleep(self.config.open_delay as u32);
            if self.list_is_open(ctrl)? {
                log_info!(
                    "[achievement] 成就列表已打开，第{}次尝试",
                    "[achievement] achievement list opened on attempt {}",
                    attempt + 1
                );
                return Ok(());
            }

            // Overview grid: click 天地万象 to enter the left-sidebar list.
            ctrl.click_at(OVERVIEW_FIRST_CATEGORY_POS.0, OVERVIEW_FIRST_CATEGORY_POS.1);
            yas::utils::sleep(self.config.open_delay as u32);
            if self.list_is_open(ctrl)? {
                log_info!(
                    "[achievement] 成就列表已打开，第{}次尝试",
                    "[achievement] achievement list opened on attempt {}",
                    attempt + 1
                );
                return Ok(());
            }

            log_debug!(
                "[achievement] 未检测到成就列表（第{}次尝试），重试中...",
                "[achievement] achievement list not detected (attempt {}), retrying...",
                attempt + 1
            );
            // Achievement UI samples as "main world" at the Paimon-icon points,
            // so do not use return_to_main_ui here — just Esc out.
            for _ in 0..4 {
                ctrl.key_press(enigo::Key::Escape);
                yas::utils::sleep(400);
            }
        }

        bail!(
            "无法打开成就界面。请确认游戏在前台且未遮挡暂停菜单。 / Failed to open the achievement screen. Make sure the game is focused and the pause menu is not covered."
        )
    }

    fn list_is_open(&self, ctrl: &GenshinGameController) -> Result<bool> {
        let frame = ctrl.capture_game()?;
        Ok(detect_list_rect(&frame).is_some())
    }

    fn scan_open_list(
        &self,
        ctrl: &mut GenshinGameController,
        pools: &SharedOcrPools,
        completed: &mut BTreeSet<u32>,
        seen_rows: &mut BTreeSet<String>,
        report: &dyn Fn(usize, &str),
    ) -> Result<()> {
        let cancel = ctrl.cancel_token();
        let mut idle_frames = 0u32;
        let mut dump_index = 0u32;
        let mut click_pos: Option<(f64, f64)> = None;

        loop {
            if cancel.check_rmb() {
                bail!("cancelled");
            }
            if self.config.max_count > 0 && completed.len() >= self.config.max_count {
                return Ok(());
            }

            let frame = ctrl.capture_game()?;
            let Some(list_rect) = detect_list_rect(&frame) else {
                bail!("未找到成就列表 / Achievement list not found");
            };
            let Some(list_img) = crop_pixel(&frame, list_rect) else {
                bail!("未找到成就列表 / Achievement list not found");
            };

            if click_pos.is_none() {
                click_pos = Some(self.click_pos_for_list(ctrl, list_rect, &list_img));
            }

            let keep_last = idle_frames + 1 >= IDLE_FRAMES_BEFORE_STOP;
            let bands = split_row_bands(&list_img, keep_last);
            let new_before = seen_rows.len();

            if self.config.dump_images {
                let _ = std::fs::create_dir_all("debug_images/achievement");
                let path = format!("debug_images/achievement/list_{dump_index:04}.png");
                let _ = list_img.save(&path);
                dump_index += 1;
            }

            let rows: Vec<(usize, RgbImage)> = bands
                .iter()
                .enumerate()
                .filter_map(|(i, band)| crop_row(&list_img, band).map(|row| (i, row)))
                .collect();
            let catalog = Arc::clone(&self.catalog);
            let recognized: Vec<(usize, RgbImage, super::recognize::RecognizedRow)> = rows
                .into_par_iter()
                .filter_map(|(i, row)| {
                    let ocr = pools.v4().get();
                    match recognize_row(&ocr, &row, &catalog) {
                        Ok(recognized) => Some((i, row, recognized)),
                        Err(_) => None,
                    }
                })
                .collect();

            for (i, row, recognized) in recognized {
                let key = row_key(
                    &recognized.title,
                    &recognized.subtitle,
                    &recognized.status,
                    &recognized.date,
                );
                if key.is_empty() {
                    continue;
                }
                if !seen_rows.insert(key) {
                    continue;
                }

                if self.config.dump_images {
                    let path = format!("debug_images/achievement/row_{dump_index:04}_{i}.png");
                    let _ = row.save(&path);
                }

                if let Some(id) = recognized.id {
                    if recognized.done {
                        completed.insert(id);
                    }
                    if self.config.verbose || self.config.log_progress {
                        log_info!(
                            "[achievement] {} id={} done={} title='{}'",
                            "[achievement] {} id={} done={} title='{}'",
                            if recognized.done {
                                "完成"
                            } else {
                                "未完成"
                            },
                            id,
                            recognized.done,
                            recognized.title
                        );
                    }
                    report(completed.len(), &recognized.title);
                } else if self.config.verbose {
                    log_warn!(
                        "[achievement] 未匹配: title='{}' subtitle='{}' status='{}'",
                        "[achievement] unmatched: title='{}' subtitle='{}' status='{}'",
                        recognized.title,
                        recognized.subtitle,
                        recognized.status
                    );
                }
            }

            if seen_rows.len() == new_before {
                idle_frames += 1;
            } else {
                idle_frames = 0;
            }
            if idle_frames >= IDLE_FRAMES_BEFORE_STOP {
                break;
            }

            if let Some((x, y)) = click_pos {
                // Cocogoat: click the list, then MOUSEEVENTF_WHEEL -120 × 11.
                ctrl.click_at(x, y);
                for _ in 0..LIST_WHEEL_TICKS {
                    ctrl.mouse_scroll(1);
                }
                yas::utils::sleep(self.config.scroll_delay as u32);
            }
        }

        Ok(())
    }

    fn click_pos_for_list(
        &self,
        ctrl: &GenshinGameController,
        list_rect: PixelRect,
        list_img: &RgbImage,
    ) -> (f64, f64) {
        let bands = split_row_bands(list_img, true);
        let (px, py) = if let Some(band) = bands.first() {
            (
                list_rect.x + band.rect.x + band.rect.w / 3,
                list_rect.y + band.rect.y + band.rect.h * 2 / 3,
            )
        } else {
            (list_rect.x + list_rect.w / 3, list_rect.y + list_rect.h / 8)
        };
        pixel_to_base(px, py, &ctrl.scaler)
    }

    fn click_category_slot(&self, ctrl: &mut GenshinGameController, slot: usize) {
        let slot = slot.min(CATEGORY_VISIBLE.saturating_sub(1));
        let y = CATEGORY_FIRST_Y + CATEGORY_STEP_Y * slot as f64;
        if slot + 1 == CATEGORY_VISIBLE {
            log_info!(
                "[achievement] 点击第{}个分类槽（游戏会自动上移列表）",
                "[achievement] clicking category slot {} (list auto-scrolls)",
                slot + 1
            );
        } else {
            log_info!(
                "[achievement] 切换到下一分类",
                "[achievement] switching to next category"
            );
        }
        ctrl.click_at(CATEGORY_X, y);
        yas::utils::sleep(self.config.category_delay as u32);
    }

    fn read_selected_category(
        &self,
        ctrl: &GenshinGameController,
        pools: &SharedOcrPools,
    ) -> Option<String> {
        let frame = ctrl.capture_game().ok()?;
        let rect = detect_selected_category_rect(&frame)?;
        let crop = crop_pixel(&frame, rect)?;
        let ocr = pools.v4().get();
        let text = ocr.image_to_text(&crop, false).ok()?;
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }
}

fn pixel_to_base(px: u32, py: u32, scaler: &CoordScaler) -> (f64, f64) {
    let fx = scaler.factor_x().max(1e-6);
    let fy = scaler.factor_y().max(1e-6);
    (px as f64 / fx, py as f64 / fy)
}

fn crop_pixel(image: &RgbImage, rect: PixelRect) -> Option<RgbImage> {
    if rect.w == 0 || rect.h == 0 {
        return None;
    }
    let x = rect.x.min(image.width().saturating_sub(1));
    let y = rect.y.min(image.height().saturating_sub(1));
    let w = rect.w.min(image.width().saturating_sub(x));
    let h = rect.h.min(image.height().saturating_sub(y));
    if w == 0 || h == 0 {
        return None;
    }
    Some(image.view(x, y, w, h).to_image())
}

fn row_key(title: &str, subtitle: &str, status: &str, date: &str) -> String {
    format!("{title}|{subtitle}|{status}|{date}")
}
