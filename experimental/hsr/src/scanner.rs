use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use image::RgbImage;
use sha2::{Digest, Sha256};
use yas::{cancel::CancelToken, capture::CaptureMethod};

use crate::{
    annotator,
    device::{HsrDevice, InputCommand, WindowsHsrDevice},
    error::{hints, HsrError, HsrResult},
    layout,
    manager::{
        ManagedField, ManagedGearObservation, ManagedState, ManagerMutationDevice, MutationScope,
        VisibleGearMatcher, VisibleStat,
    },
    model::{
        CoverageLevel, EvidenceKind, InventoryCoverage, ObservationEvidence, ObservationSnapshot,
        ObservedCharacter, ObservedLightCone, TrailblazerIdentity, OBSERVATION_SCHEMA_VERSION,
    },
    observation::ValidatedObservationSnapshot,
    ocr::{
        detect_discard_state, detect_icon_state, InventoryKind, OcrField, OcrReader,
        PaddleOcrReader, PanelParser, ParsedCharacterPanel, ParsedGearPanel, StatsPanelLayout,
        MANAGED_ICON_CONFIDENCE_THRESHOLD,
    },
    reference::ReferenceCache,
    scan_timing::ScanTimings,
    scanner_export::{path_name, trailblazer_gender, CaptureExportDetails},
    vision::{
        discover_inventory_grid, frame_fingerprint, frames_similar, glyphs_match, selected_card,
        GridGeometry, NormRect, Point, SelectedCard,
    },
};

#[cfg(test)]
use crate::vision::{
    classify_inventory_scroll, inventory_scrollbar_bottom_confidence, selected_cell, ScrollEvidence,
};

/// Name plate only. The portrait bar and the 3D preview sit inside the old
/// wide header rect and keep moving while the same character stays selected.
const CHARACTER_IDENTITY_REGION: NormRect = layout::CHARACTER_NAME;
/// Left inventory grid, including the selection border. The detail panel can
/// stay identical for two copies; the selection still moves when `d` works.
const INVENTORY_GRID_REGION: NormRect = NormRect::new(0.015, 0.14, 0.70, 0.80);

const MENU_OPEN_POLLS: usize = 8;
/// Half a backpack row: a larger vertical jump of the selection frame is a
/// row step, a smaller one is cross-fade jitter.
const SELECTION_ROW_SHIFT: f64 = 0.08;
/// Lowercase fragments of the top-left menu title (「背包」 / 「角色详情」).
const INVENTORY_TITLE: &[&str] = &["背包", "inventory"];
const CHARACTER_TITLE: &[&str] = &["角色", "character"];

/// Shared by live traversal and the shipped executable's drag diagnostic.
pub fn drag_character_page<D: HsrDevice>(
    device: &mut D,
    forward: bool,
    timings: &ScanTimings,
) -> HsrResult<RgbImage> {
    let (from, to) = layout::character_page_drag(forward);
    device.input(InputCommand::Drag { from, to })?;
    device.wait(Duration::from_millis(timings.character_page_ms))?;
    device.capture_client()
}

fn character_bar_unchanged(before: &RgbImage, after: &RgbImage) -> HsrResult<bool> {
    // Compare faces, excluding the animated starfield and selection rings.
    for slot in 0..layout::CHARACTER_PAGE_SIZE {
        let center = layout::character_portrait(slot);
        let face = NormRect::new(center.x - 0.004, center.y - 0.0075, 0.008, 0.015);
        if !frames_similar(&face.crop(before)?, &face.crop(after)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanTargets {
    pub characters: bool,
    pub light_cones: bool,
    pub gear: bool,
}

impl ScanTargets {
    pub const fn all() -> Self {
        Self {
            characters: true,
            light_cones: true,
            gear: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub targets: ScanTargets,
    /// None selects BitBlt for SDR and WGC for HDR.
    pub capture_method: Option<CaptureMethod>,
    pub hdr_mode: bool,
    pub timings: ScanTimings,
    /// Baseline wheel detents for one full visible inventory page. The runtime
    /// scales this down for a clamped final-page row advance and verifies the
    /// resulting row displacement from screenshots before scanning continues.
    #[cfg(test)]
    pub inventory_scroll_ticks_per_page: usize,
    #[cfg(test)]
    pub scroll_tick_delay: Duration,
    /// Optional scan caps. Zero scans the whole category; these never reject
    /// a backpack merely because its total is larger than the requested cap.
    pub max_light_cones: usize,
    pub max_gear: usize,
    /// Stop after this many parsed inventory entries. `None` walks the OCR
    /// quantity. Used by dump sessions so a large backpack can still be sampled.
    pub scan_item_limit: Option<usize>,
    pub max_characters: usize,
    /// Without it the Trailblazer's header cannot be resolved and is omitted.
    pub trailblazer: Option<TrailblazerIdentity>,
    /// GOODScanner-style OCR dump. When true, crops and full frames are written
    /// under `debug_images/` as a side effect of parsing; click/wait code is
    /// unchanged.
    pub dump_images: bool,
    /// Retain fully parsed entries on an explicit device cancellation. Manager
    /// scans always leave this off: incomplete inventories cannot authorize actions.
    pub save_on_cancel: bool,
    /// Stop on the first unreadable entry for debugging. Normal export scans
    /// skip individual OCR failures; manager matching always enables this.
    pub stop_on_failure: bool,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            targets: ScanTargets::all(),
            capture_method: None,
            hdr_mode: false,
            timings: ScanTimings::default(),
            #[cfg(test)]
            inventory_scroll_ticks_per_page: 25,
            #[cfg(test)]
            scroll_tick_delay: Duration::from_millis(10),
            max_light_cones: 0,
            max_gear: 0,
            scan_item_limit: None,
            max_characters: 0,
            trailblazer: None,
            dump_images: false,
            save_on_cancel: false,
            stop_on_failure: false,
        }
    }
}

impl ScanConfig {
    pub fn effective_capture_method(&self) -> CaptureMethod {
        self.capture_method
            .unwrap_or_else(|| CaptureMethod::for_hdr_mode(self.hdr_mode))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScannedGearItem {
    pub ordinal: usize,
    pub parsed: ParsedGearPanel,
    pub semantic_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanResult {
    pub observations: ValidatedObservationSnapshot,
    pub gear_items: Vec<ScannedGearItem>,
    pub coverage: InventoryCoverage,
    pub export_details: CaptureExportDetails,
}

#[derive(Debug, Clone, PartialEq)]
struct InventoryScan<T> {
    items: Vec<T>,
    coverage: CoverageLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharacterTab {
    Details,
    Traces,
    Eidolons,
}

impl CharacterTab {
    fn order(reverse: bool) -> [Self; 3] {
        if reverse {
            [Self::Eidolons, Self::Traces, Self::Details]
        } else {
            [Self::Details, Self::Traces, Self::Eidolons]
        }
    }

    fn button(self) -> Point {
        match self {
            Self::Details => layout::DETAILS_BUTTON,
            Self::Traces => layout::TRACES_BUTTON,
            Self::Eidolons => layout::EIDOLONS_BUTTON,
        }
    }
}

struct CharacterPages {
    parsed: HsrResult<ParsedCharacterPanel>,
    traces: Option<HsrResult<crate::scanner_export::CharacterDetails>>,
    details: RgbImage,
    last: RgbImage,
}

pub struct HsrScanner<D, R> {
    device: D,
    parser: PanelParser<R>,
    references: ReferenceCache,
    config: ScanConfig,
    uid: Option<u64>,
    uid_candidate: Option<u64>,
    uid_error_logged: bool,
    observer: Option<crate::scan_progress::ScanObserver>,
}

impl HsrScanner<WindowsHsrDevice, PaddleOcrReader> {
    pub fn live(references: ReferenceCache, config: ScanConfig) -> HsrResult<Self> {
        Self::live_with_cancel(references, config, CancelToken::new())
    }

    /// Build a live scanner whose device observes the caller-owned per-run
    /// cancellation token. Clones of the token can stop an active GUI worker.
    pub fn live_with_cancel(
        references: ReferenceCache,
        config: ScanConfig,
        cancel: CancelToken,
    ) -> HsrResult<Self> {
        let device = WindowsHsrDevice::locate_with_cancel(
            config.effective_capture_method(),
            config.hdr_mode,
            cancel,
        )?;
        let reader = PaddleOcrReader::new()?;
        Ok(Self::new(device, reader, references, config))
    }
}

impl<D: HsrDevice, R: OcrReader> HsrScanner<D, R> {
    pub fn new(device: D, reader: R, references: ReferenceCache, config: ScanConfig) -> Self {
        Self {
            device,
            parser: PanelParser::new(reader),
            references,
            config,
            uid: None,
            uid_candidate: None,
            uid_error_logged: false,
            observer: None,
        }
    }

    pub fn with_observer(mut self, observer: crate::scan_progress::ScanObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    fn report(
        &self,
        category: crate::scan_progress::ScanCategory,
        event: crate::scan_progress::ScanEvent,
    ) {
        if let Some(observer) = &self.observer {
            observer(category, event);
        }
    }

    pub fn device(&self) -> &D {
        &self.device
    }

    pub fn parser(&self) -> &PanelParser<R> {
        &self.parser
    }

    pub fn references(&self) -> &ReferenceCache {
        &self.references
    }

    /// Capture a fresh, complete Relic inventory for manager lock/unlock.
    /// Identity comes from [`Self::scan_gear`] — the same parser and crops as
    /// the screenshot scanner. The extra manager step is requiring complete
    /// coverage before any mutation clicks.
    ///
    /// Equip-for-character and recent-Relic rescan are not implemented yet.
    pub fn scan_manager_inventory(&mut self) -> HsrResult<Vec<ManagedGearObservation>> {
        annotator::init(self.config.dump_images);
        let result = self.scan_manager_inventory_inner();
        annotator::flush();
        result
    }

    fn scan_manager_inventory_inner(&mut self) -> HsrResult<Vec<ManagedGearObservation>> {
        self.device.focus_and_verify()?;
        let scan = self.scan_gear()?;
        if scan.coverage != CoverageLevel::Complete {
            return Err(HsrError::new(
                "HSR-MANAGER-INVENTORY-INCOMPLETE",
                hints::SCREEN_INVALID,
                "manager preview requires both quantity reads to agree and the next-item walk to consume that quantity; gear coverage remained unknown",
            ));
        }
        Ok(scan
            .items
            .into_iter()
            .map(|item| managed_observation(&item.parsed))
            .collect())
    }

    pub fn scan(mut self) -> HsrResult<ScanResult> {
        annotator::init(self.config.dump_images);
        if self.config.dump_images {
            yas::log_info!(
                "已开启 OCR 截图转储 → debug_images/",
                "OCR image dumping enabled → debug_images/"
            );
        }
        let result = self.scan_inner();
        annotator::flush();
        result
    }

    fn scan_inner(&mut self) -> HsrResult<ScanResult> {
        self.device.focus_and_verify()?;
        let mut characters = Vec::new();
        let mut light_cones = Vec::new();
        let mut gear_items = Vec::new();

        let mut export_details = CaptureExportDetails::default();
        let character_coverage = if self.config.targets.characters {
            self.report(
                crate::scan_progress::ScanCategory::Characters,
                crate::scan_progress::ScanEvent::Started,
            );
            let scan = self.scan_characters()?;
            self.report(
                crate::scan_progress::ScanCategory::Characters,
                crate::scan_progress::ScanEvent::Finished {
                    recognized: scan.items.len(),
                    complete: scan.coverage == CoverageLevel::Complete,
                },
            );
            characters = scan.items;
            export_details = scan.details;
            scan.coverage
        } else {
            CoverageLevel::Unknown
        };
        let light_cone_coverage = if self.config.targets.light_cones && !self.device.is_cancelled()
        {
            self.report(
                crate::scan_progress::ScanCategory::LightCones,
                crate::scan_progress::ScanEvent::Started,
            );
            let scan = self.scan_light_cones()?;
            self.report(
                crate::scan_progress::ScanCategory::LightCones,
                crate::scan_progress::ScanEvent::Finished {
                    recognized: scan.items.len(),
                    complete: scan.coverage == CoverageLevel::Complete,
                },
            );
            light_cones = scan.items;
            scan.coverage
        } else {
            CoverageLevel::Unknown
        };
        let gear_coverage = if self.config.targets.gear && !self.device.is_cancelled() {
            self.report(
                crate::scan_progress::ScanCategory::Gear,
                crate::scan_progress::ScanEvent::Started,
            );
            let scan = self.scan_gear()?;
            self.report(
                crate::scan_progress::ScanCategory::Gear,
                crate::scan_progress::ScanEvent::Finished {
                    recognized: scan.items.len(),
                    complete: scan.coverage == CoverageLevel::Complete,
                },
            );
            gear_items = scan.items;
            scan.coverage
        } else {
            CoverageLevel::Unknown
        };

        if self.device.is_cancelled() && !self.config.save_on_cancel {
            return Err(HsrError::new(
                "HSR-SCAN-CANCELLED",
                hints::CANCELLED,
                "scan stopped before all selected categories completed",
            ));
        }
        let coverage = InventoryCoverage {
            characters: character_coverage,
            light_cones: light_cone_coverage,
            relics: gear_coverage,
        };
        let snapshot = ObservationSnapshot {
            schema_version: OBSERVATION_SCHEMA_VERSION,
            evidence: ObservationEvidence {
                kind: EvidenceKind::ScreenCapture,
                revision: "goodscanner-hsr-screen-v2".to_string(),
                coverage,
            },
            characters,
            light_cones,
            gear: gear_items
                .iter()
                .map(|entry| entry.parsed.observation.clone())
                .collect(),
        };
        if self.uid.is_none() {
            yas::log_warn!(
                "未能从任何画面左下角读出 UID，导出中的 UID 将为空。",
                "The bottom-left UID was unreadable on every screen tried; the export UID will be null."
            );
        }
        export_details.uid = self.uid;
        Ok(ScanResult {
            observations: ValidatedObservationSnapshot::from_screen_capture(snapshot)?,
            gear_items,
            coverage,
            export_details,
        })
    }

    /// UID is optional export metadata and never stops the scan. The 3D
    /// preview's light streaks sometimes cross the watermark and can turn one
    /// digit into another, so a UID is accepted only once two panels agree;
    /// `scan` warns once if none did.
    fn observe_uid(&mut self, frame: &RgbImage) {
        if self.uid.is_some() {
            return;
        }
        match self.parser.read_uid(frame) {
            Ok(Some(uid)) if self.uid_candidate == Some(uid) => self.uid = Some(uid),
            Ok(Some(uid)) => self.uid_candidate = Some(uid),
            Ok(None) => {},
            Err(error) => {
                if !self.uid_error_logged {
                    yas::log_warn!(
                        "读取 UID 失败，将在后续画面重试。完整错误详情：{}",
                        "Reading the UID failed; retrying on later screens. Full error details: {}",
                        error
                    );
                    self.uid_error_logged = true;
                } else {
                    yas::log_debug!("重读 UID 失败：{}", "UID retry failed: {}", error);
                }
            },
        }
    }

    fn scan_light_cones(&mut self) -> HsrResult<InventoryScan<ObservedLightCone>> {
        yas::log_info!("正在扫描光锥。", "Scanning Light Cones.");
        let scan =
            self.scan_inventory(InventoryKind::LightCone, |parser, frame, panel, refs, _| {
                parser
                    .parse_light_cone(frame, panel, refs)
                    .map(|parsed| parsed.observation)
            })?;
        yas::log_info!(
            "光锥扫描结束：{} 件；覆盖率={:?}。",
            "Light Cone scan finished: {} items; coverage={:?}.",
            scan.items.len(),
            scan.coverage
        );
        Ok(scan)
    }

    fn scan_gear(&mut self) -> HsrResult<InventoryScan<ScannedGearItem>> {
        yas::log_info!(
            "正在扫描遗器（包括隧洞遗器与位面饰品）。",
            "Scanning gear (Cavern Relics and Planar Ornaments)."
        );
        let scan = self.scan_inventory(
            InventoryKind::Gear,
            |parser, frame, panel, refs, ordinal| {
                let parsed = parser.parse_gear(frame, panel, refs)?;
                let semantic_fingerprint = gear_fingerprint(&parsed);
                Ok(ScannedGearItem {
                    ordinal,
                    parsed,
                    semantic_fingerprint,
                })
            },
        )?;
        yas::log_info!(
            "遗器扫描结束：{} 件；覆盖率={:?}。",
            "Gear scan finished: {} items; coverage={:?}.",
            scan.items.len(),
            scan.coverage
        );
        Ok(scan)
    }

    fn scan_inventory<T: std::fmt::Debug>(
        &mut self,
        kind: InventoryKind,
        mut parse: impl FnMut(
            &mut PanelParser<R>,
            &RgbImage,
            StatsPanelLayout,
            &ReferenceCache,
            usize,
        ) -> HsrResult<T>,
    ) -> HsrResult<InventoryScan<T>> {
        let mut items = Vec::new();
        let traversal = (|| {
            let mut session = self.enter_inventory(kind)?;
            let quantity = session.cursor.quantity();
            let reads_agree = session.quantity_reads[0] == session.quantity_reads[1];
            let category_cap = match kind {
                InventoryKind::LightCone => self.config.max_light_cones,
                InventoryKind::Gear => self.config.max_gear,
            };
            let limit = self
                .config
                .scan_item_limit
                .unwrap_or(quantity)
                .min(quantity)
                .min(if category_cap == 0 {
                    quantity
                } else {
                    category_cap
                });
            items.reserve(limit);
            let progress_category = match kind {
                InventoryKind::LightCone => crate::scan_progress::ScanCategory::LightCones,
                InventoryKind::Gear => crate::scan_progress::ScanCategory::Gear,
            };
            self.report(
                progress_category,
                crate::scan_progress::ScanEvent::Progress {
                    recognized: 0,
                    visited: 0,
                    total: Some(limit),
                },
            );
            let mut incomplete_reason = None;
            // The first slot is already selected. Later items use the inventory
            // next-item key. The client moves the highlight and scrolls the grid;
            // clicking later cells and paging the wheel is what stalled live scans.
            for ordinal in 0..limit {
                self.observe_uid(&session.frame);
                match dump_parsed_item(
                    inventory_dump_category(kind),
                    ordinal,
                    &session.frame,
                    || {
                        parse(
                            &mut self.parser,
                            &session.frame,
                            session.panel,
                            &self.references,
                            ordinal,
                        )
                    },
                ) {
                    Ok(item) => items.push(item),
                    Err(error)
                        if !self.config.stop_on_failure
                            && is_omittable_scan_error(error.code()) =>
                    {
                        incomplete_reason.get_or_insert_with(|| {
                            format!("one or more entries were omitted; firstCause={error}")
                        });
                        yas::log_warn!(
                        "一个库存条目无法可靠读出，已省略并继续下一项；覆盖率将标记为未知。完整错误详情：{}",
                        "An inventory entry could not be read reliably and was omitted; the walk continues and coverage will be marked unknown. Full error details: {}",
                        error
                    );
                    },
                    Err(error) => {
                        yas::log_error!(
                            "库存第 {} 项识别失败。完整错误详情：{}",
                            "Inventory entry {} failed. Full error details: {}",
                            ordinal + 1,
                            error
                        );
                        return Err(error);
                    },
                }
                self.report(
                    progress_category,
                    crate::scan_progress::ScanEvent::Progress {
                        recognized: items.len(),
                        visited: ordinal + 1,
                        total: Some(limit),
                    },
                );
                if ordinal % 25 == 0 || ordinal + 1 == limit {
                    yas::log_info!(
                        "库存进度：{}/{}。",
                        "Inventory progress: {}/{}.",
                        ordinal + 1,
                        quantity
                    );
                }
                if ordinal + 1 == limit {
                    break;
                }
                match self.advance_inventory_with_next_key(
                    &session.panel,
                    &session.grid,
                    &session.frame,
                    &mut session.walk,
                ) {
                    Ok(frame) => session.frame = frame,
                    Err(error) if error.code() == "HSR-SCAN-NAV" => {
                        incomplete_reason.get_or_insert_with(|| error.to_string());
                        break;
                    },
                    Err(error) => return Err(error),
                }
            }
            if limit < quantity {
                incomplete_reason.get_or_insert_with(|| {
                    format!("scan cap={limit} reached before quantity={quantity}")
                });
            }

            let coverage = if let Some(reason) = incomplete_reason {
                log_inventory_coverage_warning(kind, items.len(), &reason);
                CoverageLevel::Unknown
            } else if !reads_agree || items.len() != quantity {
                let reason = format!(
                "next-item walk parsed {} entries; quantity reads were {:?} and agreement={reads_agree}",
                items.len(),
                session.quantity_reads
            );
                log_inventory_coverage_warning(kind, items.len(), &reason);
                CoverageLevel::Unknown
            } else {
                CoverageLevel::Complete
            };
            self.leave_menu()?;
            Ok(coverage)
        })();
        let coverage = self.category_coverage(traversal)?;
        Ok(InventoryScan { items, coverage })
    }

    fn category_coverage(&self, result: HsrResult<CoverageLevel>) -> HsrResult<CoverageLevel> {
        match result {
            Err(error)
                if self.config.save_on_cancel
                    && self.device.is_cancelled()
                    && error.hint() == hints::CANCELLED =>
            {
                Ok(CoverageLevel::Unknown)
            },
            other => other,
        }
    }

    /// Reopen the inventory and select one scan ordinal, then reparse the
    /// panel immediately before a state-changing click. Callers must compare
    /// the returned visible identity with a prior complete-inventory reread.
    fn select_gear_ordinal(&mut self, ordinal: usize) -> HsrResult<SelectedGearContext> {
        let mut session = self.enter_inventory(InventoryKind::Gear)?;
        if ordinal >= session.cursor.quantity() {
            return Err(HsrError::new(
                "HSR-MANAGER-ORDINAL-DRIFT",
                hints::SCREEN_INVALID,
                format!(
                    "selected ordinal={ordinal} is outside fresh quantity={}",
                    session.cursor.quantity()
                ),
            ));
        }
        for _ in 0..ordinal {
            session.frame = self.advance_inventory_with_next_key(
                &session.panel,
                &session.grid,
                &session.frame,
                &mut session.walk,
            )?;
        }
        let parsed = self
            .parser
            .parse_gear(&session.frame, session.panel, &self.references)?;
        let selection = selected_card(&session.grid, &session.frame).ok_or_else(|| {
            HsrError::new(
                "HSR-MANAGER-SELECTION-DRIFT",
                hints::SCREEN_INVALID,
                format!("next-item walk reached ordinal={ordinal} without a visible selected card"),
            )
        })?;
        Ok(SelectedGearContext {
            parsed,
            panel: session.panel,
            grid: session.grid,
            selection,
            parsed_frame: session.frame,
        })
    }

    fn enter_inventory(&mut self, kind: InventoryKind) -> HsrResult<InventorySession> {
        // Escape in the overworld opens the pause menu, and `1` there switches
        // the active party member (its animation swallows the next key). So
        // `1`, which leaves controller UI where pointer clicks do not change
        // tabs, is pressed only once the backpack is open.
        self.open_menu('b', INVENTORY_TITLE)?;
        self.issue_input(InputCommand::Key('1'))?;
        self.wait_attended(Duration::from_millis(self.config.timings.input_settle_ms))?;

        let tab = match kind {
            InventoryKind::LightCone => layout::LIGHT_CONE_TAB,
            InventoryKind::Gear => layout::GEAR_TAB,
        };
        let mut last_error = None;
        for offset in [0.0, -0.012, 0.012, -0.024, 0.024] {
            self.issue_input(InputCommand::Click(crate::vision::Point::new(
                tab.x + offset,
                tab.y,
            )))?;
            self.wait_attended(Duration::from_millis(self.config.timings.inventory_tab_ms))?;
            let before = self.capture_stable()?;
            if let Err(error) = discover_inventory_grid(&before) {
                last_error = Some(error);
                continue;
            }
            let grid = backpack_grid(kind);
            let selected = match self.select_first_inventory_cell(&grid) {
                Ok(frame) => frame,
                Err(error) if error.code() == "HSR-SCAN-FIRST-CELL" => {
                    last_error = Some(error);
                    continue;
                },
                Err(error) => return Err(error),
            };
            match self
                .parser
                .discover_inventory_panel(&selected, kind, &self.references)
            {
                Ok(panel) => {
                    dump_inventory_setup(kind, &selected);
                    let primary_quantity = match self.read_inventory_quantity(&selected) {
                        Ok(quantity) => quantity,
                        Err(error) => {
                            annotator::finalize_error(None, &error.to_string());
                            return Err(error);
                        },
                    };
                    if let Err(error) =
                        self.validate_inventory_quantity(primary_quantity, "primary")
                    {
                        annotator::finalize_error(None, &error.to_string());
                        return Err(error);
                    }
                    self.wait_attended(Duration::from_millis(
                        self.config.timings.capture_interval_ms,
                    ))?;
                    let confirmation_frame = self.device.capture_client()?;
                    annotator::add_image("confirmation", &confirmation_frame);
                    if !frames_similar(
                        &panel.immutable_panel().crop(&selected)?,
                        &panel.immutable_panel().crop(&confirmation_frame)?,
                    ) {
                        let error = HsrError::new(
                            "HSR-SCAN-QUANTITY-FRAME",
                            hints::SCREEN_INVALID,
                            "independent quantity confirmation frame did not preserve the selected first card and immutable detail panel",
                        );
                        annotator::finalize_error(None, &error.to_string());
                        return Err(error);
                    }
                    let confirmation_quantity =
                        match self.read_inventory_quantity(&confirmation_frame) {
                            Ok(quantity) => quantity,
                            Err(error) => {
                                annotator::finalize_error(None, &error.to_string());
                                return Err(error);
                            },
                        };
                    if let Err(error) =
                        self.validate_inventory_quantity(confirmation_quantity, "confirmation")
                    {
                        annotator::finalize_error(None, &error.to_string());
                        return Err(error);
                    }
                    // Use the larger plausible reading for traversal so a
                    // disagreement cannot silently truncate the inventory.
                    // Coverage remains unknown until all reads agree and the
                    // terminal probes independently prove the bottom.
                    let quantity = primary_quantity.max(confirmation_quantity);
                    let cursor = InventoryCursor::new(quantity, &grid)?;
                    annotator::finalize_success(&format!(
                        "quantity_reads=[{primary_quantity}, {confirmation_quantity}]"
                    ));
                    return Ok(InventorySession {
                        cursor,
                        grid,
                        panel,
                        frame: confirmation_frame,
                        quantity_reads: [primary_quantity, confirmation_quantity],
                        walk: InventoryWalk::default(),
                    });
                },
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            HsrError::new(
                "HSR-SCAN-TAB",
                hints::SCREEN_INVALID,
                "could not validate the requested inventory tab",
            )
        }))
    }

    /// Click the top-left card and confirm it is the selected one, so the
    /// walk cannot silently start a row late.
    fn select_first_inventory_cell(&mut self, grid: &GridGeometry) -> HsrResult<RgbImage> {
        let first = grid.first().expect("backpack grid has cells");
        let mut observed = None;
        for _ in 0..2 {
            self.issue_input(InputCommand::Click(first))?;
            self.issue_input(InputCommand::Hover(layout::INVENTORY_POINTER_REST))?;
            self.wait_attended(Duration::from_millis(self.config.timings.panel_switch_ms))?;
            let frame = self.capture_stable()?;
            observed = selected_card(grid, &frame);
            if observed.is_some_and(|card| {
                card.column == 0 && (card.y - first.y).abs() < SELECTION_ROW_SHIFT
            }) {
                return Ok(frame);
            }
        }
        Err(HsrError::new(
            "HSR-SCAN-FIRST-CELL",
            hints::SCREEN_INVALID,
            format!("clicking the first backpack card left selection={observed:?}"),
        ))
    }

    fn read_inventory_quantity(&mut self, frame: &RgbImage) -> HsrResult<usize> {
        let crop = layout::QUANTITY.crop(frame)?;
        let text = self
            .parser_reader_mut()
            .read(OcrField::InventoryQuantity, &crop)?;
        annotator::record_ocr(
            OcrField::InventoryQuantity.dump_name().as_str(),
            layout::QUANTITY,
            &text,
        );
        let digits: String = text.chars().filter(char::is_ascii_digit).collect();
        inventory_quantity_from_digits(&digits).map_err(|error| {
            HsrError::new(
                "HSR-SCAN-QUANTITY",
                hints::OCR_FAILED,
                format!("inventory quantity OCR did not yield an integer; cause={error}"),
            )
        })
    }

    fn validate_inventory_quantity(&self, quantity: usize, read: &str) -> HsrResult<()> {
        if !(1..=4_000).contains(&quantity) {
            return Err(HsrError::new(
                "HSR-SCAN-QUANTITY",
                hints::OCR_FAILED,
                format!("{read} inventory quantity={quantity}; OCR plausible range=1..=4000"),
            ));
        }
        Ok(())
    }

    fn parser_reader_mut(&mut self) -> &mut R {
        self.parser.reader_mut()
    }

    /// Press the walk's next-item key and wait until the detail panel has
    /// settled on the new item. The selection border moves before the text
    /// does, so a grid-only change is not yet safe to read.
    fn advance_inventory_with_next_key(
        &mut self,
        panel: &StatsPanelLayout,
        grid: &GridGeometry,
        previous: &RgbImage,
        walk: &mut InventoryWalk,
    ) -> HsrResult<RgbImage> {
        let seen = selected_card(grid, previous);
        let selection = if seen.is_some_and(|card| card.column == walk.column) {
            seen
        } else {
            self.settled_selection(grid)?
        };
        let key = walk.next_key(selection.map(|card| card.column), grid.columns());
        let moved = self.press_inventory_key(key, panel, grid, previous, selection)?;
        walk.stepped(key);
        Ok(moved)
    }

    /// The selection frame cross-fades between cards right after a step, so
    /// a frame taken as soon as the panel changed can still show the old
    /// card. When that disagrees with the dead-reckoned column, wait until two
    /// consecutive fresh frames agree.
    fn settled_selection(&mut self, grid: &GridGeometry) -> HsrResult<Option<SelectedCard>> {
        let mut last = None;
        for _ in 0..6 {
            self.wait_attended(Duration::from_millis(
                self.config.timings.capture_interval_ms,
            ))?;
            let frame = self.device.capture_client()?;
            let card = selected_card(grid, &frame);
            if card.is_some() && card.map(|c| c.column) == last.map(|c: SelectedCard| c.column) {
                return Ok(card);
            }
            last = card;
        }
        Ok(None)
    }

    fn press_inventory_key(
        &mut self,
        key: char,
        panel: &StatsPanelLayout,
        grid: &GridGeometry,
        previous: &RgbImage,
        previous_selection: Option<SelectedCard>,
    ) -> HsrResult<RgbImage> {
        let previous_panel = frame_fingerprint(previous, panel.immutable_panel())?;
        let previous_grid = frame_fingerprint(previous, INVENTORY_GRID_REGION)?;
        for attempt in 0..3 {
            self.issue_input(InputCommand::Key(key))?;
            self.wait_attended(Duration::from_millis(self.config.timings.key_settle_ms))?;
            let deadline_steps = (u128::from(self.config.timings.panel_timeout_ms)
                / u128::from(self.config.timings.poll_interval_ms.max(1)))
            .max(2) as usize;
            let mut grid_moved_at: Option<Instant> = None;
            let mut covered = false;
            for _ in 0..deadline_steps {
                self.ensure_attended()?;
                let frame = self.device.capture_client()?;
                let panel_now = frame_fingerprint(&frame, panel.immutable_panel())?;
                let panel_changed = panel_now != previous_panel;
                // Icon idle animation changes grid pixels without moving the
                // highlight. Only a new selected cell counts as the next copy.
                if !panel_changed
                    && grid_moved_at.is_none()
                    && frame_fingerprint(&frame, INVENTORY_GRID_REGION)? != previous_grid
                {
                    let selection = selected_card(grid, &frame);
                    if matches!((previous_selection, selection), (Some(before), Some(after)) if before.column != after.column || (before.y - after.y).abs() > SELECTION_ROW_SHIFT)
                    {
                        grid_moved_at = Some(Instant::now());
                    }
                }
                if panel_changed
                    || grid_moved_at.is_some_and(|at| {
                        at.elapsed()
                            >= Duration::from_millis(self.config.timings.selection_settle_ms)
                    })
                {
                    if panel_changed {
                        self.wait_attended(Duration::from_millis(
                            self.config.timings.capture_interval_ms,
                        ))?;
                    }
                    let settled = self.device.capture_client()?;
                    // A modal replaces the backpack with an animating backdrop.
                    // That motion is not an inventory step.
                    if discover_inventory_grid(&settled).is_ok() {
                        return Ok(settled);
                    }
                    covered = true;
                    break;
                }
                self.wait_attended(Duration::from_millis(
                    self.config.timings.poll_interval_ms.max(1),
                ))?;
            }
            if !covered && grid_moved_at.is_some() {
                let frame = self.device.capture_client()?;
                if discover_inventory_grid(&frame).is_ok() {
                    return Ok(frame);
                }
                covered = true;
            }
            if covered {
                yas::log_warn!(
                    "库存网格被弹窗挡住，正在关闭弹窗后继续。",
                    "Inventory grid is covered by a dialog; dismissing it and continuing."
                );
                self.issue_input(InputCommand::Escape)?;
                self.wait_attended(Duration::from_millis(self.config.timings.menu_close_ms))?;
                let restored = self.device.capture_client()?;
                if discover_inventory_grid(&restored).is_ok()
                    && frame_fingerprint(&restored, panel.immutable_panel())? != previous_panel
                {
                    return Ok(restored);
                }
            }
            if attempt < 2 {
                yas::log_warn!(
                    "下一项按键后库存画面没有变化，正在再按一次。",
                    "Inventory view did not change after the next-item key; pressing it once more."
                );
            }
        }
        Err(HsrError::new(
            "HSR-SCAN-NAV",
            hints::SCREEN_INVALID,
            "next-item key did not move the inventory selection or the detail panel",
        ))
    }

    #[cfg(test)]
    fn advance_inventory_session(
        &mut self,
        session: &mut InventorySession,
    ) -> HsrResult<Option<usize>> {
        loop {
            match session.cursor.next_step()? {
                None => return Ok(None),
                Some(InventoryStep::Scroll { rows }) => {
                    let after = self.scroll_inventory_rows(
                        &session.frame,
                        &session.grid,
                        session.panel,
                        rows,
                    )?;
                    session.cursor.mark_scrolled(rows)?;
                    session.frame = after;
                },
                Some(InventoryStep::Select {
                    ordinal,
                    cell_index,
                }) => {
                    if !(ordinal == 0
                        && selected_cell(&session.grid, &session.frame).map(|entry| entry.0)
                            == Some(0))
                    {
                        session.frame = self.select_inventory_cell(
                            &session.grid,
                            session.panel,
                            cell_index,
                            ordinal,
                        )?;
                    }
                    session.cursor.mark_selected(ordinal)?;
                    return Ok(Some(ordinal));
                },
            }
        }
    }

    #[cfg(test)]
    fn select_inventory_cell(
        &mut self,
        grid: &GridGeometry,
        _panel: StatsPanelLayout,
        cell_index: usize,
        ordinal: usize,
    ) -> HsrResult<RgbImage> {
        let point = grid.cell(cell_index).ok_or_else(|| {
            HsrError::new(
                "HSR-GRID-CELL",
                hints::SCREEN_INVALID,
                format!(
                    "row-major cell index={cell_index} is outside capacity={} for ordinal={ordinal}",
                    grid.capacity()
                ),
            )
        })?;
        self.issue_input(InputCommand::Click(point))?;
        self.wait_attended(Duration::from_millis(self.config.timings.key_settle_ms))?;

        let deadline_steps = (u128::from(self.config.timings.panel_timeout_ms)
            / u128::from(self.config.timings.poll_interval_ms.max(1)))
        .max(2) as usize;
        let mut selected_seen = false;
        for _ in 0..deadline_steps {
            self.ensure_attended()?;
            let frame = self.device.capture_client()?;
            if selected_cell(grid, &frame).map(|entry| entry.0) == Some(cell_index) {
                selected_seen = true;
                // Glow and the selection ring flicker, so identical panel
                // crops never arrive. A hit that is still the same cell
                // after a short settle is enough.
                self.wait_attended(Duration::from_millis(
                    self.config.timings.capture_interval_ms,
                ))?;
                let settled = self.device.capture_client()?;
                if selected_cell(grid, &settled).map(|entry| entry.0) == Some(cell_index) {
                    return Ok(settled);
                }
            }
            self.wait_attended(Duration::from_millis(
                self.config.timings.poll_interval_ms.max(1),
            ))?;
        }
        Err(HsrError::new(
            "HSR-SCAN-SELECTION",
            hints::SCREEN_INVALID,
            format!(
                "cell click was not postvalidated; ordinal={ordinal}, cell={cell_index}, selectedSeen={selected_seen}"
            ),
        ))
    }

    #[cfg(test)]
    fn scroll_inventory_rows(
        &mut self,
        before: &RgbImage,
        grid: &GridGeometry,
        _panel: StatsPanelLayout,
        rows: usize,
    ) -> HsrResult<RgbImage> {
        let after = self.issue_inventory_scroll(grid, rows)?;
        match classify_inventory_scroll(before, &after, grid, rows)? {
            ScrollEvidence::Advanced {
                rows: actual_rows, ..
            } if actual_rows == rows => Ok(after),
            ScrollEvidence::Advanced {
                rows: actual_rows,
                confidence,
            } => Err(HsrError::new(
                "HSR-SCAN-SCROLL-DISTANCE",
                hints::SCREEN_INVALID,
                format!(
                    "scroll advanced an unexpected number of rows; requested={rows}, actual={actual_rows}, confidence={confidence:.3}"
                ),
            )),
            ScrollEvidence::End { confidence } => Err(HsrError::new(
                "HSR-SCAN-EARLY-END",
                hints::SCREEN_INVALID,
                format!(
                    "inventory reached a visually unchanged end before the OCR quantity bound; requestedRows={rows}, confidence={confidence:.3}"
                ),
            )),
            ScrollEvidence::Uncertain { confidence } => Err(HsrError::new(
                "HSR-SCAN-SCROLL-UNCERTAIN",
                hints::SCREEN_INVALID,
                format!(
                    "scroll pixels changed without proving row-aligned advance; requestedRows={rows}, confidence={confidence:.3}"
                ),
            )),
        }
    }

    #[cfg(test)]
    fn issue_inventory_scroll(&mut self, grid: &GridGeometry, rows: usize) -> HsrResult<RgbImage> {
        let ticks = self.inventory_scroll_ticks(rows, grid.rows())?;
        for _ in 0..ticks {
            self.issue_input(InputCommand::Scroll(1))?;
            self.wait_attended(self.config.scroll_tick_delay)?;
        }
        self.capture_stable()
    }

    #[cfg(test)]
    fn inventory_scroll_ticks(&self, rows: usize, visible_rows: usize) -> HsrResult<usize> {
        let base_ticks = self.config.inventory_scroll_ticks_per_page;
        if rows == 0 || visible_rows == 0 || base_ticks == 0 {
            return Err(HsrError::new(
                "HSR-SCAN-SCROLL-CONFIG",
                hints::SCREEN_INVALID,
                format!(
                    "invalid scroll configuration; requestedRows={rows}, visibleRows={visible_rows}, ticksPerPage={base_ticks}"
                ),
            ));
        }
        Ok((base_ticks * rows).div_ceil(visible_rows).max(1))
    }

    #[cfg(test)]
    fn confirm_inventory_completion(
        &mut self,
        session: &mut InventorySession,
    ) -> HsrResult<InventoryCompletionAssessment> {
        let next_cell = self.probe_next_inventory_cell(session)?;
        let before_scroll = session.frame.clone();
        let after_first_scroll = self.issue_inventory_scroll(&session.grid, 1)?;
        let first_bottom =
            classify_inventory_scroll(&before_scroll, &after_first_scroll, &session.grid, 1)?;
        let first_scrollbar =
            inventory_scrollbar_bottom_confidence(&after_first_scroll, &session.grid);

        // A second attended wheel event is independent input evidence. Both
        // resulting frames must remain at the same proven scrollbar bottom;
        // one ignored or misdirected wheel event is never sufficient.
        let after_second_scroll = self.issue_inventory_scroll(&session.grid, 1)?;
        let second_bottom =
            classify_inventory_scroll(&after_first_scroll, &after_second_scroll, &session.grid, 1)?;
        let second_scrollbar =
            inventory_scrollbar_bottom_confidence(&after_second_scroll, &session.grid);
        let scrollbar_bottom_confidence = first_scrollbar
            .zip(second_scrollbar)
            .map(|(first, second)| first.min(second));

        let terminal_quantity = match self.read_inventory_quantity(&after_second_scroll) {
            Ok(quantity) => match self.validate_inventory_quantity(quantity, "terminal") {
                Ok(()) => Some(quantity),
                Err(error) => {
                    yas::log_warn!(
                        "库存末端数量超出可信范围；覆盖率将标记为未知，管理器不会执行变更。完整错误详情：{}",
                        "The terminal inventory quantity was outside the trusted range; coverage will be marked unknown and the manager will not mutate. Full error details: {}",
                        error
                    );
                    if self.config.stop_on_failure {
                        return Err(error);
                    }
                    None
                },
            },
            Err(error) => {
                yas::log_warn!(
                    "无法在库存末端再次读取数量；覆盖率将标记为未知，管理器不会执行变更。完整错误详情：{}",
                    "The inventory quantity could not be read again at the terminal boundary; coverage will be marked unknown and the manager will not mutate. Full error details: {}",
                    error
                );
                if self.config.stop_on_failure {
                    return Err(error);
                }
                None
            },
        };
        session.frame = after_second_scroll;
        Ok(assess_inventory_completion(InventoryCompletionEvidence {
            primary_quantity: session.quantity_reads[0],
            confirmation_quantity: session.quantity_reads[1],
            terminal_quantity,
            next_cell,
            terminal_scrolls: [first_bottom, second_bottom],
            scrollbar_bottom_confidence,
        }))
    }

    #[cfg(test)]
    fn probe_next_inventory_cell(
        &mut self,
        session: &mut InventorySession,
    ) -> HsrResult<NextCellEvidence> {
        let Some(last_cell) = session.cursor.last_selected_cell() else {
            return Ok(NextCellEvidence::Uncertain);
        };
        let next_cell = last_cell + 1;
        if next_cell >= session.grid.capacity() {
            return Ok(NextCellEvidence::NoCandidate);
        }
        let point = session.grid.cell(next_cell).ok_or_else(|| {
            HsrError::new(
                "HSR-GRID-CELL",
                hints::SCREEN_INVALID,
                format!("terminal probe cell={next_cell} is outside the derived grid"),
            )
        })?;
        self.issue_input(InputCommand::Click(point))?;
        self.wait_attended(Duration::from_millis(self.config.timings.key_settle_ms))?;
        let after = self.capture_stable()?;
        let selected = selected_cell(&session.grid, &after).map(|entry| entry.0);
        session.frame = after;
        Ok(match selected {
            Some(index) if index == next_cell => NextCellEvidence::ExtraItem,
            Some(index) if index == last_cell => NextCellEvidence::StayedOnLast,
            _ => NextCellEvidence::Uncertain,
        })
    }

    fn scan_characters(&mut self) -> HsrResult<CharacterScan> {
        yas::log_info!("正在扫描角色详情。", "Scanning Character details.");
        let mut items: Vec<ObservedCharacter> = Vec::new();
        let mut export_details = CaptureExportDetails {
            trailblazer: self
                .config
                .trailblazer
                .as_ref()
                .map(|identity| identity.gender.name().to_string()),
            ..CaptureExportDetails::default()
        };
        let traversal = (|| {
            self.open_menu('c', CHARACTER_TITLE)?;
            // Leave controller UI once, then use only mouse traversal.
            self.issue_input(InputCommand::Key('1'))?;
            self.wait_attended(Duration::from_millis(self.config.timings.input_settle_ms))?;
            self.issue_input(InputCommand::Click(layout::DETAILS_BUTTON))?;
            self.wait_attended(Duration::from_millis(self.config.timings.panel_switch_ms))?;
            self.reset_character_bar()?;
            self.issue_input(InputCommand::Click(layout::character_portrait(0)))?;
            self.wait_attended(Duration::from_millis(
                self.config.timings.character_switch_ms,
            ))?;
            let mut slot = 0;

            let limit = self.config.max_characters;
            let mut seen = BTreeSet::new();
            let mut terminal_proven = false;
            let mut coverage_degraded = false;
            // Bound a malfunctioning traversal by the reference roster, allowing
            // one overlapping final page. Completion comes from the controller's
            // terminal evidence, never from this bound or an entered total.
            let visit_bound = self.references.character_count() * 2 + layout::CHARACTER_PAGE_SIZE;
            for index in 0..visit_bound {
                let pages = self.read_character_pages(index, index % 2 == 1)?;
                self.observe_uid(&pages.details);
                let parsed = match pages.parsed {
                    Ok(parsed) => Some(parsed),
                    Err(error)
                        if !self.config.stop_on_failure
                            && error.code() == "HSR-OCR-CHARACTER-AMBIGUOUS" =>
                    {
                        coverage_degraded = true;
                        yas::log_warn!(
                        "当前角色名称对应多个公开角色模板，且画面没有足够的命途或变体证据；已省略该角色并将角色覆盖率标记为未知。完整错误详情：{}",
                        "The current character name maps to multiple public templates and the screen lacks sufficient path or variant evidence; the entry was omitted and character coverage is marked unknown. Full error details: {}",
                        error
                    );
                        None
                    },
                    Err(error)
                        if !self.config.stop_on_failure
                            && is_omittable_scan_error(error.code()) =>
                    {
                        coverage_degraded = true;
                        yas::log_warn!(
                        "角色面板有字段无法识别，已省略该角色并将角色覆盖率标记为未知。完整错误详情：{}",
                        "A field on the Character panel was unreadable; the entry was omitted and character coverage is marked unknown. Full error details: {}",
                        error
                    );
                        None
                    },
                    Err(error) => {
                        yas::log_error!(
                            "角色第 {} 项识别失败。完整错误详情：{}",
                            "Character entry {} failed. Full error details: {}",
                            index + 1,
                            error
                        );
                        return Err(error);
                    },
                };
                if let Some(parsed) = parsed {
                    // A final drag clamps to the end and overlaps the previous
                    // page. Revisited identities are expected and are not exported
                    // twice; continue clicking through the rest of that page.
                    if seen.insert(parsed.observation.character_id) {
                        if let Some(trace_details) = self.accept_character_traces(pages.traces)? {
                            export_details
                                .characters
                                .insert(parsed.observation.character_id, trace_details);
                        }
                        if trailblazer_gender(parsed.observation.character_id).is_some() {
                            export_details.current_trailblazer_path =
                                Some(path_name(&parsed.reference.path)?.to_string());
                        }
                        items.push(parsed.observation);
                        self.report(
                            crate::scan_progress::ScanCategory::Characters,
                            crate::scan_progress::ScanEvent::Progress {
                                recognized: items.len(),
                                visited: items.len(),
                                total: None,
                            },
                        );
                        yas::log_info!("角色进度：{}。", "Character progress: {}.", items.len());
                    }
                }
                if limit > 0 && items.len() >= limit {
                    break;
                }
                // Keep the final tab selected; the next character walks back
                // through the same tabs without a redundant return to Details.
                match self.advance_character(&pages.last, &mut slot, index) {
                    Ok(_) => {},
                    Err(error) if error.code() == "HSR-CHAR-END" => {
                        terminal_proven = true;
                        break;
                    },
                    Err(error) if error.code() == "HSR-CHAR-ADVANCE" => {
                        coverage_degraded = true;
                        yas::log_warn!(
                        "点击后没有切换到下一个角色，角色列表在此停止。完整错误详情：{}",
                        "Clicking did not open another Character; roster traversal stopped here. Full error details: {}",
                        error
                    );
                        break;
                    },
                    Err(error) => return Err(error),
                };
            }
            self.leave_menu()?;
            let coverage = if terminal_proven && !coverage_degraded {
                CoverageLevel::Complete
            } else {
                CoverageLevel::Unknown
            };
            Ok(coverage)
        })();
        let coverage = self.category_coverage(traversal)?;
        yas::log_info!(
            "角色扫描完成：{} 名。",
            "Character scan complete: {} entries.",
            items.len()
        );
        Ok(CharacterScan {
            items,
            details: export_details,
            coverage,
        })
    }

    fn reset_character_bar(&mut self) -> HsrResult<()> {
        self.issue_input(InputCommand::Hover(Point::new(0.85, 0.15)))?;
        let mut previous = self.capture_stable()?;
        for _ in 0..self
            .references
            .character_count()
            .div_ceil(layout::CHARACTER_PAGE_SIZE)
            + 2
        {
            let next = drag_character_page(&mut self.device, false, &self.config.timings)?;
            if character_bar_unchanged(&previous, &next)? {
                let confirmation =
                    drag_character_page(&mut self.device, false, &self.config.timings)?;
                if character_bar_unchanged(&next, &confirmation)? {
                    return Ok(());
                }
                previous = confirmation;
                continue;
            }
            previous = next;
        }
        Err(HsrError::new(
            "HSR-CHAR-START",
            hints::SCREEN_INVALID,
            "portrait bar did not reach a stationary first page",
        ))
    }

    fn advance_character(
        &mut self,
        previous: &RgbImage,
        slot: &mut usize,
        ordinal: usize,
    ) -> HsrResult<RgbImage> {
        let previous_identity = CHARACTER_IDENTITY_REGION.crop(previous)?;
        let paged = *slot + 1 == layout::CHARACTER_PAGE_SIZE
            || crate::vision::last_visible_character_selected(previous) == Some(true);
        let next_slot = if paged {
            let after = drag_character_page(&mut self.device, true, &self.config.timings)?;
            if character_bar_unchanged(previous, &after)? {
                // Verify with a second independent drag: a missed input must
                // not by itself claim that every Character was scanned.
                let confirmation =
                    drag_character_page(&mut self.device, true, &self.config.timings)?;
                if !character_bar_unchanged(&after, &confirmation)? {
                    return Err(HsrError::new(
                        "HSR-CHAR-ADVANCE",
                        hints::SCREEN_INVALID,
                        "terminal drag confirmation moved the roster; traversal is incomplete",
                    ));
                }
                return Err(HsrError::new(
                    "HSR-CHAR-END",
                    hints::SCREEN_INVALID,
                    "last visible portrait visited; two forward drags stayed at the roster end",
                ));
            }
            0
        } else {
            *slot + 1
        };
        // Retry the same absolute portrait position. A delayed click plus a
        // retry cannot skip another character, and a page is dragged only once.
        for attempt in 1..=3 {
            self.issue_input(InputCommand::Click(layout::character_portrait(next_slot)))?;
            match self.wait_until_identity_changes(
                &previous_identity,
                ordinal,
                Duration::from_millis(self.config.timings.panel_timeout_ms),
            ) {
                Ok(frame) => {
                    *slot = next_slot;
                    return Ok(frame);
                },
                Err(error) if error.code() == "HSR-CHAR-ADVANCE" && paged => {
                    // A clamped page can start with the character that was
                    // selected at the previous page's end. The bar did move;
                    // read this overlap once and continue to the second slot.
                    let frame = self.capture_stable()?;
                    *slot = next_slot;
                    return Ok(frame);
                },
                Err(error) if error.code() == "HSR-CHAR-ADVANCE" && attempt < 3 => {},
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    fn wait_until_identity_changes(
        &mut self,
        previous_identity: &RgbImage,
        ordinal: usize,
        timeout: Duration,
    ) -> HsrResult<RgbImage> {
        self.wait_attended(Duration::from_millis(
            self.config.timings.character_switch_ms,
        ))?;
        let deadline_steps = (timeout.as_millis()
            / u128::from(self.config.timings.poll_interval_ms.max(1)))
        .max(2) as usize;
        let mut changed_frame: Option<RgbImage> = None;
        let mut changed_identity = None;
        for _ in 0..deadline_steps {
            self.ensure_attended()?;
            let frame = self.device.capture_client()?;
            let identity = CHARACTER_IDENTITY_REGION.crop(&frame)?;
            if !frames_similar(previous_identity, &identity) {
                if changed_identity
                    .as_ref()
                    .is_some_and(|prior| frames_similar(prior, &identity))
                {
                    return Ok(frame);
                }
                changed_frame = Some(frame);
                changed_identity = Some(identity);
            }
            self.wait_attended(Duration::from_millis(
                self.config.timings.poll_interval_ms.max(1),
            ))?;
        }
        let detail = if changed_frame.is_some() {
            "identity region changed but did not stabilize"
        } else {
            "identity region never changed after the portrait click"
        };
        Err(HsrError::new(
            "HSR-CHAR-ADVANCE",
            hints::SCREEN_INVALID,
            format!("failed after character ordinal={ordinal}; {detail}"),
        ))
    }

    /// Only the OCR parser crosses threads. The device (including its Win32
    /// capture resources) stays on the caller thread. A three-frame channel
    /// bounds memory and lets forward OCR overlap both subsequent tab waits.
    fn read_character_pages(&mut self, index: usize, reverse: bool) -> HsrResult<CharacterPages> {
        let parser = &mut self.parser;
        let device = &mut self.device;
        let references = &self.references;
        let trailblazer = self.config.trailblazer.as_ref();
        let timings = &self.config.timings;
        std::thread::scope(|scope| {
            let (sender, receiver) = std::sync::mpsc::sync_channel::<(CharacterTab, RgbImage)>(3);
            let worker = scope.spawn(move || {
                let mut parsed = None;
                let mut identity = None;
                let mut traces_frame = None;
                let mut traces = None;
                let mut eidolon = 0;
                for (tab, frame) in receiver {
                    // The common header is present on every tab. Resolve it on
                    // the first screenshot so reverse-order Traces OCR can run
                    // during the switch to Details, without reading the name twice.
                    if identity.is_none() {
                        identity = Some(dump_parsed_item(
                            "character_identity",
                            index,
                            &frame,
                            || parser.parse_character_identity(&frame, references, trailblazer),
                        ));
                    }
                    match tab {
                        CharacterTab::Details => {
                            parsed = Some(match identity.as_ref().unwrap() {
                                Ok(reference) => {
                                    dump_parsed_item("characters", index, &frame, || {
                                        parser.parse_character_level(&frame, reference.clone(), 0)
                                    })
                                },
                                Err(error) => Err(error.clone()),
                            });
                        },
                        CharacterTab::Traces => traces_frame = Some(frame),
                        CharacterTab::Eidolons => {
                            annotator::begin_item("character_eidolons", index, &frame);
                            annotator::add_image("eidolons", &frame);
                            eidolon = layout::EIDOLON_NODES
                                .iter()
                                .take_while(|&&node| eidolon_node_unlocked(&frame, node))
                                .count() as u8;
                            annotator::finalize_success(&format!("eidolon={eidolon}"));
                        },
                    }
                    // Header OCR identifies the path before parsing Traces in either
                    // direction; retain the frame until that reference is available.
                    if let (Some(Ok(reference)), Some(frame)) = (&identity, &traces_frame) {
                        traces = Some(dump_parsed_item("character_traces", index, frame, || {
                            parser.parse_character_traces(frame, &reference.path)
                        }));
                        traces_frame = None;
                    }
                }
                if let Some(Ok(character)) = &mut parsed {
                    character.observation.eidolon = eidolon;
                }
                (parsed, traces)
            });
            let capture = (|| {
                let mut details = None;
                let mut last = None;
                let mut identity = None;
                for (position, tab) in CharacterTab::order(reverse).into_iter().enumerate() {
                    // The previous character already left us on the first tab.
                    if position > 0 {
                        ensure_device_attended(device)?;
                        device.input(InputCommand::Click(tab.button()))?;
                        ensure_device_attended(device)?;
                        let delay = if tab == CharacterTab::Traces {
                            timings.traces_open_ms
                        } else {
                            timings.panel_switch_ms
                        };
                        device.wait(Duration::from_millis(delay))?;
                    }
                    ensure_device_attended(device)?;
                    let frame = device.capture_client()?;
                    ensure_device_attended(device)?;
                    let current_identity = CHARACTER_IDENTITY_REGION.crop(&frame)?;
                    if identity
                        .as_ref()
                        .is_some_and(|previous| !glyphs_match(previous, &current_identity))
                    {
                        return Err(HsrError::new("HSR-CHAR-PANEL-IDENTITY", hints::SCREEN_INVALID,
                            "character identity changed between tabs; refusing to combine different characters"));
                    }
                    identity = Some(current_identity);
                    if tab == CharacterTab::Details {
                        details = Some(frame.clone());
                    }
                    last = Some(frame.clone());
                    sender
                        .send((tab, frame))
                        .expect("OCR worker remains alive until capture finishes");
                }
                Ok((details.unwrap(), last.unwrap()))
            })();
            drop(sender);
            // Join even when navigation fails, before exposing a cancellation or
            // focus error. Partial frames never become a completed character.
            let result = worker.join();
            let (details, last) = capture?;
            let (parsed, traces) = result.expect("character OCR worker panicked");
            Ok(CharacterPages {
                parsed: parsed.expect("completed character capture includes Details"),
                traces,
                details,
                last,
            })
        })
    }

    fn accept_character_traces(
        &mut self,
        result: Option<HsrResult<crate::scanner_export::CharacterDetails>>,
    ) -> HsrResult<Option<crate::scanner_export::CharacterDetails>> {
        let Some(result) = result else {
            return Ok(None);
        };
        match result {
            Ok(details) => Ok(Some(details)),
            Err(error) if !self.config.stop_on_failure && traces_unreadable(&error) => {
                yas::log_warn!(
                    "行迹等级无法从画面读出，已省略该角色的 skills/traces。完整错误详情：{}",
                    "Trace levels could not be read from the screen; skills/traces for this character were omitted. Full error details: {}",
                    error
                );
                Ok(None)
            },
            Err(error) => Err(error),
        }
    }

    fn capture_stable(&mut self) -> HsrResult<RgbImage> {
        self.ensure_attended()?;
        // Menu transition waits already ran. Take two frames; HSR's 3D preview
        // and starfield never go fully still, so do not poll until timeout.
        let first = self.device.capture_client()?;
        self.wait_attended(Duration::from_millis(
            self.config.timings.capture_interval_ms,
        ))?;
        self.ensure_attended()?;
        let second = self.device.capture_client()?;
        if chrome_settled(&first, &second) {
            return Ok(second);
        }
        log::debug!("Menu chrome differs between captures; using the latest frame (ambient animation is expected).");
        Ok(second)
    }

    /// Open a menu by hotkey unless its title is already showing. From the
    /// overworld the hotkey opens it directly; `leave_menu` can instead end on
    /// the pause menu, which ignores menu hotkeys until it is closed.
    fn open_menu(&mut self, hotkey: char, title: &[&str]) -> HsrResult<()> {
        self.prepare_menu_focus()?;
        if self.menu_open(title)? {
            return Ok(());
        }
        // Escape toggles the pause menu, so across two retries one of the
        // Escape presses always lands back on the overworld.
        for attempt in 0..3 {
            if attempt > 0 {
                self.issue_input(InputCommand::Escape)?;
                self.wait_attended(Duration::from_millis(self.config.timings.menu_close_ms))?;
            }
            self.issue_input(InputCommand::Key(hotkey))?;
            self.wait_attended(Duration::from_millis(self.config.timings.menu_open_ms))?;
            if self.wait_for_menu(title)? {
                return Ok(());
            }
        }
        Err(HsrError::new(
            "HSR-SCAN-MENU",
            hints::SCREEN_INVALID,
            format!("hotkey '{hotkey}' did not open the menu titled {title:?}"),
        ))
    }

    /// The character menu loads a 3D preview first and can take a few seconds
    /// longer than the backpack to show its title.
    fn wait_for_menu(&mut self, title: &[&str]) -> HsrResult<bool> {
        for poll in 0..MENU_OPEN_POLLS {
            if poll > 0 {
                self.wait_attended(Duration::from_millis(
                    self.config.timings.menu_poll_interval_ms.max(1),
                ))?;
            }
            if self.menu_open(title)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn menu_open(&mut self, title: &[&str]) -> HsrResult<bool> {
        let frame = self.capture_stable()?;
        self.parser.shows_menu_title(&frame, title)
    }

    /// Click a non-interactive corner so the client actually receives keys.
    /// Center clicks can confirm nearby interact prompts.
    fn prepare_menu_focus(&mut self) -> HsrResult<()> {
        self.issue_input(InputCommand::Click(crate::vision::Point::new(0.12, 0.82)))?;
        self.wait_attended(Duration::from_millis(self.config.timings.input_settle_ms))
    }

    fn leave_menu(&mut self) -> HsrResult<()> {
        self.issue_input(InputCommand::Escape)?;
        self.wait_attended(Duration::from_millis(self.config.timings.menu_close_ms))?;
        self.issue_input(InputCommand::Escape)?;
        self.wait_attended(Duration::from_millis(self.config.timings.menu_close_ms))
    }

    fn ensure_attended(&self) -> HsrResult<()> {
        ensure_device_attended(&self.device)
    }

    fn issue_input(&mut self, command: InputCommand) -> HsrResult<()> {
        self.ensure_attended()?;
        self.device.input(command)?;
        self.ensure_attended()
    }

    fn wait_attended(&mut self, duration: Duration) -> HsrResult<()> {
        self.ensure_attended()?;
        self.device.wait(duration)?;
        self.ensure_attended()
    }
}

fn ensure_device_attended<D: HsrDevice>(device: &D) -> HsrResult<()> {
    if device.is_cancelled() {
        return Err(HsrError::new(
            "HSR-SCAN-CANCELLED",
            hints::CANCELLED,
            "cancellation became active during an attended scanner operation",
        ));
    }
    if !device.is_foreground() {
        return Err(HsrError::new(
            "HSR-SCAN-FOCUS",
            hints::FOCUS_REQUIRED,
            "foreground ownership was lost during an attended scanner operation",
        ));
    }
    Ok(())
}

/// Owned count from an inventory header. A missing slash concatenates the
/// count and the capacity (`2232/3000` → `22323000`); peel a known capacity
/// when the merged number cannot itself be a count.
fn inventory_quantity_from_digits(digits: &str) -> Result<usize, String> {
    if digits.is_empty() {
        return Err("no digits".to_string());
    }
    if let Ok(quantity) = digits.parse::<usize>() {
        if (1..=4_000).contains(&quantity) {
            return Ok(quantity);
        }
    }
    for cap in ["1500", "2000", "2500", "3000", "1000", "4000", "500", "800"] {
        if let Some(head) = digits.strip_suffix(cap) {
            if let Ok(quantity) = head.parse::<usize>() {
                if (1..=4_000).contains(&quantity) {
                    return Ok(quantity);
                }
            }
        }
    }
    Err(format!("digits={digits}"))
}

fn inventory_dump_category(kind: InventoryKind) -> &'static str {
    match kind {
        InventoryKind::LightCone => "light_cones",
        InventoryKind::Gear => "gear",
    }
}

fn dump_inventory_setup(kind: InventoryKind, frame: &RgbImage) {
    let category = match kind {
        InventoryKind::LightCone => "inventory_light_cones",
        InventoryKind::Gear => "inventory_gear",
    };
    annotator::begin_item(category, 0, frame);
    annotator::add_image("full", frame);
}

fn dump_parsed_item<T: std::fmt::Debug>(
    category: &str,
    index: usize,
    frame: &RgbImage,
    parse: impl FnOnce() -> HsrResult<T>,
) -> HsrResult<T> {
    annotator::begin_item(category, index, frame);
    annotator::add_image("full", frame);
    match parse() {
        Ok(item) => {
            annotator::finalize_success(&format!("{item:#?}"));
            Ok(item)
        },
        Err(error) => {
            annotator::finalize_error(None, &error.to_string());
            Err(error)
        },
    }
}

fn chrome_settled(left: &RgbImage, right: &RgbImage) -> bool {
    [layout::UI_CHROME_LEFT, layout::UI_CHROME_RIGHT]
        .into_iter()
        .all(|rect| match (rect.crop(left), rect.crop(right)) {
            (Ok(left_crop), Ok(right_crop)) => frames_similar(&left_crop, &right_crop),
            _ => false,
        })
}

fn traces_unreadable(error: &HsrError) -> bool {
    matches!(
        error.code(),
        "HSR-OCR-SCRIPT" | "HSR-OCR-SEMANTIC" | "HSR-OCR-INFERENCE"
    )
}

impl<D: HsrDevice, R: OcrReader> ManagerMutationDevice for HsrScanner<D, R> {
    fn reread(
        &mut self,
        _matcher: &VisibleGearMatcher,
    ) -> Result<Vec<ManagedGearObservation>, String> {
        self.scan_manager_inventory()
            .map_err(|error| error.to_string())
    }

    fn toggle_once(
        &mut self,
        target: &ManagedGearObservation,
        scope: MutationScope,
    ) -> Result<(), String> {
        let result = self.toggle_once_verified(target, scope);
        result.map_err(|error| error.to_string())
    }
}

impl<D: HsrDevice, R: OcrReader> HsrScanner<D, R> {
    fn toggle_once_verified(
        &mut self,
        target: &ManagedGearObservation,
        scope: MutationScope,
    ) -> HsrResult<()> {
        // The public mutation-device adapter is an independent safety boundary:
        // reject locked or unknown mark-discard targets before touching the device.
        ensure_mutation_lock_evidence(scope, &target.state)?;
        self.device.focus_and_verify()?;

        // A complete reread detects visible duplicates before any status
        // mutation. The scan-order ordinal is used only to relocate the one
        // already-proven target within this single attended operation.
        let reread = self.scan_gear()?;
        if reread.coverage != CoverageLevel::Complete {
            return Err(HsrError::new(
                "HSR-MANAGER-INVENTORY-INCOMPLETE",
                hints::SCREEN_INVALID,
                "fresh pre-mutation inventory reread did not prove complete coverage",
            ));
        }
        let matches = reread
            .items
            .iter()
            .filter(|item| managed_observation(&item.parsed) == *target)
            .collect::<Vec<_>>();
        let [matched] = matches.as_slice() else {
            return Err(HsrError::new(
                "HSR-MANAGER-FRESH-MATCH",
                hints::SCREEN_INVALID,
                format!(
                    "expected one exact fresh visible match before click; found={}",
                    matches.len()
                ),
            ));
        };
        if target.equipped != Some(false) {
            return Err(HsrError::new(
                "HSR-MANAGER-EQUIPPED",
                hints::SCREEN_INVALID,
                "status mutation requires confidently unequipped gear",
            ));
        }

        let selected = self.select_gear_ordinal(matched.ordinal)?;
        let selected_observation = managed_observation(&selected.parsed);
        if selected_observation != *target
            || selected_observation.equipped != Some(false)
            || gear_fingerprint(&selected.parsed) != matched.semantic_fingerprint
        {
            return Err(HsrError::new(
                "HSR-MANAGER-SELECTION-DRIFT",
                hints::SCREEN_INVALID,
                "selected gear no longer equals the complete fresh visible observation",
            ));
        }

        let (field, desired) = match scope {
            MutationScope::Lock => (ManagedField::Lock, true),
            MutationScope::Unlock => (ManagedField::Lock, false),
            MutationScope::MarkDiscard => (ManagedField::Discard, true),
            MutationScope::UnmarkDiscard => (ManagedField::Discard, false),
        };
        let before = managed_field(&selected_observation.state, field);
        let collateral_before = managed_field(
            &selected_observation.state,
            match field {
                ManagedField::Lock => ManagedField::Discard,
                ManagedField::Discard => ManagedField::Lock,
            },
        );
        if before.is_none() || before == Some(desired) {
            return Err(HsrError::new(
                "HSR-MANAGER-STATE-DRIFT",
                hints::SCREEN_INVALID,
                format!("status field={field:?} is unknown or already desired before click"),
            ));
        }
        // Reapply the shared rule to the freshly selected panel. The immediate
        // pixel recapture below then binds this explicit unlocked state to the click.
        ensure_mutation_lock_evidence(scope, &selected_observation.state)?;

        // This cheap final recapture happens after all OCR calls. It binds the
        // click to the same selected cell, exact immutable panel pixels, and
        // both confidently known icon states without opening another OCR gap.
        let button = self.recapture_and_validate_pre_click(&selected, target, field)?;

        // This is the sole state-changing input. WindowsHsrDevice additionally
        // rechecks cancellation, foreground ownership, and client geometry
        // immediately before issuing the button event.
        self.issue_input(InputCommand::Click(button.center()))?;
        self.wait_attended(Duration::from_millis(self.config.timings.status_toggle_ms))?;
        let post_frame = self.capture_stable()?;
        let post = self
            .parser
            .parse_gear(&post_frame, selected.panel, &self.references)?;
        let post_observation = managed_observation(&post);
        if post_observation.matcher != target.matcher
            || post_observation.equipped != Some(false)
            || managed_field(&post_observation.state, field) != Some(desired)
            || !unrequested_known_field_unchanged(
                &selected_observation.state,
                &post_observation.state,
                field,
            )
        {
            return Err(HsrError::new(
                "HSR-MANAGER-POSTVERIFY",
                hints::SCREEN_INVALID,
                format!(
                    "fresh screenshot/OCR did not prove the requested status postcondition or preserve the known unrequested flag; collateralBefore={collateral_before:?}"
                ),
            ));
        }
        self.leave_menu()?;
        Ok(())
    }

    fn recapture_and_validate_pre_click(
        &mut self,
        selected: &SelectedGearContext,
        target: &ManagedGearObservation,
        field: ManagedField,
    ) -> HsrResult<NormRect> {
        self.ensure_attended()?;
        let frame = self.device.capture_client()?;
        self.ensure_attended()?;

        let current = selected_card(&selected.grid, &frame);
        let before_fingerprint =
            frame_fingerprint(&selected.parsed_frame, selected.panel.immutable_panel())?;
        let current_fingerprint = frame_fingerprint(&frame, selected.panel.immutable_panel())?;
        let same_card = current.is_some_and(|card| {
            card.column == selected.selection.column
                && (card.y - selected.selection.y).abs() < SELECTION_ROW_SHIFT
        });
        if !same_card || before_fingerprint != current_fingerprint {
            return Err(HsrError::new(
                "HSR-MANAGER-PRECLICK-DRIFT",
                hints::SCREEN_INVALID,
                format!(
                    "immediate pre-click frame did not preserve selected card and immutable panel fingerprint; expected={:?}, actual={current:?}",
                    selected.selection
                ),
            ));
        }

        let lock_button = selected.panel.lock_button(InventoryKind::Gear);
        let discard_button = selected.panel.discard_button();
        let (lock, lock_confidence) = detect_icon_state(&lock_button.crop(&frame)?);
        let (discard, discard_confidence) = detect_discard_state(&discard_button.crop(&frame)?);
        if lock_confidence < MANAGED_ICON_CONFIDENCE_THRESHOLD
            || discard_confidence < MANAGED_ICON_CONFIDENCE_THRESHOLD
            || lock != target.state.lock
            || discard != target.state.discard
        {
            return Err(HsrError::new(
                "HSR-MANAGER-PRECLICK-STATE",
                hints::SCREEN_INVALID,
                format!(
                    "immediate pre-click icon states were unknown or changed; lock={lock:?}@{lock_confidence:.3}, discard={discard:?}@{discard_confidence:.3}"
                ),
            ));
        }

        let button = match field {
            ManagedField::Lock => lock_button,
            ManagedField::Discard => discard_button,
        };
        let hit_target = button.crop(&frame)?;
        if hit_target.width() < 2 || hit_target.height() < 2 {
            return Err(HsrError::new(
                "HSR-MANAGER-PRECLICK-HITTEST",
                hints::SCREEN_INVALID,
                "derived status button did not produce a valid immediate hit target",
            ));
        }
        Ok(button)
    }
}

fn ensure_mutation_lock_evidence(scope: MutationScope, state: &ManagedState) -> HsrResult<()> {
    if scope.has_required_lock_evidence(Some(state)) {
        return Ok(());
    }
    Err(HsrError::new(
        "HSR-MANAGER-MARK-DISCARD-LOCK-EVIDENCE",
        hints::MARK_DISCARD_REQUIRES_UNLOCKED,
        format!(
            "mutation adapter requires fresh lock=Some(false) for MarkDiscard before status input; observedLock={:?}; no status input was issued",
            state.lock
        ),
    ))
}

struct SelectedGearContext {
    parsed: ParsedGearPanel,
    panel: StatsPanelLayout,
    grid: GridGeometry,
    selection: SelectedCard,
    parsed_frame: RgbImage,
}

struct InventorySession {
    cursor: InventoryCursor,
    grid: GridGeometry,
    panel: StatsPanelLayout,
    frame: RgbImage,
    quantity_reads: [usize; 2],
    walk: InventoryWalk,
}

fn backpack_grid(kind: InventoryKind) -> GridGeometry {
    layout::backpack_grid(match kind {
        InventoryKind::LightCone => layout::LIGHT_CONE_FIRST_ROW_Y,
        InventoryKind::Gear => layout::GEAR_FIRST_ROW_Y,
    })
}

/// WASD in the backpack is spatial navigation: `d` on the last column moves
/// focus onto the detail panel's buttons instead of wrapping. The walk is a
/// serpentine: along a row, `s` at its end, then back along the next row.
/// The selection frame cross-fades for a moment after each step, so the
/// column is also tracked by dead reckoning and resynced whenever the
/// selected card is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InventoryWalk {
    rightward: bool,
    column: usize,
}

impl Default for InventoryWalk {
    fn default() -> Self {
        Self {
            rightward: true,
            column: 0,
        }
    }
}

impl InventoryWalk {
    fn next_key(&mut self, selected: Option<usize>, columns: usize) -> char {
        if columns == 0 {
            return if self.rightward { 'd' } else { 'a' };
        }
        if let Some(index) = selected {
            self.column = index % columns;
        }
        let at_row_end = if self.rightward {
            self.column + 1 == columns
        } else {
            self.column == 0
        };
        match (at_row_end, self.rightward) {
            (true, _) => 's',
            (false, true) => 'd',
            (false, false) => 'a',
        }
    }

    fn stepped(&mut self, key: char) {
        match key {
            's' => self.rightward = !self.rightward,
            'd' => self.column += 1,
            _ => self.column = self.column.saturating_sub(1),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NextCellEvidence {
    NoCandidate,
    StayedOnLast,
    ExtraItem,
    Uncertain,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct InventoryCompletionEvidence {
    primary_quantity: usize,
    confirmation_quantity: usize,
    terminal_quantity: Option<usize>,
    next_cell: NextCellEvidence,
    terminal_scrolls: [ScrollEvidence; 2],
    scrollbar_bottom_confidence: Option<f64>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InventoryCompletionAssessment {
    coverage: CoverageLevel,
    reason: &'static str,
}

#[cfg(test)]
fn assess_inventory_completion(
    evidence: InventoryCompletionEvidence,
) -> InventoryCompletionAssessment {
    let all_quantities_agree = evidence.terminal_quantity.is_some_and(|terminal| {
        evidence.primary_quantity == evidence.confirmation_quantity
            && evidence.primary_quantity == terminal
    });
    if !all_quantities_agree {
        return InventoryCompletionAssessment {
            coverage: CoverageLevel::Unknown,
            reason: "independent inventory quantity reads disagreed or the terminal read failed",
        };
    }
    match evidence.next_cell {
        NextCellEvidence::ExtraItem => {
            return InventoryCompletionAssessment {
                coverage: CoverageLevel::Unknown,
                reason: "the cell immediately after the OCR quantity was selectable",
            };
        },
        NextCellEvidence::Uncertain => {
            return InventoryCompletionAssessment {
                coverage: CoverageLevel::Unknown,
                reason: "the cell immediately after the OCR quantity could not be disproven",
            };
        },
        NextCellEvidence::NoCandidate | NextCellEvidence::StayedOnLast => {},
    }
    if evidence
        .terminal_scrolls
        .iter()
        .any(|scroll| matches!(scroll, ScrollEvidence::Advanced { .. }))
    {
        return InventoryCompletionAssessment {
            coverage: CoverageLevel::Unknown,
            reason: "inventory advanced beyond the OCR quantity bound",
        };
    }
    if evidence
        .terminal_scrolls
        .iter()
        .any(|scroll| matches!(scroll, ScrollEvidence::Uncertain { .. }))
    {
        return InventoryCompletionAssessment {
            coverage: CoverageLevel::Unknown,
            reason: "one or more terminal scroll motions were visually uncertain",
        };
    }
    if evidence
        .terminal_scrolls
        .iter()
        .any(|scroll| matches!(scroll, ScrollEvidence::End { confidence } if *confidence < 0.50))
    {
        return InventoryCompletionAssessment {
            coverage: CoverageLevel::Unknown,
            reason: "repeated bottom-of-inventory evidence was below the conservative confidence threshold",
        };
    }
    if !evidence
        .scrollbar_bottom_confidence
        .is_some_and(|confidence| confidence >= 0.50)
    {
        return InventoryCompletionAssessment {
            coverage: CoverageLevel::Unknown,
            reason: "the visible scrollbar thumb did not independently prove the terminal position",
        };
    }
    InventoryCompletionAssessment {
        coverage: CoverageLevel::Complete,
        reason: "three quantity reads agreed, the next cell was absent, two attended scrolls stayed at the visually proven scrollbar bottom",
    }
}

fn is_omittable_ambiguity(code: &str) -> bool {
    matches!(code, "HSR-OCR-GEAR-AMBIGUOUS" | "HSR-OCR-STAT-AMBIGUOUS")
}

fn is_omittable_scan_error(code: &str) -> bool {
    is_omittable_ambiguity(code) || code.starts_with("HSR-OCR-")
}

fn log_inventory_coverage_warning(kind: InventoryKind, scanned: usize, reason: &str) {
    yas::log_warn!(
        "库存数量或末端状态无法交叉验证（{:?}）；保留已扫描的 {} 项，但覆盖率标记为未知，管理器不会执行变更。完整错误详情：{}",
        "Inventory quantity or terminal state could not be cross-validated ({:?}); retained {} scanned entries, but coverage is marked unknown and the manager will not mutate. Full error details: {}",
        kind,
        scanned,
        reason
    );
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InventoryStep {
    Scroll { rows: usize },
    Select { ordinal: usize, cell_index: usize },
}

/// Quantity-bounded row-major inventory cursor. It models the overlap created
/// when the last page is clamped to the bottom of the list. No item identity is
/// involved, so two genuinely identical adjacent pieces are both visited.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InventoryCursor {
    quantity: usize,
    // Grid shape is checked in `new`. The live walk only reads `quantity`.
    #[allow(dead_code)]
    columns: usize,
    #[allow(dead_code)]
    visible_rows: usize,
    #[allow(dead_code)]
    visible_start_row: usize,
    #[allow(dead_code)]
    next_ordinal: usize,
}

impl InventoryCursor {
    fn new(quantity: usize, grid: &GridGeometry) -> HsrResult<Self> {
        let columns = grid.columns();
        let visible_rows = grid.rows();
        if columns == 0 || visible_rows == 0 || grid.centers.iter().any(|row| row.len() != columns)
        {
            return Err(HsrError::new(
                "HSR-GRID-GEOMETRY",
                hints::SCREEN_INVALID,
                format!(
                    "inventory grid is empty or non-rectangular; rows={visible_rows}, columns={columns}"
                ),
            ));
        }
        Ok(Self {
            quantity,
            columns,
            visible_rows,
            visible_start_row: 0,
            next_ordinal: 0,
        })
    }

    fn quantity(&self) -> usize {
        self.quantity
    }

    #[cfg(test)]
    fn last_selected_cell(&self) -> Option<usize> {
        let ordinal = self.next_ordinal.checked_sub(1)?;
        let viewport_start = self.visible_start_row * self.columns;
        (ordinal >= viewport_start).then_some(ordinal - viewport_start)
    }

    #[cfg(test)]
    fn next_step(&self) -> HsrResult<Option<InventoryStep>> {
        if self.next_ordinal >= self.quantity {
            return Ok(None);
        }
        let viewport_start = self.visible_start_row * self.columns;
        let viewport_end = viewport_start + self.visible_rows * self.columns;
        if self.next_ordinal >= viewport_end {
            let total_rows = self.quantity.div_ceil(self.columns);
            let maximum_start = total_rows.saturating_sub(self.visible_rows);
            let next_start = (self.visible_start_row + self.visible_rows).min(maximum_start);
            if next_start <= self.visible_start_row {
                return Err(HsrError::new(
                    "HSR-GRID-BOUNDS",
                    hints::SCREEN_INVALID,
                    format!(
                        "quantity requires another item but no later viewport exists; quantity={}, nextOrdinal={}, startRow={} grid={}x{}",
                        self.quantity,
                        self.next_ordinal,
                        self.visible_start_row,
                        self.columns,
                        self.visible_rows
                    ),
                ));
            }
            return Ok(Some(InventoryStep::Scroll {
                rows: next_start - self.visible_start_row,
            }));
        }
        if self.next_ordinal < viewport_start {
            return Err(HsrError::new(
                "HSR-GRID-ORDER",
                hints::SCREEN_INVALID,
                format!(
                    "row-major cursor moved backwards; nextOrdinal={}, viewportStart={viewport_start}",
                    self.next_ordinal
                ),
            ));
        }
        Ok(Some(InventoryStep::Select {
            ordinal: self.next_ordinal,
            cell_index: self.next_ordinal - viewport_start,
        }))
    }

    #[cfg(test)]
    fn mark_selected(&mut self, ordinal: usize) -> HsrResult<()> {
        match self.next_step()? {
            Some(InventoryStep::Select {
                ordinal: expected, ..
            }) if expected == ordinal => {
                self.next_ordinal += 1;
                Ok(())
            },
            other => Err(HsrError::new(
                "HSR-GRID-ORDER",
                hints::SCREEN_INVALID,
                format!(
                    "selection acknowledgement was out of order; ordinal={ordinal}, expected={other:?}"
                ),
            )),
        }
    }

    #[cfg(test)]
    fn mark_scrolled(&mut self, rows: usize) -> HsrResult<()> {
        match self.next_step()? {
            Some(InventoryStep::Scroll { rows: expected }) if expected == rows => {
                self.visible_start_row += rows;
                Ok(())
            },
            other => Err(HsrError::new(
                "HSR-GRID-ORDER",
                hints::SCREEN_INVALID,
                format!("scroll acknowledgement was out of order; rows={rows}, expected={other:?}"),
            )),
        }
    }
}

struct CharacterScan {
    items: Vec<ObservedCharacter>,
    details: CaptureExportDetails,
    coverage: CoverageLevel,
}

pub fn gear_fingerprint(parsed: &ParsedGearPanel) -> String {
    let gear = &parsed.observation;
    let mut hasher = Sha256::new();
    hasher.update(parsed.reference.key.as_bytes());
    hasher.update([0]);
    hasher.update(parsed.reference.set_key.as_bytes());
    hasher.update([parsed.reference.rarity, gear.level]);
    hasher.update(format!("{:?}", parsed.reference.slot).as_bytes());
    hasher.update(gear.main_stat_key.as_bytes());
    hasher.update(gear.main_stat_value.to_bits().to_le_bytes());
    for substat in &gear.substats {
        hasher.update(substat.stat_key.as_bytes());
        hasher.update(substat.value.to_bits().to_le_bytes());
    }
    if let Some(location) = &parsed.location_key {
        hasher.update(location.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub fn managed_observation(parsed: &ParsedGearPanel) -> ManagedGearObservation {
    let mut substats = parsed
        .observation
        .substats
        .iter()
        .map(|stat| VisibleStat {
            key: stat.stat_key.clone(),
            value: stat.value,
        })
        .collect::<Vec<_>>();
    substats.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.value.total_cmp(&right.value))
    });
    let status_evidence_is_actionable = parsed.icon_confidence.is_finite()
        && parsed.icon_confidence >= MANAGED_ICON_CONFIDENCE_THRESHOLD;
    ManagedGearObservation {
        matcher: VisibleGearMatcher {
            key: parsed.reference.key.clone(),
            game_id: parsed.reference.game_id,
            set_key: parsed.reference.set_key.clone(),
            slot: parsed.reference.slot,
            rarity: parsed.reference.rarity,
            level: parsed.observation.level,
            main_stat: VisibleStat {
                key: parsed.observation.main_stat_key.clone(),
                value: parsed.observation.main_stat_value,
            },
            substats,
            location_key: parsed.location_key.clone(),
        },
        state: ManagedState {
            lock: status_evidence_is_actionable
                .then_some(parsed.observation.lock)
                .flatten(),
            discard: status_evidence_is_actionable
                .then_some(parsed.observation.discard)
                .flatten(),
        },
        equipped: parsed.equipped,
    }
}

fn managed_field(state: &ManagedState, field: ManagedField) -> Option<bool> {
    match field {
        ManagedField::Lock => state.lock,
        ManagedField::Discard => state.discard,
    }
}

fn unrequested_known_field_unchanged(
    before: &ManagedState,
    after: &ManagedState,
    requested: ManagedField,
) -> bool {
    let other = match requested {
        ManagedField::Lock => ManagedField::Discard,
        ManagedField::Discard => ManagedField::Lock,
    };
    match managed_field(before, other) {
        Some(value) => managed_field(after, other) == Some(value),
        None => true,
    }
}

/// Activated Eidolon badges have a continuous white ring; locked badges show
/// only a dim outline (ring fraction ≤0.14 vs ≥0.90 on 77 live Characters).
/// A node that can be activated but has not been also has the ring, yet its
/// disk glows orange (mean red−blue ≈+40 vs +2..+21 for activated art), so
/// warm disks are rejected.
fn eidolon_node_unlocked(frame: &RgbImage, node: Point) -> bool {
    annotator::record_node(
        "eidolon_node",
        frame,
        node.x,
        node.y,
        layout::EIDOLON_RING_RADIUS + 5.0 / 1080.0,
    );
    const RING_SAMPLES: usize = 72;
    const MIN_RING_FRACTION: f64 = 0.5;
    const MAX_DISK_WARMTH: f64 = 30.0;
    const SHIFT: f64 = 4.0 / 1080.0;
    let height = frame.height() as f64;
    let center_x = node.x * frame.width() as f64;
    let center_y = node.y * height;
    let ring_radius = layout::EIDOLON_RING_RADIUS * height;
    let shift = (SHIFT * height).round() as i32;
    let white = |x: f64, y: f64| {
        let (x, y) = (x.round(), y.round());
        if x < 0.0 || y < 0.0 || x >= frame.width() as f64 || y >= height {
            return false;
        }
        frame
            .get_pixel(x as u32, y as u32)
            .0
            .iter()
            .all(|&channel| channel > 170)
    };
    // The badge can sit a few pixels off its nominal center, so the ring is
    // matched over a small shift search.
    let ring = (-shift..=shift)
        .step_by(2)
        .flat_map(|dy| (-shift..=shift).step_by(2).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| {
            let hits = (0..RING_SAMPLES)
                .filter(|&k| {
                    let angle = std::f64::consts::TAU * k as f64 / RING_SAMPLES as f64;
                    white(
                        center_x + dx as f64 + ring_radius * angle.cos(),
                        center_y + dy as f64 + ring_radius * angle.sin(),
                    )
                })
                .count();
            hits as f64 / RING_SAMPLES as f64
        })
        .fold(0.0, f64::max);
    if ring < MIN_RING_FRACTION {
        return false;
    }
    disk_warmth(
        frame,
        center_x,
        center_y,
        layout::EIDOLON_DISK_RADIUS * height,
    ) < MAX_DISK_WARMTH
}

fn disk_warmth(frame: &RgbImage, center_x: f64, center_y: f64, radius: f64) -> f64 {
    let (mut total, mut count) = (0_i64, 0_i64);
    let reach = radius.ceil() as i32;
    for dy in (-reach..=reach).step_by(2) {
        for dx in (-reach..=reach).step_by(2) {
            if f64::from(dx * dx + dy * dy) > radius * radius {
                continue;
            }
            let x = center_x.round() as i32 + dx;
            let y = center_y.round() as i32 + dy;
            if x < 0 || y < 0 || x >= frame.width() as i32 || y >= frame.height() as i32 {
                continue;
            }
            let [r, _, b] = frame.get_pixel(x as u32, y as u32).0;
            total += i64::from(r) - i64::from(b);
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    total as f64 / count as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;
    use yas::cancel::StopReason;

    use crate::{
        device::ReplayDevice, localization::Language, model::ReferenceSnapshot,
        ocr::ScriptedOcrReader,
    };

    #[test]
    fn inventory_walk_is_a_serpentine_over_rows() {
        let mut walk = InventoryWalk::default();
        let mut keys = String::new();
        for _ in 0..17 {
            let key = walk.next_key(None, 8);
            keys.push(key);
            walk.stepped(key);
        }

        assert_eq!(keys, "dddddddsaaaaaaasd");
    }

    #[test]
    fn uid_is_accepted_only_after_two_panels_agree() {
        let snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let mut scanner = HsrScanner::new(
            ReplayDevice::new(1280, 720, Vec::new()),
            ScriptedOcrReader::default().with(
                OcrField::Uid,
                [
                    "UID:600732506",
                    "UID:600782506",
                    "UID:600732506",
                    "UID:600732506",
                ],
            ),
            ReferenceCache::from_snapshot(snapshot).unwrap(),
            ScanConfig::default(),
        );
        let frame = RgbImage::new(1280, 720);

        let mut observed = Vec::new();
        for _ in 0..4 {
            scanner.observe_uid(&frame);
            observed.push(scanner.uid);
        }

        assert_eq!(observed, [None, None, None, Some(600_732_506)]);
    }

    #[test]
    fn eidolon_count_stops_at_the_first_badge_without_an_activated_ring() {
        let mut frame = RgbImage::from_pixel(1920, 1080, Rgb([30, 32, 40]));
        let paint_badge = |frame: &mut RgbImage, node: Point, ring: bool, disk: Rgb<u8>| {
            let (cx, cy) = (node.x * 1920.0, node.y * 1080.0);
            for y in (cy as u32 - 45)..(cy as u32 + 45) {
                for x in (cx as u32 - 45)..(cx as u32 + 45) {
                    let distance = (x as f64 - cx).hypot(y as f64 - cy);
                    if distance <= 30.0 {
                        frame.put_pixel(x, y, disk);
                    } else if ring && (37.0..=41.0).contains(&distance) {
                        frame.put_pixel(x, y, Rgb([235, 235, 235]));
                    }
                }
            }
        };
        let activated_art = Rgb([120, 110, 105]);
        paint_badge(&mut frame, layout::EIDOLON_NODES[0], true, activated_art);
        paint_badge(&mut frame, layout::EIDOLON_NODES[1], true, activated_art);
        paint_badge(
            &mut frame,
            layout::EIDOLON_NODES[2],
            true,
            Rgb([220, 150, 60]),
        );
        paint_badge(&mut frame, layout::EIDOLON_NODES[3], false, activated_art);

        let unlocked: Vec<bool> = layout::EIDOLON_NODES
            .iter()
            .map(|&node| eidolon_node_unlocked(&frame, node))
            .collect();

        assert_eq!(unlocked, [true, true, false, false, false, false]);
    }

    #[test]
    fn inventory_walk_resyncs_from_the_visible_selection() {
        let mut walk = InventoryWalk::default();

        assert_eq!(walk.next_key(Some(7), 8), 's');
        walk.stepped('s');
        assert_eq!(walk.next_key(Some(15), 8), 'a');
        assert_eq!(walk.next_key(Some(8), 8), 's');
    }

    fn test_grid() -> GridGeometry {
        GridGeometry {
            centers: (0..5)
                .map(|row| {
                    (0..9)
                        .map(|column| {
                            crate::vision::Point::new(
                                0.071 + column as f64 * 0.0645,
                                0.26 + row as f64 * 0.138,
                            )
                        })
                        .collect()
                })
                .collect(),
            cell_width: 0.0645 * 0.90,
            cell_height: 0.138 * 0.90,
            confidence: 1.0,
        }
    }

    #[test]
    fn live_scanner_honors_caller_cancel_before_device_or_ocr_setup() {
        let snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let references = ReferenceCache::from_snapshot(snapshot).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel(StopReason::UserAbort);

        let error = HsrScanner::live_with_cancel(references, ScanConfig::default(), cancel)
            .err()
            .expect("a pre-cancelled live scanner must not initialize device or OCR state");

        assert_eq!(error.code(), "HSR-DEVICE-CANCELLED");
    }

    fn page_frame(
        grid: &GridGeometry,
        start_row: usize,
        quantity: usize,
        selected_global: Option<usize>,
    ) -> RgbImage {
        let width = 1280;
        let height = 720;
        let mut image = RgbImage::from_pixel(width, height, Rgb([24, 28, 35]));
        for (row, centers) in grid.centers.iter().enumerate() {
            for (column, center) in centers.iter().enumerate() {
                let global = (start_row + row) * grid.columns() + column;
                if global < quantity {
                    let color = Rgb([
                        45 + (global * 17 % 120) as u8,
                        50 + (global * 29 % 115) as u8,
                        55 + (global * 43 % 110) as u8,
                    ]);
                    paint_rect(
                        &mut image,
                        *center,
                        grid.cell_width * 0.68,
                        grid.cell_height * 0.68,
                        color,
                    );
                }
                if selected_global == Some(global) {
                    paint_ring(&mut image, *center, grid.cell_width, grid.cell_height);
                }
            }
        }
        let gutter_x = grid.centers[0].last().unwrap().x + grid.cell_width / 2.0 + 0.018;
        let total_rows = quantity.div_ceil(grid.columns());
        let maximum_start = total_rows.saturating_sub(grid.rows());
        let track_bottom = grid.centers.last().unwrap().last().unwrap().y + grid.cell_height / 2.0;
        let thumb_center_y = if start_row >= maximum_start {
            track_bottom - 0.04
        } else {
            0.24 + start_row as f64 * 0.018
        };
        paint_rect(
            &mut image,
            crate::vision::Point::new(gutter_x, thumb_center_y),
            0.008,
            0.08,
            Rgb([210, 210, 215]),
        );
        image
    }

    fn paint_rect(
        image: &mut RgbImage,
        center: crate::vision::Point,
        width: f64,
        height: f64,
        color: Rgb<u8>,
    ) {
        let x0 = ((center.x - width / 2.0) * image.width() as f64).max(0.0) as u32;
        let x1 = ((center.x + width / 2.0) * image.width() as f64)
            .min(image.width().saturating_sub(1) as f64) as u32;
        let y0 = ((center.y - height / 2.0) * image.height() as f64).max(0.0) as u32;
        let y1 = ((center.y + height / 2.0) * image.height() as f64)
            .min(image.height().saturating_sub(1) as f64) as u32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                image.put_pixel(x, y, color);
            }
        }
    }

    fn paint_ring(image: &mut RgbImage, center: crate::vision::Point, width: f64, height: f64) {
        let cx = (center.x * image.width() as f64).round() as i32;
        let cy = (center.y * image.height() as f64).round() as i32;
        let half_w = (width * image.width() as f64 / 2.0).round() as i32;
        let half_h = (height * image.height() as f64 / 2.0).round() as i32;
        for thickness in 0..4 {
            for x in (cx - half_w)..=(cx + half_w) {
                for y in [cy - half_h + thickness, cy + half_h - thickness] {
                    image.put_pixel(x as u32, y as u32, Rgb([250, 250, 250]));
                }
            }
            for y in (cy - half_h)..=(cy + half_h) {
                for x in [cx - half_w + thickness, cx + half_w - thickness] {
                    image.put_pixel(x as u32, y as u32, Rgb([250, 250, 250]));
                }
            }
        }
    }

    fn scanner_with_frames(
        frames: Vec<RgbImage>,
        mut config: ScanConfig,
    ) -> HsrScanner<ReplayDevice, ScriptedOcrReader> {
        config.timings.panel_timeout_ms = 40;
        let snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let references = ReferenceCache::from_snapshot(snapshot).unwrap();
        HsrScanner::new(
            ReplayDevice::new(1280, 720, frames),
            ScriptedOcrReader::default(),
            references,
            config,
        )
    }

    #[test]
    fn hdr_automatic_capture_and_explicit_overrides_use_the_selected_backend() {
        let mut config = ScanConfig {
            hdr_mode: true,
            ..Default::default()
        };
        assert_eq!(config.effective_capture_method(), CaptureMethod::Wgc);
        for method in [CaptureMethod::BitBlt, CaptureMethod::Wgc] {
            config.capture_method = Some(method);
            assert_eq!(config.effective_capture_method(), method);
        }
        config.capture_method = None;
        config.hdr_mode = false;
        assert_eq!(config.effective_capture_method(), CaptureMethod::BitBlt);
    }

    #[test]
    fn defaults_scan_all_and_use_shared_bitblt_capture() {
        let config = ScanConfig::default();
        assert_eq!(config.capture_method, None);
        assert_eq!(config.effective_capture_method(), CaptureMethod::BitBlt);
        assert_eq!(config.max_light_cones, 0);
        assert_eq!(config.max_gear, 0);
        assert_eq!(config.max_characters, 0);
        assert!(config.inventory_scroll_ticks_per_page > 0);
        assert!(!config.scroll_tick_delay.is_zero());
        assert!(!config.dump_images);
    }

    #[test]
    fn cursor_visits_partial_final_page_without_deduplicating_ordinals() {
        let grid = test_grid();
        let mut cursor = InventoryCursor::new(50, &grid).unwrap();
        let mut selected = Vec::new();
        let mut scrolls = Vec::new();
        while let Some(step) = cursor.next_step().unwrap() {
            match step {
                InventoryStep::Select {
                    ordinal,
                    cell_index,
                } => {
                    selected.push((ordinal, cell_index));
                    cursor.mark_selected(ordinal).unwrap();
                },
                InventoryStep::Scroll { rows } => {
                    scrolls.push(rows);
                    cursor.mark_scrolled(rows).unwrap();
                },
            }
        }
        assert_eq!(selected.len(), 50);
        assert_eq!(
            selected.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            (0..50).collect::<Vec<_>>()
        );
        assert_eq!(scrolls, vec![1]);
        assert_eq!(
            &selected[45..],
            &[(45, 36), (46, 37), (47, 38), (48, 39), (49, 40)]
        );
    }

    #[test]
    fn production_page_walker_issues_row_major_clicks_then_verified_scroll() {
        let grid = test_grid();
        let initial = page_frame(&grid, 0, 50, Some(0));
        let mut frames = Vec::new();
        for ordinal in 1..45 {
            let frame = page_frame(&grid, 0, 50, Some(ordinal));
            frames.extend([frame.clone(), frame]);
        }
        let after_scroll = page_frame(&grid, 1, 50, Some(44));
        frames.extend([after_scroll.clone(), after_scroll]);
        for ordinal in 45..50 {
            let frame = page_frame(&grid, 1, 50, Some(ordinal));
            frames.extend([frame.clone(), frame]);
        }
        let config = ScanConfig {
            inventory_scroll_ticks_per_page: 5,
            scroll_tick_delay: Duration::from_millis(1),
            ..ScanConfig::default()
        };
        let mut scanner = scanner_with_frames(frames, config);
        let mut session = InventorySession {
            cursor: InventoryCursor::new(50, &grid).unwrap(),
            grid: grid.clone(),
            panel: StatsPanelLayout::MAINTAINED_SEED,
            frame: initial,
            quantity_reads: [50, 50],
            walk: InventoryWalk::default(),
        };
        let mut visited = Vec::new();
        while let Some(ordinal) = scanner.advance_inventory_session(&mut session).unwrap() {
            visited.push(ordinal);
        }
        assert_eq!(visited, (0..50).collect::<Vec<_>>());

        let commands = scanner.device().commands();
        assert_eq!(commands.len(), 50);
        for (ordinal, command) in commands[..44].iter().enumerate() {
            assert_eq!(
                command,
                &InputCommand::Click(grid.cell(ordinal + 1).unwrap())
            );
        }
        assert_eq!(commands[44], InputCommand::Scroll(1));
        for (offset, command) in commands[45..].iter().enumerate() {
            assert_eq!(
                command,
                &InputCommand::Click(grid.cell(36 + offset).unwrap())
            );
        }
    }

    #[test]
    fn unchanged_scroll_is_an_early_end_not_silent_complete_coverage() {
        let grid = test_grid();
        let before = page_frame(&grid, 0, 90, Some(44));
        let config = ScanConfig {
            inventory_scroll_ticks_per_page: 5,
            scroll_tick_delay: Duration::from_millis(1),
            ..ScanConfig::default()
        };
        let mut scanner = scanner_with_frames(vec![before.clone(), before.clone()], config);
        let error = scanner
            .scroll_inventory_rows(&before, &grid, StatsPanelLayout::MAINTAINED_SEED, 5)
            .unwrap_err();
        assert_eq!(error.code(), "HSR-SCAN-EARLY-END");
        assert!(error
            .localized_message(crate::localization::Language::En)
            .contains("could not be validated"));
        assert!(error
            .localized_message(crate::localization::Language::ZhCn)
            .contains("无法通过"));
        assert_eq!(
            scanner.device().commands(),
            &vec![InputCommand::Scroll(1); 5]
        );
    }

    #[test]
    fn focus_loss_and_cancel_stop_before_any_screenshot_is_consumed() {
        let grid = test_grid();

        let sentinel = RgbImage::from_pixel(1280, 720, Rgb([7, 8, 9]));
        let mut focus_scanner = scanner_with_frames(vec![sentinel.clone()], ScanConfig::default());
        focus_scanner.device.lose_focus_after_commands(1);
        let focus_error = focus_scanner
            .select_inventory_cell(&grid, StatsPanelLayout::MAINTAINED_SEED, 1, 1)
            .unwrap_err();
        assert_eq!(focus_error.code(), "HSR-SCAN-FOCUS");
        assert_eq!(focus_scanner.device().commands().len(), 1);
        assert_eq!(focus_scanner.device().remaining_frames(), 1);

        let mut cancel_scanner = scanner_with_frames(vec![sentinel], ScanConfig::default());
        cancel_scanner.device.cancel_after_commands(1);
        let cancel_error = cancel_scanner
            .select_inventory_cell(&grid, StatsPanelLayout::MAINTAINED_SEED, 1, 1)
            .unwrap_err();
        assert_eq!(cancel_error.code(), "HSR-SCAN-CANCELLED");
        assert_eq!(cancel_scanner.device().commands().len(), 1);
        assert_eq!(cancel_scanner.device().remaining_frames(), 1);
    }

    #[test]
    fn inventory_quantity_keeps_the_owned_count_when_the_slash_is_missing() {
        assert_eq!(inventory_quantity_from_digits("257").unwrap(), 257);
        assert_eq!(inventory_quantity_from_digits("2232").unwrap(), 2232);
        assert_eq!(inventory_quantity_from_digits("22323000").unwrap(), 2232);
        assert_eq!(inventory_quantity_from_digits("2571500").unwrap(), 257);
        assert_eq!(inventory_quantity_from_digits("30003000").unwrap(), 3000);
        assert!(inventory_quantity_from_digits("").is_err());
        assert!(inventory_quantity_from_digits("22329999").is_err());
    }

    #[test]
    fn character_navigation_requires_visible_identity_change() {
        let mut previous = RgbImage::from_pixel(1280, 720, Rgb([24, 28, 35]));
        let mut changed = previous.clone();
        paint_rect(
            &mut changed,
            crate::vision::Point::new(0.158, 0.073),
            0.10,
            0.02,
            Rgb([180, 120, 90]),
        );
        let config = ScanConfig::default();
        paint_character_portraits(&mut previous);
        let mut scanner =
            scanner_with_frames(vec![changed.clone(), changed.clone()], config.clone());
        let mut slot = 0;
        let next = scanner.advance_character(&previous, &mut slot, 0).unwrap();
        assert_eq!(next, changed);
        assert_eq!(
            scanner.device().commands(),
            &[InputCommand::Click(layout::character_portrait(1))]
        );
        assert_eq!(slot, 1);

        let mut unchanged = RgbImage::from_pixel(1280, 720, Rgb([24, 28, 35]));
        paint_character_portraits(&mut unchanged);
        let mut stuck = scanner_with_frames(vec![unchanged.clone(); 12], config);
        slot = 0;
        let error = stuck
            .advance_character(&unchanged, &mut slot, 0)
            .unwrap_err();
        assert_eq!(error.code(), "HSR-CHAR-ADVANCE");
        assert_eq!(
            stuck.device().commands(),
            &[
                InputCommand::Click(layout::character_portrait(1)),
                InputCommand::Click(layout::character_portrait(1)),
                InputCommand::Click(layout::character_portrait(1)),
            ]
        );
        assert_eq!(slot, 0);
    }

    #[test]
    fn ninth_portrait_pages_once_and_waits_for_a_delayed_first_slot_click() {
        let previous = character_frame(Rgb([90, 120, 170]));
        let mut paged = previous.clone();
        paint_character_portraits(&mut paged);
        let mut changed = character_frame(Rgb([170, 90, 120]));
        paint_character_portraits(&mut changed);
        let mut scanner = scanner_with_frames(
            vec![
                paged.clone(),
                paged.clone(),
                paged,
                changed.clone(),
                changed.clone(),
            ],
            ScanConfig {
                timings: ScanTimings {
                    panel_timeout_ms: 40,
                    ..ScanTimings::default()
                },
                ..ScanConfig::default()
            },
        );
        let mut slot = 8;
        assert_eq!(
            scanner.advance_character(&previous, &mut slot, 8).unwrap(),
            changed
        );
        assert_eq!(slot, 0);
        let (from, to) = layout::character_page_drag(true);
        assert_eq!(
            scanner.device().commands(),
            &[
                InputCommand::Drag { from, to },
                InputCommand::Click(layout::character_portrait(0)),
            ]
        );
    }

    #[test]
    fn clamped_page_may_start_with_the_previous_selected_character() {
        let previous = character_frame(Rgb([90, 120, 170]));
        let mut paged = previous.clone();
        paint_character_portraits(&mut paged);
        let mut scanner = scanner_with_frames(
            vec![paged.clone(); 5],
            ScanConfig {
                timings: ScanTimings {
                    panel_timeout_ms: 40,
                    ..ScanTimings::default()
                },
                ..ScanConfig::default()
            },
        );
        let mut slot = 8;
        assert_eq!(
            scanner.advance_character(&previous, &mut slot, 8).unwrap(),
            paged
        );
        assert_eq!(slot, 0);
    }

    #[test]
    fn stationary_final_page_stops_without_clicking_the_first_slot_again() {
        let previous = character_frame(Rgb([90, 120, 170]));
        let mut scanner = scanner_with_frames(vec![previous.clone(); 2], ScanConfig::default());
        let mut slot = 8;
        assert_eq!(
            scanner
                .advance_character(&previous, &mut slot, 8)
                .unwrap_err()
                .code(),
            "HSR-CHAR-END"
        );
        let (from, to) = layout::character_page_drag(true);
        assert_eq!(
            scanner.device().commands(),
            &[
                InputCommand::Drag { from, to },
                InputCommand::Drag { from, to }
            ]
        );
        assert_eq!(slot, 8);
    }

    #[test]
    fn portrait_page_comparison_ignores_animation_outside_the_faces() {
        let before = RgbImage::from_pixel(1280, 720, Rgb([24, 28, 35]));
        let mut after = before.clone();
        paint_rect(
            &mut after,
            Point::new(0.25, 0.1),
            0.04,
            0.02,
            Rgb([255, 255, 255]),
        );
        assert!(character_bar_unchanged(&before, &after).unwrap());
        paint_rect(
            &mut after,
            layout::character_portrait(4),
            0.02,
            0.03,
            Rgb([170, 90, 120]),
        );
        assert!(!character_bar_unchanged(&before, &after).unwrap());
    }

    fn paint_character_portraits(frame: &mut RgbImage) {
        let width = frame.width();
        for (center, half_width) in [
            (width * 35 / 100, 28),
            (width * 42 / 100, 48),
            (width * 49 / 100, 28),
        ] {
            for x in center.saturating_sub(half_width)..=center + half_width {
                for y in 30..70 {
                    if x < frame.width() && y < frame.height() {
                        frame.put_pixel(x, y, Rgb([230, 190, 80]));
                    }
                }
            }
        }
    }

    fn character_frame(color: Rgb<u8>) -> RgbImage {
        let mut frame = RgbImage::from_pixel(1280, 720, Rgb([24, 28, 35]));
        paint_rect(
            &mut frame,
            crate::vision::Point::new(0.158, 0.073),
            0.10,
            0.02,
            color,
        );
        frame
    }

    /// `open_menu` spends one stable capture (two frames) reading the title.
    fn with_character_menu_open(frames: Vec<RgbImage>) -> Vec<RgbImage> {
        let title_frame = frames[0].clone();
        [title_frame.clone(), title_frame]
            .into_iter()
            .chain(frames)
            .collect()
    }

    fn two_character_references() -> ReferenceCache {
        let mut snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let mut second = snapshot.characters[0].clone();
        second.game_id = 1102;
        second.key = "1102".to_string();
        second.name.zh_cn = "希儿".to_string();
        second.name.en = "Seele".to_string();
        second.path = "Rogue".to_string();
        snapshot.characters.push(second);
        ReferenceCache::from_snapshot(snapshot).unwrap()
    }

    fn run_character_simulation(
        frames: Vec<RgbImage>,
        names: &[&str],
        maximum: usize,
    ) -> CharacterScan {
        let frames = with_character_menu_open(frames);
        let reader = ScriptedOcrReader::default()
            .with(OcrField::MenuTitle, ["角色详情"])
            .with(OcrField::CharacterName, names.iter().copied())
            .with(
                OcrField::CharacterLevel,
                std::iter::repeat_n("等级 80/80", names.len()),
            );
        let config = ScanConfig {
            timings: ScanTimings {
                panel_timeout_ms: 40,
                ..ScanTimings::default()
            },
            max_characters: maximum,
            ..ScanConfig::default()
        };
        let mut scanner = HsrScanner::new(
            ReplayDevice::new(1280, 720, frames),
            reader,
            two_character_references(),
            config,
        );
        scanner.scan_characters().unwrap()
    }

    #[test]
    fn character_pages_capture_once_and_overlap_ocr_in_both_directions() {
        struct PipelineDevice {
            replay: ReplayDevice,
            release: std::sync::mpsc::Sender<()>,
            captures: usize,
            waits: Vec<Duration>,
        }
        impl HsrDevice for PipelineDevice {
            fn identity(&self) -> &crate::device::WindowIdentity {
                self.replay.identity()
            }
            fn capture_client(&mut self) -> HsrResult<RgbImage> {
                self.captures += 1;
                if self.captures == 2 {
                    self.release.send(()).unwrap();
                }
                self.replay.capture_client()
            }
            fn focus_and_verify(&mut self) -> HsrResult<()> {
                self.replay.focus_and_verify()
            }
            fn is_foreground(&self) -> bool {
                self.replay.is_foreground()
            }
            fn input(&mut self, command: InputCommand) -> HsrResult<()> {
                self.replay.input(command)
            }
            fn wait(&mut self, duration: Duration) -> HsrResult<()> {
                self.waits.push(duration);
                self.replay.wait(duration)
            }
            fn is_cancelled(&self) -> bool {
                self.replay.is_cancelled()
            }
        }
        struct WaitingReader {
            reader: ScriptedOcrReader,
            release: std::sync::mpsc::Receiver<()>,
        }
        impl OcrReader for WaitingReader {
            fn read(&mut self, field: OcrField, image: &RgbImage) -> HsrResult<String> {
                if field == OcrField::CharacterName {
                    // Serial OCR would time out: only the next tab's capture
                    // releases this first page read. No sleep or speed assertion.
                    self.release.recv_timeout(Duration::from_secs(2)).unwrap();
                }
                self.reader.read(field, image)
            }
        }
        for reverse in [false, true] {
            let frame = character_frame(Rgb([90, 120, 170]));
            let (sender, receiver) = std::sync::mpsc::channel();
            let device = PipelineDevice {
                replay: ReplayDevice::new(1280, 720, vec![frame; 3]),
                release: sender,
                captures: 0,
                waits: Vec::new(),
            };
            let mut reader = ScriptedOcrReader::default()
                .with(OcrField::CharacterName, ["三月七"])
                .with(OcrField::CharacterLevel, ["等级 80/80"]);
            for key in ["basic", "skill", "ult", "talent"] {
                reader = reader.with(
                    OcrField::CharacterSkill(key),
                    [if key == "basic" { "1/6" } else { "1/10" }],
                );
            }
            let mut scanner = HsrScanner::new(
                device,
                WaitingReader {
                    reader,
                    release: receiver,
                },
                two_character_references(),
                ScanConfig::default(),
            );
            let pages = scanner.read_character_pages(0, reverse).unwrap();
            let character = pages.parsed.unwrap();
            assert_eq!(character.observation.character_id, 1001);
            assert_eq!(character.observation.level, 80);
            assert_eq!(character.observation.eidolon, 0);
            assert!(pages.traces.unwrap().is_ok());
            assert_eq!(scanner.device.captures, 3);
            assert_eq!(
                scanner.device.replay.commands(),
                &[
                    InputCommand::Click(layout::TRACES_BUTTON),
                    InputCommand::Click(if reverse {
                        layout::DETAILS_BUTTON
                    } else {
                        layout::EIDOLONS_BUTTON
                    }),
                ]
            );
            assert_eq!(scanner.device.waits, vec![Duration::from_millis(500); 2]);
        }
    }

    #[test]
    fn character_tab_identity_drift_never_combines_pages() {
        let mut first = character_frame(Rgb([90, 120, 170]));
        let mut changed = first.clone();
        for y in 45..55 {
            for x in 90..110 {
                first.put_pixel(x, y, Rgb([240, 240, 240]));
            }
            for x in 140..160 {
                changed.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
        for reverse in [false, true] {
            let mut scanner =
                scanner_with_frames(vec![first.clone(), changed.clone()], ScanConfig::default());
            let error = scanner.read_character_pages(0, reverse).err().unwrap();
            assert_eq!(error.code(), "HSR-CHAR-PANEL-IDENTITY");
            assert_eq!(
                scanner.device.commands(),
                &[InputCommand::Click(layout::TRACES_BUTTON)]
            );
        }
    }

    #[test]
    fn cancellation_during_character_tabs_joins_worker_and_stops_input() {
        for reverse in [false, true] {
            let frame = character_frame(Rgb([90, 120, 170]));
            let mut scanner = scanner_with_frames(vec![frame; 3], ScanConfig::default());
            scanner.device.cancel_after_commands(1);
            let error = scanner.read_character_pages(0, reverse).err().unwrap();
            assert_eq!(error.code(), "HSR-SCAN-CANCELLED");
            assert_eq!(scanner.device.remaining_frames(), 2);
            assert_eq!(
                scanner.device.commands(),
                &[InputCommand::Click(layout::TRACES_BUTTON)]
            );
        }
    }

    #[test]
    fn character_repetition_without_expected_count_remains_unknown() {
        let first = character_frame(Rgb([90, 120, 170]));
        let visually_changed_same_id = character_frame(Rgb([170, 90, 120]));
        let frames = {
            let mut frames = Vec::new();
            frames.extend(std::iter::repeat_with(|| first.clone()).take(7));
            frames.extend(std::iter::repeat_with(|| visually_changed_same_id.clone()).take(40));
            frames
        };
        let scan = run_character_simulation(frames, &["三月七", "三月七"], 3);
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.coverage, CoverageLevel::Unknown);
    }

    #[test]
    fn stationary_roster_end_proves_completion_without_an_entered_total() {
        let first = character_frame(Rgb([90, 120, 170]));
        let second = character_frame(Rgb([170, 90, 120]));
        // Four reset captures + three tabs; subsequent entries need two
        // identity-change captures + three tabs, with no return to Details.
        let mut frames = vec![first.clone(); 7];
        frames.extend(vec![second.clone(); 5]);
        // Model overlap: every visible position is visited, but identities
        // already seen on a clamped page are exported only once.
        for slot in 2..9 {
            frames.extend(vec![
                if slot % 2 == 0 {
                    first.clone()
                } else {
                    second.clone()
                };
                5
            ]);
        }
        frames.extend(vec![first; 2]);
        let scan = run_character_simulation(
            frames,
            &[
                "三月七",
                "希儿",
                "三月七",
                "希儿",
                "三月七",
                "希儿",
                "三月七",
                "希儿",
                "三月七",
            ],
            0,
        );
        assert_eq!(scan.items.len(), 2);
        assert_eq!(scan.coverage, CoverageLevel::Complete);
    }

    #[test]
    fn short_roster_finishes_at_its_last_visible_portrait() {
        let mut frame = character_frame(Rgb([90, 120, 170]));
        paint_rect(
            &mut frame,
            layout::character_portrait(0),
            0.065,
            0.07,
            Rgb([230, 190, 80]),
        );
        assert_eq!(
            crate::vision::last_visible_character_selected(&frame),
            Some(true)
        );
        let scan = run_character_simulation(vec![frame; 14], &["三月七"], 0);
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.coverage, CoverageLevel::Complete);
    }

    #[test]
    fn moving_terminal_confirmation_does_not_claim_roster_end() {
        let before = character_frame(Rgb([90, 120, 170]));
        let mut moved = before.clone();
        paint_character_portraits(&mut moved);
        let mut scanner = scanner_with_frames(vec![before.clone(), moved], ScanConfig::default());
        let mut slot = 8;
        let error = scanner
            .advance_character(&before, &mut slot, 8)
            .unwrap_err();
        assert_eq!(error.code(), "HSR-CHAR-ADVANCE");
    }

    #[test]
    fn reset_retries_when_a_missed_drag_is_followed_by_a_moving_confirmation() {
        let before = character_frame(Rgb([90, 120, 170]));
        let mut moved = before.clone();
        paint_character_portraits(&mut moved);
        let mut scanner = scanner_with_frames(
            vec![
                before.clone(),
                before.clone(),
                before,
                moved.clone(),
                moved.clone(),
                moved,
            ],
            ScanConfig::default(),
        );
        scanner.reset_character_bar().unwrap();
        assert_eq!(scanner.device().remaining_frames(), 0);
        let (from, to) = layout::character_page_drag(false);
        assert_eq!(
            scanner.device().commands(),
            &[
                InputCommand::Hover(Point::new(0.85, 0.15)),
                InputCommand::Drag { from, to },
                InputCommand::Drag { from, to },
                InputCommand::Drag { from, to },
                InputCommand::Drag { from, to },
            ]
        );
    }

    #[test]
    fn inventory_caps_sample_each_category_without_rejecting_its_total() {
        for kind in [InventoryKind::LightCone, InventoryKind::Gear] {
            let grid = backpack_grid(kind);
            let frame = page_frame(&grid, 0, 3_000, Some(0));
            let snapshot: ReferenceSnapshot =
                serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json"))
                    .unwrap();
            let name_field = match kind {
                InventoryKind::LightCone => OcrField::LightConeName,
                InventoryKind::Gear => OcrField::GearName,
            };
            let title = match kind {
                InventoryKind::LightCone => "制胜的瞬间",
                InventoryKind::Gear => "过客的逢春木簪",
            };
            let reader = ScriptedOcrReader::default()
                .with(OcrField::MenuTitle, ["背包"])
                .with(name_field, [title])
                .with(OcrField::InventoryQuantity, ["3000/3000", "3000/3000"]);
            let config = ScanConfig {
                max_light_cones: usize::from(kind == InventoryKind::LightCone),
                max_gear: usize::from(kind == InventoryKind::Gear),
                ..ScanConfig::default()
            };
            let mut scanner = HsrScanner::new(
                ReplayDevice::new(1280, 720, vec![frame; 7]),
                reader,
                ReferenceCache::from_snapshot(snapshot).unwrap(),
                config,
            );
            let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = events.clone();
            scanner = scanner.with_observer(Box::new(move |category, event| {
                observed.lock().unwrap().push((category, event))
            }));
            let scan = scanner
                .scan_inventory(kind, |_, _, _, _, ordinal| Ok(ordinal))
                .unwrap();
            assert_eq!(scan.items, [0]);
            assert_eq!(scan.coverage, CoverageLevel::Unknown);
            assert!(events.lock().unwrap().iter().any(|(_, event)| matches!(
                event,
                crate::scan_progress::ScanEvent::Progress {
                    recognized: 1,
                    visited: 1,
                    total: Some(1)
                }
            )));
            assert!(!scanner
                .device()
                .commands()
                .contains(&InputCommand::Key('d')));
        }
    }

    #[test]
    fn inventory_ocr_policy_skips_unreadable_entries_or_stops_but_never_masks_navigation_failures()
    {
        for kind in [InventoryKind::LightCone, InventoryKind::Gear] {
            for stop in [false, true] {
                for code in ["HSR-OCR-TEST", "HSR-NAV-TEST"] {
                    let frame = page_frame(&backpack_grid(kind), 0, 3, Some(0));
                    let snapshot: ReferenceSnapshot = serde_json::from_str(include_str!(
                        "../tests/fixtures/reference_cache.json"
                    ))
                    .unwrap();
                    let name_field = if kind == InventoryKind::LightCone {
                        OcrField::LightConeName
                    } else {
                        OcrField::GearName
                    };
                    let title = if kind == InventoryKind::LightCone {
                        "制胜的瞬间"
                    } else {
                        "过客的逢春木簪"
                    };
                    let mut scanner = HsrScanner::new(
                        ReplayDevice::new(1280, 720, vec![frame; 7]),
                        ScriptedOcrReader::default()
                            .with(OcrField::MenuTitle, ["背包"])
                            .with(name_field, [title])
                            .with(OcrField::InventoryQuantity, ["3/2000", "3/2000"]),
                        ReferenceCache::from_snapshot(snapshot).unwrap(),
                        ScanConfig {
                            stop_on_failure: stop,
                            max_light_cones: 1,
                            max_gear: 1,
                            ..Default::default()
                        },
                    );
                    let result = scanner.scan_inventory::<usize>(kind, |_, _, _, _, _| {
                        Err(HsrError::new(
                            code,
                            hints::FOCUS_REQUIRED,
                            "OCR_POLICY_MARKER",
                        ))
                    });
                    if stop || code == "HSR-NAV-TEST" {
                        let error = result.err().expect("debug or navigation error must stop");
                        assert_eq!(error.code(), code);
                        assert!(error.to_string().contains("OCR_POLICY_MARKER"));
                    } else {
                        let result = result.unwrap();
                        assert!(result.items.is_empty());
                        assert_eq!(result.coverage, CoverageLevel::Unknown);
                    }
                }
            }
        }
    }

    #[test]
    fn user_stop_keeps_inventory_entries_only_when_requested_and_never_masks_focus_loss() {
        for kind in [InventoryKind::LightCone, InventoryKind::Gear] {
            let make = |cap, save_on_cancel| {
                let grid = backpack_grid(kind);
                let frame = page_frame(&grid, 0, 3, Some(0));
                let name_field = if kind == InventoryKind::LightCone {
                    OcrField::LightConeName
                } else {
                    OcrField::GearName
                };
                let title = if kind == InventoryKind::LightCone {
                    "制胜的瞬间"
                } else {
                    "过客的逢春木簪"
                };
                let reader = ScriptedOcrReader::default()
                    .with(OcrField::MenuTitle, ["背包"])
                    .with(name_field, [title])
                    .with(OcrField::InventoryQuantity, ["3/2000", "3/2000"]);
                let reference: ReferenceSnapshot =
                    serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json"))
                        .unwrap();
                HsrScanner::new(
                    ReplayDevice::new(1280, 720, vec![frame; 7]),
                    reader,
                    ReferenceCache::from_snapshot(reference).unwrap(),
                    ScanConfig {
                        max_light_cones: cap,
                        max_gear: cap,
                        save_on_cancel,
                        ..Default::default()
                    },
                )
            };
            let mut baseline = make(1, false);
            assert_eq!(
                baseline
                    .scan_inventory(kind, |_, _, _, _, ordinal| Ok(ordinal))
                    .unwrap()
                    .items,
                [0]
            );
            let stop_at = baseline.device().commands().len() - 1;
            for save in [false, true] {
                let mut scanner = make(0, save);
                scanner.device.cancel_after_commands(stop_at);
                let scan = scanner.scan_inventory(kind, |_, _, _, _, ordinal| Ok(ordinal));
                if save {
                    let scan = scan.unwrap();
                    assert_eq!(scan.items, [0]);
                    assert_eq!(scan.coverage, CoverageLevel::Unknown);
                } else {
                    assert_eq!(scan.unwrap_err().hint(), hints::CANCELLED);
                }
                assert_eq!(scanner.device().commands().len(), stop_at);
                assert!(matches!(
                    scanner.device().commands().last(),
                    Some(InputCommand::Key(_))
                ));
            }
            let mut scanner = make(0, true);
            scanner.device.lose_focus_after_commands(stop_at);
            assert_eq!(
                scanner
                    .scan_inventory(kind, |_, _, _, _, ordinal| Ok(ordinal))
                    .unwrap_err()
                    .hint(),
                hints::FOCUS_REQUIRED
            );
        }
    }

    #[test]
    fn user_stop_retains_complete_characters_and_skips_all_later_categories() {
        let make = |maximum, save_on_cancel| {
            let frame = character_frame(Rgb([90, 120, 170]));
            let reader = ScriptedOcrReader::default()
                .with(OcrField::MenuTitle, ["角色详情"])
                .with(OcrField::CharacterName, ["三月七"])
                .with(OcrField::CharacterLevel, ["等级 80/80"]);
            HsrScanner::new(
                ReplayDevice::new(1280, 720, with_character_menu_open(vec![frame; 64])),
                reader,
                two_character_references(),
                ScanConfig {
                    max_characters: maximum,
                    save_on_cancel,
                    timings: ScanTimings {
                        panel_timeout_ms: 40,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
        };
        let mut baseline = make(1, false);
        assert_eq!(baseline.scan_characters().unwrap().items.len(), 1);
        let stop_at = baseline.device().commands().len() - 1;
        let mut scanner = make(0, true);
        scanner.device.cancel_after_commands(stop_at);
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = events.clone();
        scanner = scanner.with_observer(Box::new(move |category, event| {
            recorded.lock().unwrap().push((category, event))
        }));
        let scan = scanner.scan().unwrap();
        assert_eq!(scan.observations.as_inner().characters.len(), 1);
        assert!(scan.observations.as_inner().light_cones.is_empty());
        assert!(scan.gear_items.is_empty());
        assert_eq!(scan.coverage.characters, CoverageLevel::Unknown);
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .all(|(category, _)| *category == crate::scan_progress::ScanCategory::Characters));
    }

    #[test]
    fn character_cap_before_another_stable_panel_remains_unknown() {
        let first = character_frame(Rgb([90, 120, 170]));
        let second = character_frame(Rgb([170, 90, 120]));
        let extra = character_frame(Rgb([90, 170, 120]));
        let frames = {
            let mut frames = Vec::new();
            frames.extend(std::iter::repeat_with(|| first.clone()).take(7));
            frames.extend(std::iter::repeat_with(|| second.clone()).take(5));
            frames.extend(std::iter::repeat_with(|| extra.clone()).take(2));
            frames
        };
        let scan = run_character_simulation(frames, &["三月七", "希儿"], 2);
        assert_eq!(scan.items.len(), 2);
        assert_eq!(scan.coverage, CoverageLevel::Unknown);
    }

    #[test]
    fn repeating_first_character_does_not_prove_mouse_roster_completion() {
        let first = character_frame(Rgb([90, 120, 170]));
        let second = character_frame(Rgb([170, 90, 120]));
        let frames = {
            let mut frames = Vec::new();
            frames.extend(std::iter::repeat_with(|| first.clone()).take(7));
            frames.extend(std::iter::repeat_with(|| second.clone()).take(5));
            frames.extend(std::iter::repeat_with(|| first.clone()).take(40));
            frames
        };
        let scan = run_character_simulation(frames, &["三月七", "希儿", "三月七"], 0);
        assert_eq!(scan.items.len(), 2);
        assert_eq!(scan.coverage, CoverageLevel::Unknown);
    }

    #[test]
    fn postverify_rejects_collateral_change_to_known_unrequested_flag() {
        let before = ManagedState {
            lock: Some(false),
            discard: Some(false),
        };
        let expected = ManagedState {
            lock: Some(true),
            discard: Some(false),
        };
        let collateral = ManagedState {
            lock: Some(true),
            discard: Some(true),
        };
        assert!(unrequested_known_field_unchanged(
            &before,
            &expected,
            ManagedField::Lock
        ));
        assert!(!unrequested_known_field_unchanged(
            &before,
            &collateral,
            ManagedField::Lock
        ));

        let unknown_before = ManagedState {
            lock: Some(false),
            discard: None,
        };
        assert!(unrequested_known_field_unchanged(
            &unknown_before,
            &collateral,
            ManagedField::Lock
        ));
    }

    fn parsed_manager_gear(icon_confidence: f64) -> ParsedGearPanel {
        let snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let references = ReferenceCache::from_snapshot(snapshot).unwrap();
        ParsedGearPanel {
            observation: crate::model::ObservedGear {
                piece_id: 61011,
                level: 15,
                main_stat_key: "HPDelta".to_string(),
                main_stat_value: 705.0,
                substats: Vec::new(),
                equipped_character_id: None,
                lock: Some(true),
                discard: Some(false),
            },
            reference: references.gear(61011).unwrap().clone(),
            equipped: Some(false),
            location_key: None,
            icon_confidence,
        }
    }

    #[test]
    fn managed_icon_confidence_fails_closed_below_boundary_and_accepts_exact_boundary() {
        let below = managed_observation(&parsed_manager_gear(
            MANAGED_ICON_CONFIDENCE_THRESHOLD - 0.000_001,
        ));
        assert_eq!(below.state.lock, None);
        assert_eq!(below.state.discard, None);
        assert_eq!(below.equipped, Some(false));

        let boundary = managed_observation(&parsed_manager_gear(MANAGED_ICON_CONFIDENCE_THRESHOLD));
        assert_eq!(boundary.state.lock, Some(true));
        assert_eq!(boundary.state.discard, Some(false));

        let invalid = managed_observation(&parsed_manager_gear(f64::NAN));
        assert_eq!(invalid.state.lock, None);
        assert_eq!(invalid.state.discard, None);
    }

    #[test]
    fn direct_manager_trait_refuses_locked_or_unknown_discard_without_device_input() {
        let frame = RgbImage::from_pixel(1280, 720, Rgb([24, 28, 35]));

        for (lock, observed_detail) in [
            (Some(true), "observedLock=Some(true)"),
            (None, "observedLock=None"),
        ] {
            let mut target = managed_observation(&parsed_manager_gear(1.0));
            target.state.lock = lock;
            let domain_error =
                ensure_mutation_lock_evidence(MutationScope::MarkDiscard, &target.state)
                    .expect_err("locked or unknown lock evidence must reject discard marking");
            assert_eq!(
                domain_error.code(),
                "HSR-MANAGER-MARK-DISCARD-LOCK-EVIDENCE"
            );
            assert!(domain_error
                .localized_message(Language::En)
                .contains("Discard marking was refused"));
            assert!(domain_error
                .localized_message(Language::ZhCn)
                .contains("已拒绝标记弃置"));

            let mut scanner = scanner_with_frames(vec![frame.clone()], ScanConfig::default());
            let error = ManagerMutationDevice::toggle_once(
                &mut scanner,
                &target,
                MutationScope::MarkDiscard,
            )
            .expect_err("the public mutation trait must fail before touching the device");

            assert!(error.contains("[HSR-MANAGER-MARK-DISCARD-LOCK-EVIDENCE]"));
            assert!(error.contains(observed_detail));
            assert!(error.contains("no status input was issued"));
            assert!(scanner.device().commands().is_empty());
            assert_eq!(
                scanner.device().remaining_frames(),
                1,
                "the adapter guard must run before even capturing from the device"
            );
        }
    }

    #[test]
    fn immediate_preclick_recapture_rejects_post_parse_screen_drift_without_clicking() {
        let grid = test_grid();
        let panel = StatsPanelLayout::MAINTAINED_SEED;
        let mut parsed_frame = page_frame(&grid, 0, 2, Some(0));
        let lock = panel.lock_button(InventoryKind::Gear);
        let discard = panel.discard_button();
        paint_rect(
            &mut parsed_frame,
            lock.center(),
            lock.width,
            lock.height,
            Rgb([220, 170, 70]),
        );
        paint_rect(
            &mut parsed_frame,
            discard.center(),
            discard.width,
            discard.height,
            Rgb([220, 220, 220]),
        );
        let mut drifted = parsed_frame.clone();
        let changed_panel = panel.panel.relative(NormRect::new(0.10, 0.40, 0.45, 0.20));
        paint_rect(
            &mut drifted,
            changed_panel.center(),
            changed_panel.width,
            changed_panel.height,
            Rgb([175, 85, 120]),
        );

        let parsed = parsed_manager_gear(1.0);
        let target = managed_observation(&parsed);
        let selected = SelectedGearContext {
            parsed,
            panel,
            selection: SelectedCard {
                column: 0,
                y: grid.first().unwrap().y,
            },
            grid,
            parsed_frame,
        };
        let mut scanner = scanner_with_frames(vec![drifted], ScanConfig::default());
        let error = scanner
            .recapture_and_validate_pre_click(&selected, &target, ManagedField::Lock)
            .unwrap_err();
        assert_eq!(error.code(), "HSR-MANAGER-PRECLICK-DRIFT");
        assert!(scanner.device().commands().is_empty());
    }

    #[test]
    fn plausible_quantity_underread_never_claims_complete() {
        let visible_extra = assess_inventory_completion(InventoryCompletionEvidence {
            primary_quantity: 40,
            confirmation_quantity: 40,
            terminal_quantity: Some(40),
            next_cell: NextCellEvidence::ExtraItem,
            terminal_scrolls: [ScrollEvidence::End { confidence: 1.0 }; 2],
            scrollbar_bottom_confidence: Some(1.0),
        });
        assert_eq!(visible_extra.coverage, CoverageLevel::Unknown);

        let later_page = assess_inventory_completion(InventoryCompletionEvidence {
            primary_quantity: 45,
            confirmation_quantity: 45,
            terminal_quantity: Some(45),
            next_cell: NextCellEvidence::NoCandidate,
            terminal_scrolls: [
                ScrollEvidence::Advanced {
                    rows: 1,
                    confidence: 1.0,
                },
                ScrollEvidence::End { confidence: 1.0 },
            ],
            scrollbar_bottom_confidence: Some(1.0),
        });
        assert_eq!(later_page.coverage, CoverageLevel::Unknown);
    }

    #[test]
    fn plausible_quantity_overread_or_disagreement_never_claims_complete() {
        let assessment = assess_inventory_completion(InventoryCompletionEvidence {
            primary_quantity: 50,
            confirmation_quantity: 49,
            terminal_quantity: Some(49),
            next_cell: NextCellEvidence::StayedOnLast,
            terminal_scrolls: [ScrollEvidence::End { confidence: 1.0 }; 2],
            scrollbar_bottom_confidence: Some(1.0),
        });
        assert_eq!(assessment.coverage, CoverageLevel::Unknown);
        assert!(assessment.reason.contains("quantity reads disagreed"));
    }

    #[test]
    fn matching_quantity_and_terminal_controller_proof_claim_complete() {
        let grid = test_grid();
        let terminal = page_frame(&grid, 0, 2, Some(1));
        let frames = vec![
            terminal.clone(),
            terminal.clone(),
            terminal.clone(),
            terminal.clone(),
            terminal.clone(),
            terminal.clone(),
        ];
        let snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let references = ReferenceCache::from_snapshot(snapshot).unwrap();
        let config = ScanConfig {
            timings: ScanTimings {
                panel_timeout_ms: 40,
                ..ScanTimings::default()
            },
            inventory_scroll_ticks_per_page: 5,
            scroll_tick_delay: Duration::from_millis(1),
            ..ScanConfig::default()
        };
        let mut scanner = HsrScanner::new(
            ReplayDevice::new(1280, 720, frames),
            ScriptedOcrReader::default().with(OcrField::InventoryQuantity, ["2/2000"]),
            references,
            config,
        );
        let mut cursor = InventoryCursor::new(2, &grid).unwrap();
        cursor.mark_selected(0).unwrap();
        cursor.mark_selected(1).unwrap();
        let mut session = InventorySession {
            cursor,
            grid: grid.clone(),
            panel: StatsPanelLayout::MAINTAINED_SEED,
            frame: terminal,
            quantity_reads: [2, 2],
            walk: InventoryWalk::default(),
        };

        let assessment = scanner.confirm_inventory_completion(&mut session).unwrap();
        assert_eq!(assessment.coverage, CoverageLevel::Complete);
        assert_eq!(
            scanner.device().commands(),
            &[
                InputCommand::Click(grid.cell(2).unwrap()),
                InputCommand::Scroll(1),
                InputCommand::Scroll(1)
            ]
        );
    }

    #[test]
    fn page_boundary_with_ignored_scroll_and_nonterminal_thumb_is_never_complete() {
        let grid = test_grid();
        // OCR plausibly under-read exactly one visible page (45) while the
        // sanitized frame models a longer 90-item inventory. There is no next
        // derived cell, and both ignored wheel events leave pixels unchanged.
        let nonterminal = page_frame(&grid, 0, 90, Some(44));
        let frames = vec![
            nonterminal.clone(),
            nonterminal.clone(),
            nonterminal.clone(),
            nonterminal.clone(),
        ];
        let snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let references = ReferenceCache::from_snapshot(snapshot).unwrap();
        let config = ScanConfig {
            timings: ScanTimings {
                panel_timeout_ms: 40,
                ..ScanTimings::default()
            },
            inventory_scroll_ticks_per_page: 5,
            scroll_tick_delay: Duration::from_millis(1),
            ..ScanConfig::default()
        };
        let mut scanner = HsrScanner::new(
            ReplayDevice::new(1280, 720, frames),
            ScriptedOcrReader::default().with(OcrField::InventoryQuantity, ["45/2000"]),
            references,
            config,
        );
        let mut cursor = InventoryCursor::new(45, &grid).unwrap();
        for ordinal in 0..45 {
            cursor.mark_selected(ordinal).unwrap();
        }
        let mut session = InventorySession {
            cursor,
            grid: grid.clone(),
            panel: StatsPanelLayout::MAINTAINED_SEED,
            frame: nonterminal,
            quantity_reads: [45, 45],
            walk: InventoryWalk::default(),
        };

        let assessment = scanner.confirm_inventory_completion(&mut session).unwrap();
        assert_eq!(assessment.coverage, CoverageLevel::Unknown);
        assert!(assessment.reason.contains("scrollbar thumb"));
        assert_eq!(
            scanner.device().commands(),
            &[InputCommand::Scroll(1), InputCommand::Scroll(1)]
        );
    }

    #[test]
    fn ambiguous_character_is_omitted_and_coverage_is_unknown() {
        let frame = RgbImage::from_pixel(1280, 720, Rgb([24, 28, 35]));
        let mut snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let mut variant = snapshot.characters[0].clone();
        variant.game_id = 1224;
        variant.key = "1224".to_string();
        variant.path = "Memory".to_string();
        snapshot.characters.push(variant);
        let references = ReferenceCache::from_snapshot(snapshot).unwrap();
        let config = ScanConfig {
            timings: ScanTimings {
                panel_timeout_ms: 40,
                ..ScanTimings::default()
            },
            max_characters: 1,
            ..ScanConfig::default()
        };
        for stop in [false, true] {
            let mut scanner = HsrScanner::new(
                ReplayDevice::new(1280, 720, vec![frame.clone(); 24]),
                ScriptedOcrReader::default()
                    .with(OcrField::MenuTitle, ["角色详情"])
                    .with(OcrField::CharacterName, ["三月七"]),
                references.clone(),
                ScanConfig {
                    stop_on_failure: stop,
                    ..config.clone()
                },
            );
            let scan = scanner.scan_characters();
            if stop {
                assert_eq!(
                    scan.err().expect("strict OCR policy must stop").code(),
                    "HSR-OCR-CHARACTER-AMBIGUOUS"
                );
            } else {
                let scan = scan.unwrap();
                assert!(scan.items.is_empty());
                assert_eq!(scan.coverage, CoverageLevel::Unknown);
            }
        }
    }
}
