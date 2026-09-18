use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{bail, Result};
use image::{GenericImageView, RgbImage};
use yas::{log_debug, log_info, log_warn};

use super::catalog::AchievementCatalog;
use super::config::GoodAchievementScannerConfig;
use super::layout::{
    CATEGORY_FIRST_Y, CATEGORY_STEP_Y, CATEGORY_VISIBLE, CATEGORY_X, IDLE_FRAMES_BEFORE_STOP,
    LIST_TITLE_RECT, MAX_CATEGORIES,
};
use super::recognize::recognize_row;
use super::split::{crop_row, detect_list_rect, split_row_bands, PixelRect};
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

    /// Scan the currently open achievement page, then walk remaining left-side
    /// categories. Returns sorted unique completed achievement IDs.
    ///
    /// Control loop is the native port of cocogoat CaptureScanner:
    /// capture → split rows (drop last partial) → OCR → click + wheel →
    /// stop after several frames with no new rows.
    pub fn scan(
        &self,
        ctrl: &mut GenshinGameController,
        pools: &SharedOcrPools,
        progress_fn: Option<&ProgressFn<'_>>,
    ) -> Result<Vec<u32>> {
        log_info!(
            "[achievement] 开始扫描成就页。请先打开成就界面任意分类。",
            "[achievement] starting. Open any achievement category first."
        );

        let cancel = ctrl.cancel_token();
        ctrl.focus_game_window();
        if cancel.check_rmb() {
            bail!("cancelled");
        }

        let mut completed: BTreeSet<u32> = BTreeSet::new();
        let mut seen_rows: BTreeSet<String> = BTreeSet::new();
        let mut seen_categories: BTreeSet<String> = BTreeSet::new();
        let started = SystemTime::now();

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

            if category_index > 0 {
                self.click_category(ctrl, category_index);
                yas::utils::sleep(self.config.category_delay as u32);
            }

            let title = self.read_category_title(ctrl, pools);
            if let Some(ref name) = title {
                if !seen_categories.insert(name.clone()) {
                    log_debug!(
                        "[achievement] 分类 '{}' 已扫描，结束",
                        "[achievement] category '{}' already scanned, stopping",
                        name
                    );
                    break;
                }
                log_info!(
                    "[achievement] 扫描分类: {}",
                    "[achievement] scanning category: {}",
                    name
                );
            }

            let before = completed.len();
            self.scan_open_list(ctrl, pools, &mut completed, &mut seen_rows, &report)?;
            if category_index > 0 && completed.len() == before && title.is_none() {
                log_debug!(
                    "[achievement] 点击下一分类后没有新成就，结束",
                    "[achievement] no new achievements after category click, stopping"
                );
                break;
            }
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
            let list_rect = detect_list_rect(&frame);
            let Some(list_img) = crop_pixel(&frame, list_rect) else {
                bail!(
                    "未找到成就列表。请先打开成就界面任意分类。 / Achievement list not found. Open any achievement category first."
                );
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

            let ocr = pools.v4().get();
            for (i, band) in bands.iter().enumerate() {
                let Some(row) = crop_row(&list_img, band) else {
                    continue;
                };
                let recognized = recognize_row(&ocr, &row, &self.catalog)?;
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
            drop(ocr);

            if seen_rows.len() == new_before {
                idle_frames += 1;
            } else {
                idle_frames = 0;
            }
            if idle_frames >= IDLE_FRAMES_BEFORE_STOP {
                break;
            }

            if let Some((x, y)) = click_pos {
                ctrl.click_at(x, y);
                ctrl.mouse_scroll(1);
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

    fn click_category(&self, ctrl: &mut GenshinGameController, index: usize) {
        let visible_index = index % CATEGORY_VISIBLE;
        if index > 0 && visible_index == 0 {
            ctrl.move_to(CATEGORY_X, CATEGORY_FIRST_Y);
            yas::utils::sleep(20);
            for _ in 0..CATEGORY_VISIBLE {
                ctrl.mouse_scroll(1);
                yas::utils::sleep(30);
            }
            yas::utils::sleep(self.config.category_delay as u32);
        }
        let y = CATEGORY_FIRST_Y + CATEGORY_STEP_Y * visible_index as f64;
        ctrl.click_at(CATEGORY_X, y);
    }

    fn read_category_title(
        &self,
        ctrl: &GenshinGameController,
        pools: &SharedOcrPools,
    ) -> Option<String> {
        let ocr = pools.v4().get();
        let (x, y, w, h) = LIST_TITLE_RECT;
        match ctrl.ocr_region(&ocr, (x, y, w, h)) {
            Ok(text) => {
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            },
            Err(_) => None,
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
