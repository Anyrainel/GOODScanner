use std::collections::BTreeSet;
use std::io::Write;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::{bail, Result};
use image::{GenericImageView, RgbImage};
use rayon::prelude::*;
use yas::ocr::ImageToText;
use yas::{log_debug, log_info, log_warn};

use super::catalog::{normalize_text, AchievementCatalog};
use super::config::GoodAchievementScannerConfig;
use super::layout::{
    ACHIEVEMENT_HEADER_POS, CATEGORY_FIRST_Y, CATEGORY_STEP_Y, CATEGORY_VISIBLE, CATEGORY_X,
    IDLE_FRAMES_BEFORE_STOP, LIST_HOVER_SETTLE_MS, LIST_SCROLLBAR_INSET, LIST_SCROLLBAR_POS,
    LIST_TICK_DELAY_MS, LIST_WHEEL_TICKS, MAX_CATEGORIES, MAX_LIST_FRAMES,
    OVERVIEW_FIRST_CATEGORY_POS, PAIMON_ACHIEVEMENT_POS, PAIMON_MENU_DELAY,
};
use super::recognize::{progress_is_open, recognize_row};
use super::split::{
    crop_card, crop_row, crop_title, detect_list_rect, detect_scrollbar_thumb,
    detect_selected_category_rect, list_nearly_equal, split_row_bands, PixelRect,
};
use crate::scanner::common::coord_scaler::CoordScaler;
use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::ocr_factory;
use crate::scanner::common::ocr_pool::{OcrPool, SharedOcrPools};
use crate::scanner::common::progress::ProgressFn;

const DUMP_DIR: &str = "debug_images/achievement";

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
    /// List motion: capture → split rows → OCR → park the cursor on the
    /// scrollbar (cards enlarge if hovered) → 8 delayed wheel detents.
    /// Stop after several frames with no new rows. Category motion clicks the
    /// next left-list row; after the 8th visible row the game auto-scrolls.
    pub fn scan(
        &self,
        ctrl: &mut GenshinGameController,
        _pools: &SharedOcrPools,
        progress_fn: Option<&ProgressFn<'_>>,
    ) -> Result<Vec<u32>> {
        log_info!("[achievement] 开始扫描成就", "[achievement] starting scan");
        let rec_backend = self.config.ocr_backend.clone();
        log_info!(
            "[achievement] OCR后端: {}",
            "[achievement] OCR backend: {}",
            rec_backend
        );
        let rec_pool = OcrPool::new(move || ocr_factory::create_ocr_model(&rec_backend), 2)?;

        let cancel = ctrl.cancel_token();
        ctrl.focus_game_window();
        if cancel.check_rmb() {
            bail!("cancelled");
        }
        self.open_achievement_screen(ctrl)?;
        if self.config.scroll_calibrate {
            self.calibrate_list_scroll(ctrl, &rec_pool)?;
            return Ok(Vec::new());
        }

        let mut completed: BTreeSet<u32> = BTreeSet::new();
        let mut seen_rows: BTreeSet<String> = BTreeSet::new();
        let mut seen_categories: BTreeSet<String> = BTreeSet::new();
        let started = SystemTime::now();
        let mut category_slot = 0usize;
        let mut same_name_retries = 0u32;
        let mut empty_new_streak = 0u32;
        progress_log("scan_start");

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

            let (selected_name, category_complete) = {
                progress_log(&format!("read_selected cat_index={category_index}"));
                let (name, complete) = self.read_selected_category(ctrl, &rec_pool);
                progress_log(&format!(
                    "read_selected_done cat_index={category_index} name={} complete={complete}",
                    name.as_deref().unwrap_or("-")
                ));
                (name, complete)
            };
            if let Some(name) = selected_name.as_deref() {
                let key = normalize_text(name);
                if !is_plausible_category_name(&key) {
                    progress_log(&format!("ignore_category_name {name}"));
                    log_info!(
                        "[achievement] 忽略无效分类名: {}",
                        "[achievement] ignoring implausible category name: {}",
                        name
                    );
                    if self.config.dump_images {
                        if let Some(crop) = self.capture_selected_category(ctrl) {
                            save_dump(
                                &format!("c{category_index:02}_badname_{}.png", dump_slug(name)),
                                &crop,
                            );
                        }
                    }
                } else if category_index > 0 && seen_categories.contains(&key) {
                    if same_name_retries == 0 {
                        same_name_retries = 1;
                        progress_log(&format!("repeat_name_retry {name}"));
                        log_info!(
                            "[achievement] 分类未切换，再点一次列表底部",
                            "[achievement] category did not advance; clicking lower on the list"
                        );
                        if self.config.dump_images {
                            if let Ok(frame) = ctrl.capture_game() {
                                save_dump(
                                    &format!("c{category_index:02}_repeat_{}.png", dump_slug(name)),
                                    &frame,
                                );
                            }
                        }
                        ctrl.click_at(CATEGORY_X, 1030.0);
                        yas::utils::sleep(self.config.category_delay as u32);
                        continue;
                    }
                    log_info!(
                        "[achievement] 分类「{}」已扫描过，已到列表末尾",
                        "[achievement] category '{}' already scanned; end of list",
                        name
                    );
                    progress_log(&format!("wrap_stop name={name}"));
                    break;
                } else if !key.is_empty() {
                    same_name_retries = 0;
                    seen_categories.insert(key);
                    log_info!(
                        "[achievement] 扫描分类: {}",
                        "[achievement] scanning category: {}",
                        name
                    );
                }
            } else {
                log_info!(
                    "[achievement] 扫描分类 #{}",
                    "[achievement] scanning category #{}",
                    category_index + 1
                );
            }

            let category_label = selected_name
                .as_deref()
                .map(dump_slug)
                .unwrap_or_else(|| format!("{}", category_index + 1));
            if self.config.dump_images {
                if let Some(crop) = self.capture_selected_category(ctrl) {
                    save_dump(&format!("c{category_index:02}_{category_label}_sel.png"), &crop);
                }
            }

            let rows_before = seen_rows.len();
            let done_before = completed.len();
            let cat_started = Instant::now();
            self.scan_open_list(
                ctrl,
                &rec_pool,
                &mut completed,
                &mut seen_rows,
                &report,
                category_index,
                &category_label,
                selected_name.as_deref(),
                category_complete,
            )?;
            log_info!(
                "[achievement] 分类结束: {} 新条目={} 完成+={} 用时 {:.1}s",
                "[achievement] category done: {} new_rows={} completed+={} in {:.1}s",
                category_label,
                seen_rows.len().saturating_sub(rows_before),
                completed.len().saturating_sub(done_before),
                cat_started.elapsed().as_secs_f64()
            );
            progress_log(&format!(
                "category_end name={category_label} new_rows={} completed_delta={} secs={:.1}",
                seen_rows.len().saturating_sub(rows_before),
                completed.len().saturating_sub(done_before),
                cat_started.elapsed().as_secs_f64()
            ));
            if self.config.max_count > 0 && completed.len() >= self.config.max_count {
                break;
            }
            // A short category can OCR zero new titles (English names, banner
            // click missing the list). Only treat several empties in a row as wrap.
            if category_index > 0 && seen_rows.len() == rows_before {
                empty_new_streak += 1;
                progress_log(&format!(
                    "empty_category name={category_label} streak={empty_new_streak}"
                ));
                if empty_new_streak >= 3 {
                    log_info!(
                        "[achievement] 连续分类无新标题，已到列表末尾",
                        "[achievement] several categories with no new titles; end of list"
                    );
                    progress_log(&format!(
                        "empty_streak_stop name={category_label} streak={empty_new_streak}"
                    ));
                    break;
                }
            } else {
                empty_new_streak = 0;
            }

            if category_slot + 1 < CATEGORY_VISIBLE {
                category_slot += 1;
            }
            self.click_next_category(ctrl, category_slot, category_index);
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
        let opened = Instant::now();
        ctrl.focus_game_window();
        // Always re-enter from the overview so the left list starts at 天地万象,
        // even if a previous run left a later category open.
        if self.list_is_open(ctrl)? {
            log_info!(
                "[achievement] 列表已打开，先退出再从天地万象重新进入",
                "[achievement] list already open; leaving and re-entering from the first category"
            );
            for _ in 0..6 {
                ctrl.key_press(enigo::Key::Escape);
                yas::utils::sleep(400);
            }
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
            if self.dump_and_list_is_open(ctrl, attempt, "pre")? {
                log_info!(
                    "[achievement] 成就列表已打开，第{}次尝试 ({:.1}s)",
                    "[achievement] achievement list opened on attempt {} ({:.1}s)",
                    attempt + 1,
                    opened.elapsed().as_secs_f64()
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
            if self.dump_and_list_is_open(ctrl, attempt, "paimon")? {
                log_info!(
                    "[achievement] 成就列表已打开，第{}次尝试 ({:.1}s)",
                    "[achievement] achievement list opened on attempt {} ({:.1}s)",
                    attempt + 1,
                    opened.elapsed().as_secs_f64()
                );
                return Ok(());
            }

            // Overview grid: click 天地万象 to enter the left-sidebar list.
            ctrl.click_at(OVERVIEW_FIRST_CATEGORY_POS.0, OVERVIEW_FIRST_CATEGORY_POS.1);
            yas::utils::sleep(self.config.open_delay as u32);
            if self.dump_and_list_is_open(ctrl, attempt, "overview")? {
                log_info!(
                    "[achievement] 成就列表已打开，第{}次尝试 ({:.1}s)",
                    "[achievement] achievement list opened on attempt {} ({:.1}s)",
                    attempt + 1,
                    opened.elapsed().as_secs_f64()
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

    fn dump_and_list_is_open(
        &self,
        ctrl: &GenshinGameController,
        attempt: usize,
        step: &str,
    ) -> Result<bool> {
        let frame = ctrl.capture_game()?;
        if self.config.dump_images {
            save_dump(&format!("open_a{attempt}_{step}.png"), &frame);
        }
        Ok(detect_list_rect(&frame).is_some())
    }

    fn list_is_open(&self, ctrl: &GenshinGameController) -> Result<bool> {
        let frame = ctrl.capture_game()?;
        Ok(detect_list_rect(&frame).is_some())
    }

    fn scan_open_list(
        &self,
        ctrl: &mut GenshinGameController,
        rec: &OcrPool,
        completed: &mut BTreeSet<u32>,
        seen_rows: &mut BTreeSet<String>,
        report: &dyn Fn(usize, &str),
        category_index: usize,
        category_label: &str,
        category_name: Option<&str>,
        mut force_done: bool,
    ) -> Result<()> {
        let cancel = ctrl.cancel_token();
        let mut idle_frames = 0u32;
        let mut frame_idx = 0u32;
        let mut prev_list: Option<RgbImage> = None;
        let mut ocr_ms_total = 0u128;
        let mut cat_titles: Vec<String> = Vec::new();
        let mut matched_titles: BTreeSet<String> = BTreeSet::new();
        let expected = category_name
            .and_then(|name| self.catalog.coverage(name, &[]))
            .map(|cov| cov.expected)
            .unwrap_or(0);
        let prefix = format!("c{category_index:02}_{category_label}");
        let groove = self.scrollbar_groove(ctrl);
        ctrl.click_at(ACHIEVEMENT_HEADER_POS.0, ACHIEVEMENT_HEADER_POS.1);
        yas::utils::sleep(50);
        self.park_cursor_on_scrollbar(ctrl, groove);

        loop {
            if cancel.check_rmb() {
                bail!("cancelled");
            }
            if self.config.max_count > 0 && completed.len() >= self.config.max_count {
                return Ok(());
            }
            if frame_idx >= MAX_LIST_FRAMES {
                log_warn!(
                    "[achievement] 分类 {} 达到最大翻页次数 {}，继续下一分类",
                    "[achievement] category {} hit max list frames {}, moving on",
                    category_label,
                    MAX_LIST_FRAMES
                );
                break;
            }

            let t_capture = Instant::now();
            let frame = ctrl.capture_game()?;
            let capture_ms = t_capture.elapsed().as_millis();
            let Some(list_rect) = detect_list_rect(&frame) else {
                progress_log(&format!(
                    "list_missing cat={category_label} frame={frame_idx}"
                ));
                if self.config.dump_images {
                    save_dump(&format!("{prefix}_missing_{frame_idx:03}.png"), &frame);
                }
                if frame_idx == 0 {
                    bail!("未找到成就列表 / Achievement list not found");
                }
                log_warn!(
                    "[achievement] 本帧未找到列表，重试捕捉",
                    "[achievement] list not detected this frame, recapturing"
                );
                yas::utils::sleep(80);
                continue;
            };
            let Some(list_img) = crop_pixel(&frame, list_rect) else {
                bail!("未找到成就列表 / Achievement list not found");
            };

            let unchanged = prev_list
                .as_ref()
                .map(|prev| list_nearly_equal(prev, &list_img))
                .unwrap_or(false);
            if self.config.dump_images && frame_idx == 0 {
                save_dump(&format!("{prefix}_full.png"), &frame);
            }
            // Always keep a full last card on the panel edge. Leading scraps
            // from the previous scroll are dropped inside split_row_bands.
            let bands = split_row_bands(&list_img, true);
            let new_before = seen_rows.len();
            let mut unmatched = 0usize;

            let t_ocr = Instant::now();
            let mut card_rows: Vec<(usize, RgbImage)> = Vec::new();
            for (i, band) in bands.iter().enumerate() {
                let Some(row) = crop_row(&list_img, band) else {
                    continue;
                };
                if is_progress_banner(&row) {
                    if !force_done {
                        let ocr = rec.get();
                        if let Ok(text) = ocr.image_to_text(&row, false) {
                            if category_looks_complete(&text) {
                                force_done = true;
                                progress_log(&format!(
                                    "banner_complete cat={category_label} frame={frame_idx} text={text}"
                                ));
                            }
                        }
                    }
                    continue;
                }
                card_rows.push((i, row));
            }
            let rows = card_rows;
            progress_log(&format!(
                "ocr_start cat={category_label} frame={frame_idx} rows={}",
                rows.len()
            ));
            let catalog = Arc::clone(&self.catalog);
            let category_owned = category_name.map(|s| s.to_string());
            let recognized: Vec<(usize, RgbImage, super::recognize::RecognizedRow)> = rows
                .into_par_iter()
                .filter_map(|(i, row)| {
                    let ocr = rec.get();
                    match recognize_row(&ocr, &row, &catalog, category_owned.as_deref()) {
                        Ok(recognized) => Some((i, row, recognized)),
                        Err(_) => None,
                    }
                })
                .collect();
            let ocr_ms = t_ocr.elapsed().as_millis();
            ocr_ms_total += ocr_ms;

            for (_i, row, recognized) in recognized {
                if !is_plausible_row_title(&recognized.title) {
                    continue;
                }
                cat_titles.push(recognized.title.clone());
                let key = row_key(&recognized.title, &recognized.subtitle);
                if key.is_empty() {
                    continue;
                }
                if !seen_rows.insert(key.clone()) {
                    continue;
                }

                let done = !progress_is_open(&recognized.status)
                    && (recognized.done || force_done);
                let credited = self
                    .catalog
                    .completed_ids_for_shown(&recognized.title, done);
                if !credited.is_empty() {
                    matched_titles.insert(normalize_text(&recognized.title));
                    for id in &credited {
                        completed.insert(*id);
                    }
                    if self.config.verbose || self.config.log_progress {
                        log_info!(
                            "[achievement] {} ids={} done={} title='{}'",
                            "[achievement] {} ids={} done={} title='{}'",
                            if done { "完成" } else { "未完成" },
                            credited.len(),
                            done,
                            recognized.title
                        );
                    }
                    report(completed.len(), &recognized.title);
                } else {
                    unmatched += 1;
                    progress_log(&format!(
                        "unmatched cat={category_label} reason=no-match title={} subtitle={}",
                        recognized.title, recognized.subtitle
                    ));
                }
                if self.config.dump_images {
                    let slug = dump_slug(&key);
                    if let Some(card) = crop_card(&row) {
                        save_dump(&format!("{prefix}_card_{slug}.png"), &card);
                    }
                    if let Some(title) = crop_title(&row) {
                        save_dump(&format!("{prefix}_title_{slug}.png"), &title);
                    }
                }
            }

            let new_rows = seen_rows.len().saturating_sub(new_before);
            if new_rows == 0 {
                idle_frames += 1;
            } else {
                idle_frames = 0;
            }

            log_info!(
                "[achievement] 帧 {}/{} capture={}ms ocr={}ms rows={} new={} unmatched={} idle={} done={}",
                "[achievement] frame {}/{} capture={}ms ocr={}ms rows={} new={} unmatched={} idle={} done={}",
                frame_idx,
                category_label,
                capture_ms,
                ocr_ms,
                bands.len(),
                new_rows,
                unmatched,
                idle_frames,
                completed.len()
            );
            progress_log(&format!(
                "frame cat={category_label} i={frame_idx} capture_ms={capture_ms} ocr_ms={ocr_ms} rows={} new={new_rows} unmatched={unmatched} idle={idle_frames} done={} unchanged={unchanged}",
                bands.len(),
                completed.len()
            ));

            // One unchanged capture is the bottom of a short category. A long
            // category whose first wheel did nothing (cursor still on the
            // sidebar after the category click) must be scrolled again.
            // `matched_titles` is unique catalog hits, so hidden entries that
            // never appear in the list only cost a few extra scrolls.
            let behind = expected > matched_titles.len().saturating_add(3);
            let refocus = unchanged && new_rows == 0 && behind && idle_frames < 4;
            if unchanged && new_rows == 0 && (!behind || idle_frames >= 4) {
                log_info!(
                    "[achievement] 列表画面未变，已到分类底部: {} frame={} capture={}ms ocr={}ms",
                    "[achievement] list image unchanged, at category bottom: {} frame={} capture={}ms ocr={}ms",
                    category_label,
                    frame_idx,
                    capture_ms,
                    ocr_ms
                );
                progress_log(&format!(
                    "pixel_stop cat={category_label} frame={frame_idx} capture_ms={capture_ms} ocr_ms={ocr_ms} seen={} expected={expected}",
                    cat_titles.len()
                ));
                break;
            }

            if idle_frames >= IDLE_FRAMES_BEFORE_STOP {
                log_info!(
                    "[achievement] 无新标题，离开分类 {} (ocr {:.1}s)",
                    "[achievement] no new titles, leaving category {} (ocr {:.1}s)",
                    category_label,
                    ocr_ms_total as f64 / 1000.0
                );
                progress_log(&format!(
                    "idle_stop cat={category_label} frame={frame_idx} ocr_s={:.1}",
                    ocr_ms_total as f64 / 1000.0
                ));
                if self.config.dump_images {
                    save_dump(&format!("{prefix}_idle_{frame_idx:03}.png"), &list_img);
                }
                break;
            }

            let t_scroll = Instant::now();
            if refocus {
                ctrl.click_at(ACHIEVEMENT_HEADER_POS.0, ACHIEVEMENT_HEADER_POS.1);
                yas::utils::sleep(80);
                progress_log(&format!(
                    "respin cat={category_label} frame={frame_idx} matched={} expected={expected}",
                    matched_titles.len()
                ));
            }
            // Re-detect each step. A short list's thumb is tall; a stale
            // park from the previous category lands on the cream panel.
            let groove = self.scrollbar_groove(ctrl);
            self.scroll_list(ctrl, LIST_WHEEL_TICKS, groove);
            let scroll_ms = t_scroll.elapsed().as_millis();
            progress_log(&format!(
                "scroll cat={category_label} frame={frame_idx} ticks={LIST_WHEEL_TICKS} ms={scroll_ms}"
            ));
            log_debug!(
                "[achievement] 滚轮 {} tick + sleep {}ms 用时 {}ms",
                "[achievement] wheel {} ticks + sleep {}ms took {}ms",
                LIST_WHEEL_TICKS,
                self.config.scroll_delay,
                scroll_ms
            );

            prev_list = Some(list_img);
            frame_idx += 1;
        }

        if let Some(name) = category_name {
            if let Some(cov) = self.catalog.coverage(name, &cat_titles) {
                let missing = cov.missing.join(",");
                progress_log(&format!(
                    "coverage cat={} expected={} matched={} missing_n={} missing={}",
                    cov.name,
                    cov.expected,
                    cov.matched,
                    cov.missing.len(),
                    missing
                ));
                log_info!(
                    "[achievement] 分类覆盖 {}: 目录{} 识别{} 缺失{}",
                    "[achievement] category coverage {}: catalog {} recognized {} missing {}",
                    cov.name,
                    cov.expected,
                    cov.matched,
                    cov.missing.len()
                );
                if !cov.missing.is_empty() {
                    log_warn!(
                        "[achievement] 未扫到的标题: {}",
                        "[achievement] missing titles: {}",
                        missing
                    );
                }
            }
        }

        Ok(())
    }

    fn park_cursor_on_scrollbar(&self, ctrl: &mut GenshinGameController, groove: (f64, f64)) {
        // Backpack scrolling only move_to's the grid — it does not click.
        // Clicking the achievement thumb starts a drag and eats the wheel.
        // Header click (caller) already cleared card hover; the OS cursor on
        // the groove is what `WM_MOUSEWHEEL` hit-tests.
        ctrl.focus_game_window_quick();
        ctrl.move_to(groove.0, groove.1);
        yas::utils::sleep(LIST_HOVER_SETTLE_MS);
        progress_log(&format!(
            "park_scrollbar x={:.0} y={:.0}",
            groove.0, groove.1
        ));
    }

    fn scrollbar_groove(&self, ctrl: &GenshinGameController) -> (f64, f64) {
        if let Ok(frame) = ctrl.capture_game() {
            if let Some(list) = detect_list_rect(&frame) {
                let px = detect_scrollbar_thumb(&frame, list)
                    .map(|(x, _)| x)
                    .unwrap_or_else(|| {
                        list.x
                            .saturating_add(list.w)
                            .saturating_add(LIST_SCROLLBAR_INSET)
                            .min(frame.width().saturating_sub(2))
                    });
                // Mid-track: list.y+72 sits beside the namecard banner on
                // categories like 浮涌的阴影之地, where the wheel hits chrome
                // and the list never moves.
                let py = list
                    .y
                    .saturating_add(list.h / 2)
                    .min(frame.height().saturating_sub(2));
                return pixel_to_base(px, py, &ctrl.scaler);
            }
        }
        LIST_SCROLLBAR_POS
    }

    /// Same tick path as the backpack scanner: `mouse_scroll(1)` is one detent.
    /// Sleep after every notch so Windows cannot coalesce 8 ticks into a jump.
    fn scroll_list(&self, ctrl: &mut GenshinGameController, ticks: i32, groove: (f64, f64)) {
        self.park_cursor_on_scrollbar(ctrl, groove);
        for _ in 0..ticks {
            ctrl.mouse_scroll(1);
            yas::utils::sleep(LIST_TICK_DELAY_MS);
        }
        yas::utils::sleep(self.config.scroll_delay as u32);
    }

    /// One-tick capture/OCR loop so we can measure pixels and titles per detent.
    fn calibrate_list_scroll(
        &self,
        ctrl: &mut GenshinGameController,
        rec: &OcrPool,
    ) -> Result<()> {
        const NOTCH_FRAMES: u32 = 24;
        const MICRO_FRAMES: u32 = 8;
        log_info!(
            "[achievement] 滚轮标定：先 {} 次整格，再 {} 次 1/8 格",
            "[achievement] scroll calibrate: {} full notches, then {} 1/8-notch steps",
            NOTCH_FRAMES,
            MICRO_FRAMES
        );
        progress_log("scroll_calibrate_start");
        let groove = self.scrollbar_groove(ctrl);
        ctrl.click_at(ACHIEVEMENT_HEADER_POS.0, ACHIEVEMENT_HEADER_POS.1);
        yas::utils::sleep(80);
        self.park_cursor_on_scrollbar(ctrl, groove);
        if self.config.dump_images {
            if let Ok(frame) = ctrl.capture_game() {
                save_dump("scrollcal_full.png", &frame);
            }
        }

        let mut prev_titles: Vec<String> = Vec::new();
        let mut prev_list: Option<RgbImage> = None;
        for frame_idx in 0..NOTCH_FRAMES {
            if ctrl.cancel_token().check_rmb() {
                bail!("cancelled");
            }
            let Some((list_img, titles)) = self.capture_list_titles(ctrl, rec) else {
                bail!("未找到成就列表 / Achievement list not found");
            };
            if self.config.dump_images {
                save_dump(&format!("scrollcal_notch_{frame_idx:03}.png"), &list_img);
            }
            let shift = prev_list
                .as_ref()
                .and_then(|prev| vertical_shift_hint(prev, &list_img));
            let appeared = titles.iter().filter(|t| !prev_titles.contains(t)).count();
            let disappeared = prev_titles.iter().filter(|t| !titles.contains(t)).count();
            progress_log(&format!(
                "cal_notch i={frame_idx} titles={} appeared={appeared} disappeared={disappeared} shift={:?} first={}",
                titles.len(),
                shift,
                titles.first().cloned().unwrap_or_default()
            ));
            log_info!(
                "[achievement] notch {} titles={} +{} -{} shift={:?} '{}'",
                "[achievement] notch {} titles={} +{} -{} shift={:?} '{}'",
                frame_idx,
                titles.len(),
                appeared,
                disappeared,
                shift,
                titles.first().cloned().unwrap_or_default()
            );
            prev_titles = titles;
            prev_list = Some(list_img);
            self.scroll_list(ctrl, 1, groove);
        }

        let mut moved = false;
        for frame_idx in 0..MICRO_FRAMES {
            if ctrl.cancel_token().check_rmb() {
                bail!("cancelled");
            }
            let Some(list_img) = self.capture_list_image(ctrl) else {
                break;
            };
            if self.config.dump_images {
                save_dump(&format!("scrollcal_micro_{frame_idx:03}.png"), &list_img);
            }
            let shift = prev_list
                .as_ref()
                .and_then(|prev| vertical_shift_hint(prev, &list_img));
            if shift.unwrap_or(0).abs() >= 2 {
                moved = true;
            }
            progress_log(&format!("cal_micro i={frame_idx} shift={shift:?}"));
            log_info!(
                "[achievement] 1/8-notch {} shift={:?}",
                "[achievement] 1/8-notch {} shift={:?}",
                frame_idx,
                shift
            );
            prev_list = Some(list_img);
            self.scroll_list(ctrl, 1, groove);
        }
        if !moved {
            log_info!(
                "[achievement] Genshin 忽略小于 WHEEL_DELTA 的滚轮增量，整格滚动即可",
                "[achievement] Genshin ignores sub-notch wheel deltas; stay on full detents"
            );
            progress_log("cal_micro_ignored");
        }
        progress_log("scroll_calibrate_done");
        Ok(())
    }

    fn capture_list_image(&self, ctrl: &GenshinGameController) -> Option<RgbImage> {
        let frame = ctrl.capture_game().ok()?;
        let list_rect = detect_list_rect(&frame)?;
        crop_pixel(&frame, list_rect)
    }

    fn capture_list_titles(
        &self,
        ctrl: &GenshinGameController,
        rec: &OcrPool,
    ) -> Option<(RgbImage, Vec<String>)> {
        let list_img = self.capture_list_image(ctrl)?;
        let bands = split_row_bands(&list_img, true);
        let mut titles = Vec::new();
        for band in &bands {
            let Some(row) = crop_row(&list_img, band) else {
                continue;
            };
            if is_progress_banner(&row) {
                continue;
            }
            let ocr = rec.get();
            if let Some(crop) = crop_title(&row) {
                if let Ok(text) = ocr.image_to_text(&crop, false) {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        titles.push(text);
                    }
                }
            }
        }
        Some((list_img, titles))
    }

    fn click_next_category(
        &self,
        ctrl: &mut GenshinGameController,
        slot: usize,
        category_index: usize,
    ) {
        let slot = slot.min(CATEGORY_VISIBLE.saturating_sub(1));
        let fallback_y = CATEGORY_FIRST_Y + CATEGORY_STEP_Y * slot as f64;
        let mut y = fallback_y;
        if let Ok(frame) = ctrl.capture_game() {
            if self.config.dump_images {
                save_dump(&format!("c{category_index:02}_before_click.png"), &frame);
            }
            if let Some(rect) = detect_selected_category_rect(&frame) {
                let (_bx, cy) = pixel_to_base(
                    rect.x + rect.w / 2,
                    rect.y + rect.h / 2,
                    &ctrl.scaler,
                );
                // One row below the selected card. The inset crop is shorter
                // than the real row, so adding STEP from the center is required.
                y = (cy + CATEGORY_STEP_Y).clamp(CATEGORY_FIRST_Y, 1030.0);
                progress_log(&format!(
                    "selected_rect cat={category_index} y={} h={} center={cy:.0} next={y:.0}",
                    rect.y, rect.h
                ));
            }
        }
        progress_log(&format!(
            "click_next cat={category_index} slot={slot} y={y:.0}"
        ));
        log_info!(
            "[achievement] 点击下一分类 y={:.0}（槽{}）",
            "[achievement] clicking next category y={:.0} (slot {})",
            y,
            slot + 1
        );
        ctrl.click_at(CATEGORY_X, y);
        yas::utils::sleep(self.config.category_delay as u32);
        progress_log(&format!(
            "click_next_done cat={category_index} slot={slot} y={y:.0}"
        ));
        if self.config.dump_images {
            if let Ok(frame) = ctrl.capture_game() {
                save_dump(&format!("c{category_index:02}_after_click.png"), &frame);
            }
        }
    }

    fn read_selected_category(
        &self,
        ctrl: &GenshinGameController,
        rec: &OcrPool,
    ) -> (Option<String>, bool) {
        let Some(crop) = self.capture_selected_category(ctrl) else {
            return (None, false);
        };
        let ocr = rec.get();
        let Ok(text) = ocr.image_to_text(&crop, false) else {
            return (None, false);
        };
        let mut complete = category_looks_complete(&text);
        if !complete {
            // Percentage sits on the second line; a single OCR pass often drops it.
            let h = crop.height();
            let y = h / 2;
            if h > y {
                let bottom = crop.view(0, y, crop.width(), h - y).to_image();
                if let Ok(bottom_text) = ocr.image_to_text(&bottom, false) {
                    complete = category_looks_complete(&bottom_text);
                }
            }
        }
        (pick_category_name(&text), complete)
    }

    fn capture_selected_category(&self, ctrl: &GenshinGameController) -> Option<RgbImage> {
        let frame = ctrl.capture_game().ok()?;
        let rect = detect_selected_category_rect(&frame)?;
        crop_pixel(&frame, rect)
    }
}

fn is_plausible_category_name(name: &str) -> bool {
    let letters = name
        .chars()
        .filter(|c| !c.is_ascii_digit() && *c != '%' && *c != ' ')
        .count();
    letters >= 2
}

fn is_plausible_row_title(title: &str) -> bool {
    let text = normalize_text(title);
    if text.contains("达成进度") {
        return false;
    }
    let cjk = text
        .chars()
        .filter(|c| *c >= '\u{4e00}' && *c <= '\u{9fff}')
        .count();
    let letters = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
    cjk >= 2 || letters >= 3
}

fn pick_category_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_plausible_category_name(&normalize_text(trimmed)) {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn category_looks_complete(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("100%") || compact.contains("100％")
}

fn is_progress_banner(row: &RgbImage) -> bool {
    if row.width() < 32 || row.height() < 16 {
        return false;
    }
    // The gold bar sits in the lower half of the namecard banner. Mid-row
    // samples hit the "达成进度" title and miss it.
    for y_num in [4u32, 5, 6, 7, 8] {
        let y = (row.height() * y_num / 10).min(row.height().saturating_sub(1));
        let mut gold = 0u32;
        let mut x = 0u32;
        while x < row.width() {
            let p = row.get_pixel(x, y);
            if p[0] > 180 && p[1] > 140 && p[2] < 110 {
                gold += 1;
            }
            x += 2;
        }
        if gold > row.width() / 16 {
            return true;
        }
    }
    false
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

fn row_key(title: &str, subtitle: &str) -> String {
    let title = normalize_text(title);
    let subtitle = normalize_text(subtitle);
    if title.is_empty() && subtitle.is_empty() {
        String::new()
    } else {
        format!("{title}|{subtitle}")
    }
}

fn vertical_shift_hint(a: &RgbImage, b: &RgbImage) -> Option<i32> {
    if a.dimensions() != b.dimensions() {
        return None;
    }
    let (w, h) = a.dimensions();
    if h < 32 || w < 32 {
        return None;
    }
    let x0 = ((w as f32) * 0.12) as u32;
    let x1 = ((w as f32) * 0.55).max(x0 as f32 + 1.0) as u32;
    let max_shift = 400i32.min(h as i32 / 2);
    let mut best = (u64::MAX, 0i32);
    for s in -max_shift..=max_shift {
        let mut sum = 0u64;
        let mut n = 0u64;
        let (ay0, by0, rows) = if s >= 0 {
            (s as u32, 0u32, h.saturating_sub(s as u32))
        } else {
            (0u32, (-s) as u32, h.saturating_sub((-s) as u32))
        };
        let mut y = 0u32;
        while y < rows {
            let mut x = x0;
            while x < x1 {
                let pa = a.get_pixel(x, ay0 + y);
                let pb = b.get_pixel(x, by0 + y);
                sum += pa[0].abs_diff(pb[0]) as u64
                    + pa[1].abs_diff(pb[1]) as u64
                    + pa[2].abs_diff(pb[2]) as u64;
                n += 1;
                x += 8;
            }
            y += 8;
        }
        if n > 0 {
            let mean = sum / n;
            if mean < best.0 {
                best = (mean, s);
            }
        }
    }
    Some(best.1)
}

fn dump_slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "unknown".to_string()
    } else {
        s
    }
}

fn save_dump(name: &str, image: &RgbImage) {
    let _ = std::fs::create_dir_all(DUMP_DIR);
    let path = format!("{DUMP_DIR}/{name}");
    if let Err(error) = image.save(&path) {
        log_debug!(
            "[achievement] 无法保存调试图 {}: {:#}",
            "[achievement] could not save dump {}: {:#}",
            path,
            error
        );
    }
}

fn progress_log(line: &str) {
    let _ = std::fs::create_dir_all(DUMP_DIR);
    let path = format!("{DUMP_DIR}/progress.log");
    let ts_ms = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{ts_ms} {line}");
    }
}
