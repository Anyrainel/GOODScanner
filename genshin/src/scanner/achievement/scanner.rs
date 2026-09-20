use std::collections::{BTreeSet, HashMap};
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
    CATEGORY_FIRST_Y, CATEGORY_STEP_Y, CATEGORY_VISIBLE, CATEGORY_X, IDLE_FRAMES_BEFORE_STOP,
    LIST_WHEEL_TICKS, MAX_CATEGORIES, MAX_LIST_FRAMES, OVERVIEW_FIRST_CATEGORY_POS,
    PAIMON_ACHIEVEMENT_POS, PAIMON_MENU_DELAY,
};
use super::recognize::recognize_row;
use super::split::{
    crop_row, detect_list_rect, detect_selected_category_rect, list_nearly_equal, split_row_bands,
    PixelRect,
};
use crate::scanner::common::coord_scaler::CoordScaler;
use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::ocr_pool::SharedOcrPools;
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
        let mut staged_done: HashMap<String, usize> = HashMap::new();
        let started = SystemTime::now();
        let mut category_slot = 0usize;
        let mut same_name_retries = 0u32;
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

            let selected_name = {
                progress_log(&format!("read_selected cat_index={category_index}"));
                let name = self.read_selected_category(ctrl, pools);
                progress_log(&format!(
                    "read_selected_done cat_index={category_index} name={}",
                    name.as_deref().unwrap_or("-")
                ));
                name
            };
            let mut name_ok = false;
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
                    name_ok = true;
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
                pools,
                &mut completed,
                &mut seen_rows,
                &mut staged_done,
                &report,
                category_index,
                &category_label,
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
            // After the last category, clicking slot 8 wraps or stays put.
            // Right-hand titles then repeat what we already recognized.
            // Skip this when the left-name OCR was garbage — that re-scans the
            // current list and would look like a wrap.
            if category_index > 0 && name_ok && seen_rows.len() == rows_before {
                log_info!(
                    "[achievement] 右侧条目均已识别，已到最后一个分类",
                    "[achievement] right-hand titles already recognized; last category reached"
                );
                progress_log(&format!(
                    "titles_already_seen_stop name={category_label}"
                ));
                break;
            }

            if category_slot + 1 < CATEGORY_VISIBLE {
                category_slot += 1;
            }
            self.click_next_category(ctrl, category_slot, category_index);
        }

        let staged_assigned = self.assign_staged_ids(&mut completed, &staged_done);
        if staged_assigned > 0 {
            log_info!(
                "[achievement] 按同名条目数量补全了 {} 个阶段成就",
                "[achievement] assigned {} staged achievements from same-title counts",
                staged_assigned
            );
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
        pools: &SharedOcrPools,
        completed: &mut BTreeSet<u32>,
        seen_rows: &mut BTreeSet<String>,
        staged_done: &mut HashMap<String, usize>,
        report: &dyn Fn(usize, &str),
        category_index: usize,
        category_label: &str,
    ) -> Result<()> {
        let cancel = ctrl.cancel_token();
        let mut idle_frames = 0u32;
        let mut frame_idx = 0u32;
        let mut click_pos: Option<(f64, f64)> = None;
        let mut prev_list: Option<RgbImage> = None;
        let mut ocr_ms_total = 0u128;
        let prefix = format!("c{category_index:02}_{category_label}");

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
                bail!("未找到成就列表 / Achievement list not found");
            };
            let Some(list_img) = crop_pixel(&frame, list_rect) else {
                bail!("未找到成就列表 / Achievement list not found");
            };

            if click_pos.is_none() {
                click_pos = Some(self.click_pos_for_list(ctrl, list_rect, &list_img));
            }

            let unchanged = prev_list
                .as_ref()
                .map(|prev| list_nearly_equal(prev, &list_img))
                .unwrap_or(false);
            if self.config.dump_images && frame_idx == 0 {
                save_dump(&format!("{prefix}_full.png"), &frame);
            }
            if self.config.dump_images && (frame_idx == 0 || unchanged || frame_idx % 10 == 0) {
                let tag = if unchanged { "bottom" } else { "list" };
                save_dump(&format!("{prefix}_{tag}_{frame_idx:03}.png"), &list_img);
            }

            if unchanged {
                log_info!(
                    "[achievement] 列表画面未变，已到分类底部: {} frame={} capture={}ms idle={}",
                    "[achievement] list image unchanged, at category bottom: {} frame={} capture={}ms idle={}",
                    category_label,
                    frame_idx,
                    capture_ms,
                    idle_frames
                );
                progress_log(&format!(
                    "pixel_stop cat={category_label} frame={frame_idx} capture_ms={capture_ms}"
                ));
                break;
            }

            let keep_last = idle_frames >= 1;
            let bands = split_row_bands(&list_img, keep_last);
            let new_before = seen_rows.len();
            let mut unmatched = 0usize;

            let t_ocr = Instant::now();
            let rows: Vec<(usize, RgbImage)> = bands
                .iter()
                .enumerate()
                .filter_map(|(i, band)| {
                    let row = crop_row(&list_img, band)?;
                    if is_progress_banner(&row) {
                        None
                    } else {
                        Some((i, row))
                    }
                })
                .collect();
            progress_log(&format!(
                "ocr_start cat={category_label} frame={frame_idx} rows={}",
                rows.len()
            ));
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
            let ocr_ms = t_ocr.elapsed().as_millis();
            ocr_ms_total += ocr_ms;

            for (i, row, recognized) in recognized {
                if !is_plausible_row_title(&recognized.title) {
                    continue;
                }
                let key = row_key(&recognized.title, &recognized.subtitle);
                if key.is_empty() {
                    continue;
                }
                if !seen_rows.insert(key) {
                    continue;
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
                } else {
                    unmatched += 1;
                    let staged = self.catalog.title_id_count(&recognized.title);
                    let reason = if staged > 1 {
                        format!("staged:{staged}")
                    } else {
                        "no-match".to_string()
                    };
                    if recognized.done && staged > 1 {
                        let title_key = normalize_text(&recognized.title);
                        *staged_done.entry(title_key).or_insert(0) += 1;
                    }
                    log_warn!(
                        "[achievement] 未匹配({}): title='{}' subtitle='{}' status='{}' date='{}'",
                        "[achievement] unmatched({}): title='{}' subtitle='{}' status='{}' date='{}'",
                        reason,
                        recognized.title,
                        recognized.subtitle,
                        recognized.status,
                        recognized.date
                    );
                    // Staged titles are expected unmatched; dump real OCR misses.
                    if self.config.dump_images && staged <= 1 {
                        save_dump(
                            &format!("{prefix}_miss_{frame_idx:03}_{i}.png"),
                            &row,
                        );
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
                "frame cat={category_label} i={frame_idx} capture_ms={capture_ms} ocr_ms={ocr_ms} rows={} new={new_rows} unmatched={unmatched} idle={idle_frames} done={}",
                bands.len(),
                completed.len()
            ));

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

            if let Some((x, y)) = click_pos {
                let t_scroll = Instant::now();
                ctrl.click_at(x, y);
                for _ in 0..LIST_WHEEL_TICKS {
                    ctrl.mouse_scroll(1);
                }
                yas::utils::sleep(self.config.scroll_delay as u32);
                let scroll_ms = t_scroll.elapsed().as_millis();
                progress_log(&format!(
                    "scroll cat={category_label} frame={frame_idx} ms={scroll_ms}"
                ));
                log_debug!(
                    "[achievement] 滚轮 {} tick + sleep {}ms 用时 {}ms",
                    "[achievement] wheel {} ticks + sleep {}ms took {}ms",
                    LIST_WHEEL_TICKS,
                    self.config.scroll_delay,
                    scroll_ms
                );
            }

            prev_list = Some(list_img);
            frame_idx += 1;
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
        pools: &SharedOcrPools,
    ) -> Option<String> {
        let crop = self.capture_selected_category(ctrl)?;
        let ocr = pools.v4().get();
        let text = ocr.image_to_text(&crop, false).ok()?;
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    fn capture_selected_category(&self, ctrl: &GenshinGameController) -> Option<RgbImage> {
        let frame = ctrl.capture_game().ok()?;
        let rect = detect_selected_category_rect(&frame)?;
        crop_pixel(&frame, rect)
    }

    /// ggartifact titles repeat across stages and have no descriptions.
    /// Count completed same-title rows and take that many IDs in catalog order.
    fn assign_staged_ids(
        &self,
        completed: &mut BTreeSet<u32>,
        staged_done: &HashMap<String, usize>,
    ) -> usize {
        let mut assigned = 0usize;
        for (title, count) in staged_done {
            if *count == 0 {
                continue;
            }
            let ids = self.catalog.ids_for_title(title);
            if ids.len() < 2 {
                continue;
            }
            for id in ids.into_iter().take(*count) {
                if completed.insert(id) {
                    assigned += 1;
                }
            }
        }
        assigned
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
    normalize_text(title)
        .chars()
        .filter(|c| *c >= '\u{4e00}' && *c <= '\u{9fff}')
        .count()
        >= 2
}

fn is_progress_banner(row: &RgbImage) -> bool {
    if row.width() < 32 || row.height() < 16 {
        return false;
    }
    let y = row.height() / 2;
    let mut gold = 0u32;
    for x in 0..row.width() {
        let p = row.get_pixel(x, y);
        if p[0] > 180 && p[1] > 140 && p[2] < 110 {
            gold += 1;
        }
    }
    gold > row.width() / 8
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

fn dump_slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c })
        .filter(|c| !c.is_control())
        .take(24)
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
