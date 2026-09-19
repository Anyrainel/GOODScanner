use std::{collections::BTreeSet, time::Duration};

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
        ObservedCharacter, ObservedLightCone, OBSERVATION_SCHEMA_VERSION,
    },
    observation::ValidatedObservationSnapshot,
    ocr::{
        detect_discard_state, detect_icon_state, InventoryKind, OcrField, OcrReader, PaddleOcrReader,
        PanelParser, ParsedGearPanel, StatsPanelLayout, MANAGED_ICON_CONFIDENCE_THRESHOLD,
    },
    reference::ReferenceCache,
    vision::{
        classify_inventory_scroll, discover_inventory_grid, frame_fingerprint, frames_similar,
        inventory_scrollbar_bottom_confidence, selected_cell, GridGeometry, NormRect,
        ScrollEvidence,
    },
};

const CHARACTER_IDENTITY_REGION: NormRect = NormRect::new(0.045, 0.035, 0.84, 0.25);

/// kel-z/HSR-Scanner menu timings, added on top of `navigation_delay`.
const MENU_TRANSITION: Duration = Duration::from_millis(1_000);
const INVENTORY_OPEN: Duration = Duration::from_millis(1_500);
const TAB_SWITCH: Duration = Duration::from_millis(1_500);
const DETAILS_OPEN: Duration = Duration::from_millis(500);

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
    pub capture_method: CaptureMethod,
    pub navigation_delay: Duration,
    pub panel_timeout: Duration,
    /// Baseline wheel detents for one full visible inventory page. The runtime
    /// scales this down for a clamped final-page row advance and verifies the
    /// resulting row displacement from screenshots before scanning continues.
    pub inventory_scroll_ticks_per_page: usize,
    pub scroll_tick_delay: Duration,
    pub max_inventory_items: usize,
    /// Stop after this many parsed inventory entries. `None` walks the OCR
    /// quantity. Used by dump sessions so a large backpack can still be sampled.
    pub scan_item_limit: Option<usize>,
    pub max_characters: usize,
    pub expected_characters: Option<usize>,
    /// Configurable because current public evidence uses virtual-controller RB
    /// for roster traversal. The default `e` is not claimed live-proven until
    /// the device calibration step succeeds.
    pub next_character_key: char,
    /// GOODScanner-style OCR dump. When true, crops and full frames are written
    /// under `debug_images/` as a side effect of parsing; click/wait code is
    /// unchanged.
    pub dump_images: bool,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            targets: ScanTargets::all(),
            capture_method: CaptureMethod::Wgc,
            navigation_delay: Duration::from_millis(250),
            panel_timeout: Duration::from_millis(900),
            inventory_scroll_ticks_per_page: 25,
            scroll_tick_delay: Duration::from_millis(10),
            max_inventory_items: 4_000,
            scan_item_limit: None,
            max_characters: 200,
            expected_characters: None,
            next_character_key: 'e',
            dump_images: false,
        }
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
}

#[derive(Debug, Clone, PartialEq)]
struct InventoryScan<T> {
    items: Vec<T>,
    coverage: CoverageLevel,
}

pub struct HsrScanner<D, R> {
    device: D,
    parser: PanelParser<R>,
    references: ReferenceCache,
    config: ScanConfig,
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
        let device = WindowsHsrDevice::locate_with_cancel(config.capture_method, cancel)?;
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

    /// Capture a fresh, complete gear inventory for manager preview. The
    /// ordinal is deliberately discarded: manager identity is the complete
    /// visible matcher, never scan order.
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
                "manager preview requires redundant quantity reads, terminal-grid proof, and bottom-of-inventory proof; gear coverage remained unknown",
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

        let character_coverage = if self.config.targets.characters {
            let scan = self.scan_characters()?;
            characters = scan.items;
            scan.coverage
        } else {
            CoverageLevel::Unknown
        };
        let light_cone_coverage = if self.config.targets.light_cones {
            let scan = self.scan_light_cones()?;
            light_cones = scan.items;
            scan.coverage
        } else {
            CoverageLevel::Unknown
        };
        let gear_coverage = if self.config.targets.gear {
            let scan = self.scan_gear()?;
            gear_items = scan.items;
            scan.coverage
        } else {
            CoverageLevel::Unknown
        };

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
        Ok(ScanResult {
            observations: ValidatedObservationSnapshot::from_screen_capture(snapshot)?,
            gear_items,
            coverage,
        })
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
        let mut session = self.enter_inventory(kind)?;
        let mut items = Vec::with_capacity(session.cursor.quantity());
        let mut incomplete_reason = None;
        loop {
            let ordinal = match self.advance_inventory_session(&mut session) {
                Ok(Some(ordinal)) => ordinal,
                Ok(None) => break,
                Err(error) if is_inventory_boundary_uncertainty(error.code()) => {
                    incomplete_reason = Some(format!(
                        "quantity-bound traversal stopped after {} entries; cause={error}",
                        items.len()
                    ));
                    break;
                },
                Err(error) => return Err(error),
            };
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
                Ok(item) => {
                    items.push(item);
                    if self
                        .config
                        .scan_item_limit
                        .is_some_and(|limit| items.len() >= limit)
                    {
                        incomplete_reason = Some(format!(
                            "scan_item_limit={} reached after {} entries",
                            self.config.scan_item_limit.unwrap(),
                            items.len()
                        ));
                        break;
                    }
                },
                Err(error) if is_omittable_ambiguity(error.code()) => {
                    incomplete_reason.get_or_insert_with(|| {
                        format!(
                            "one or more visible entries were omitted because public reference identity was ambiguous; firstCause={error}"
                        )
                    });
                    yas::log_warn!(
                        "一个可见库存条目无法唯一匹配公开参考 ID，已安全省略；覆盖率将标记为未知。完整错误详情：{}",
                        "A visible inventory entry could not be mapped to one unique public reference ID and was safely omitted; coverage will be marked unknown. Full error details: {}",
                        error
                    );
                },
                Err(error) => return Err(error),
            }
        }

        let coverage = if let Some(reason) = incomplete_reason {
            log_inventory_coverage_warning(kind, items.len(), &reason);
            CoverageLevel::Unknown
        } else {
            let assessment = self.confirm_inventory_completion(&mut session)?;
            if assessment.coverage != CoverageLevel::Complete {
                log_inventory_coverage_warning(kind, items.len(), assessment.reason);
            }
            assessment.coverage
        };
        self.leave_menu()?;
        Ok(InventoryScan { items, coverage })
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
        while let Some(current) = self.advance_inventory_session(&mut session)? {
            if current == ordinal {
                let parsed =
                    self.parser
                        .parse_gear(&session.frame, session.panel, &self.references)?;
                let cell_index = session.cursor.last_selected_cell().ok_or_else(|| {
                    HsrError::new(
                        "HSR-MANAGER-SELECTION-DRIFT",
                        hints::SCREEN_INVALID,
                        "selected ordinal had no derived visible grid cell",
                    )
                })?;
                return Ok(SelectedGearContext {
                    parsed,
                    panel: session.panel,
                    grid: session.grid,
                    cell_index,
                    parsed_frame: session.frame,
                });
            }
        }
        Err(HsrError::new(
            "HSR-MANAGER-ORDINAL-DRIFT",
            hints::SCREEN_INVALID,
            format!("inventory ended before selected ordinal={ordinal}"),
        ))
    }

    fn enter_inventory(&mut self, kind: InventoryKind) -> HsrResult<InventorySession> {
        self.prepare_menu_focus()?;
        self.issue_input(InputCommand::Escape)?;
        self.wait_attended(self.config.navigation_delay + MENU_TRANSITION)?;
        self.issue_input(InputCommand::Key('b'))?;
        self.wait_attended(self.config.navigation_delay + INVENTORY_OPEN)?;

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
            self.wait_attended(self.config.navigation_delay + TAB_SWITCH)?;
            let before = self.capture_stable()?;
            let grid = match discover_inventory_grid(&before) {
                Ok(grid) => grid,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                },
            };
            let Some(first) = grid.first() else {
                continue;
            };
            self.issue_input(InputCommand::Click(first))?;
            self.wait_attended(self.config.navigation_delay + DETAILS_OPEN)?;
            let selected = self.capture_stable()?;
            if selected_cell(&grid, &selected).map(|entry| entry.0) != Some(0) {
                last_error = Some(HsrError::new(
                    "HSR-GRID-SELECTION",
                    hints::SCREEN_INVALID,
                    "first-card click did not produce an exact selected-card border at cell=0",
                ));
                continue;
            }
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
                    self.wait_attended(Duration::from_millis(30))?;
                    let confirmation_frame = self.device.capture_client()?;
                    annotator::add_image("confirmation", &confirmation_frame);
                    if selected_cell(&grid, &confirmation_frame).map(|entry| entry.0) != Some(0)
                        || !frames_similar(
                            &panel.immutable_panel().crop(&selected)?,
                            &panel.immutable_panel().crop(&confirmation_frame)?,
                        )
                    {
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
        let first = text.split(['/', '／']).next().unwrap_or_default();
        let digits: String = first.chars().filter(char::is_ascii_digit).collect();
        digits.parse().map_err(|error| {
            HsrError::new(
                "HSR-SCAN-QUANTITY",
                hints::OCR_FAILED,
                format!("inventory quantity OCR did not yield an integer; cause={error}"),
            )
        })
    }

    fn validate_inventory_quantity(&self, quantity: usize, read: &str) -> HsrResult<()> {
        if quantity == 0 || quantity > self.config.max_inventory_items {
            return Err(HsrError::new(
                "HSR-SCAN-QUANTITY",
                hints::OCR_FAILED,
                format!(
                    "{read} inventory quantity={quantity}; allowed range=1..={}",
                    self.config.max_inventory_items
                ),
            ));
        }
        Ok(())
    }

    fn parser_reader_mut(&mut self) -> &mut R {
        self.parser.reader_mut()
    }

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

    fn select_inventory_cell(
        &mut self,
        grid: &GridGeometry,
        panel: StatsPanelLayout,
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
        self.wait_attended(Duration::from_millis(18))?;

        let deadline_steps = (self.config.panel_timeout.as_millis() / 20).max(2) as usize;
        let mut previous_panel = None;
        let mut selected_seen = false;
        for _ in 0..deadline_steps {
            self.ensure_attended()?;
            let frame = self.device.capture_client()?;
            if selected_cell(grid, &frame).map(|entry| entry.0) == Some(cell_index) {
                selected_seen = true;
                let panel_crop = panel.immutable_panel().crop(&frame)?;
                if previous_panel
                    .as_ref()
                    .is_some_and(|previous| frames_similar(previous, &panel_crop))
                {
                    return Ok(frame);
                }
                previous_panel = Some(panel_crop);
            }
            self.wait_attended(Duration::from_millis(20))?;
        }
        Err(HsrError::new(
            "HSR-SCAN-SELECTION",
            hints::SCREEN_INVALID,
            format!(
                "cell click was not postvalidated; ordinal={ordinal}, cell={cell_index}, selectedSeen={selected_seen}"
            ),
        ))
    }

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

    fn issue_inventory_scroll(&mut self, grid: &GridGeometry, rows: usize) -> HsrResult<RgbImage> {
        let ticks = self.inventory_scroll_ticks(rows, grid.rows())?;
        for _ in 0..ticks {
            self.issue_input(InputCommand::Scroll(1))?;
            self.wait_attended(self.config.scroll_tick_delay)?;
        }
        self.capture_stable()
    }

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
                    None
                },
            },
            Err(error) => {
                yas::log_warn!(
                    "无法在库存末端再次读取数量；覆盖率将标记为未知，管理器不会执行变更。完整错误详情：{}",
                    "The inventory quantity could not be read again at the terminal boundary; coverage will be marked unknown and the manager will not mutate. Full error details: {}",
                    error
                );
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
        self.wait_attended(Duration::from_millis(18))?;
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
        self.prepare_menu_focus()?;
        self.issue_input(InputCommand::Escape)?;
        self.wait_attended(self.config.navigation_delay + MENU_TRANSITION)?;
        self.issue_input(InputCommand::Key('c'))?;
        self.wait_attended(self.config.navigation_delay + INVENTORY_OPEN)?;
        self.issue_input(InputCommand::Click(layout::DETAILS_BUTTON))?;
        self.wait_attended(self.config.navigation_delay + DETAILS_OPEN)?;

        let limit = self
            .config
            .expected_characters
            .unwrap_or(self.config.max_characters)
            .min(self.config.max_characters);
        let mut items = Vec::new();
        let mut seen = BTreeSet::new();
        let mut terminal_proven = false;
        let mut coverage_degraded = false;
        let mut visited = 0_usize;
        let mut details = self.capture_stable()?;
        let first_details = details.clone();
        for index in 0..limit {
            visited = index + 1;
            let eidolon = self.read_eidolon_count()?;
            self.issue_input(InputCommand::Click(layout::DETAILS_BUTTON))?;
            self.wait_attended(self.config.navigation_delay + DETAILS_OPEN)?;
            let parsed = match dump_parsed_item("characters", index, &details, || {
                self.parser
                    .parse_character_details(&details, &self.references, eidolon)
            }) {
                Ok(parsed) => Some(parsed),
                Err(error) if error.code() == "HSR-OCR-CHARACTER-AMBIGUOUS" => {
                    coverage_degraded = true;
                    yas::log_warn!(
                        "当前角色名称对应多个公开角色模板，且画面没有足够的命途或变体证据；已省略该角色并将角色覆盖率标记为未知。完整错误详情：{}",
                        "The current character name maps to multiple public templates and the screen lacks sufficient path or variant evidence; the entry was omitted and character coverage is marked unknown. Full error details: {}",
                        error
                    );
                    None
                },
                Err(error) => return Err(error),
            };
            if let Some(parsed) = parsed {
                if !seen.insert(parsed.observation.character_id) {
                    // Repetition is a safe stopping hint, but without an
                    // explicit expected count it cannot prove that no distinct
                    // panel was skipped by navigation or OCR.
                    break;
                }
                items.push(parsed.observation);
            }
            if self.config.expected_characters == Some(visited) {
                terminal_proven = self.probe_character_wrap(&details, &first_details, index)?;
                if !terminal_proven {
                    yas::log_warn!(
                        "角色数量已达到预期值，但下一张稳定角色面板未回到首个角色；覆盖率将标记为未知。",
                        "The expected Character count was reached, but the next stable panel did not wrap to the first Character; coverage will be marked unknown."
                    );
                }
                break;
            }
            if index + 1 == limit {
                break;
            }
            details = self.advance_character(&details, index)?;
        }
        self.leave_menu()?;
        if let Some(expected) = self.config.expected_characters {
            if visited != expected {
                return Err(HsrError::new(
                    "HSR-CHAR-COUNT",
                    hints::SCREEN_INVALID,
                    format!(
                        "expected {expected} character panels but visited {visited} and resolved {}",
                        items.len(),
                    ),
                ));
            }
        }
        let coverage = if terminal_proven && !coverage_degraded {
            CoverageLevel::Complete
        } else {
            CoverageLevel::Unknown
        };
        yas::log_info!(
            "角色扫描完成：{} 名。",
            "Character scan complete: {} entries.",
            items.len()
        );
        Ok(CharacterScan { items, coverage })
    }

    fn probe_character_wrap(
        &mut self,
        current: &RgbImage,
        first: &RgbImage,
        ordinal: usize,
    ) -> HsrResult<bool> {
        let current_identity = CHARACTER_IDENTITY_REGION.crop(current)?;
        let first_identity = CHARACTER_IDENTITY_REGION.crop(first)?;
        if frames_similar(&current_identity, &first_identity) {
            // A single visible identity cannot distinguish a real one-entry
            // wrap from an ignored next-character command.
            return Ok(false);
        }
        match self.advance_character(current, ordinal) {
            Ok(next) => Ok(frames_similar(
                &first_identity,
                &CHARACTER_IDENTITY_REGION.crop(&next)?,
            )),
            Err(error) if error.code() == "HSR-CHAR-ADVANCE" => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn advance_character(&mut self, previous: &RgbImage, ordinal: usize) -> HsrResult<RgbImage> {
        let previous_identity = CHARACTER_IDENTITY_REGION.crop(previous)?;
        self.issue_input(InputCommand::Key(self.config.next_character_key))?;
        self.wait_attended(Duration::from_millis(18))?;

        let deadline_steps = (self.config.panel_timeout.as_millis() / 20).max(2) as usize;
        let mut changed_frame: Option<RgbImage> = None;
        let mut changed_identity = None;
        for _ in 0..deadline_steps {
            self.ensure_attended()?;
            let frame = self.device.capture_client()?;
            let identity = CHARACTER_IDENTITY_REGION.crop(&frame)?;
            if !frames_similar(&previous_identity, &identity) {
                if changed_identity
                    .as_ref()
                    .is_some_and(|prior| frames_similar(prior, &identity))
                {
                    return Ok(frame);
                }
                changed_frame = Some(frame);
                changed_identity = Some(identity);
            }
            self.wait_attended(Duration::from_millis(20))?;
        }
        let detail = if changed_frame.is_some() {
            "identity region changed but did not stabilize"
        } else {
            "identity region never changed; configured next-character input may not be mapped"
        };
        Err(HsrError::new(
            "HSR-CHAR-ADVANCE",
            hints::SCREEN_INVALID,
            format!("failed after character ordinal={ordinal}; {detail}"),
        ))
    }

    fn read_eidolon_count(&mut self) -> HsrResult<u8> {
        self.issue_input(InputCommand::Click(layout::EIDOLONS_BUTTON))?;
        self.wait_attended(self.config.navigation_delay + DETAILS_OPEN)?;
        let frame = self.capture_stable()?;
        let positions = [
            (0.32724, 0.17778),
            (0.53368, 0.16574),
            (0.78041, 0.35741),
            (0.66635, 0.80069),
            (0.41729, 0.77153),
            (0.18408, 0.68819),
        ];
        let mut unlocked = 0_u8;
        for (x, y) in positions {
            let crop = NormRect::new(x - 0.025, y - 0.04, 0.05, 0.08).crop(&frame)?;
            let variance = luma_variance(&crop);
            let gold = gold_ratio(&crop);
            if variance < 360.0 {
                break;
            }
            // An unactivated-but-available Eidolon is strongly gold; it is not
            // counted as activated. White/high-detail nodes are active.
            if gold > 0.22 {
                break;
            }
            unlocked += 1;
        }
        Ok(unlocked)
    }

    fn capture_stable(&mut self) -> HsrResult<RgbImage> {
        self.ensure_attended()?;
        // Menu transition waits already ran. Take two frames; HSR's 3D preview
        // and starfield never go fully still, so do not poll until timeout.
        let first = self.device.capture_client()?;
        self.wait_attended(Duration::from_millis(80))?;
        self.ensure_attended()?;
        let second = self.device.capture_client()?;
        if chrome_settled(&first, &second) {
            return Ok(second);
        }
        yas::log_warn!(
            "菜单画面在等待后仍有动画，将使用最后一帧继续扫描。",
            "The menu was still animating after the wait; continuing with the last frame."
        );
        Ok(second)
    }

    /// Click a non-interactive corner so the client actually receives keys.
    /// Center clicks can confirm nearby interact prompts.
    fn prepare_menu_focus(&mut self) -> HsrResult<()> {
        self.issue_input(InputCommand::Click(crate::vision::Point::new(0.12, 0.82)))?;
        self.wait_attended(Duration::from_millis(120))
    }

    fn leave_menu(&mut self) -> HsrResult<()> {
        self.issue_input(InputCommand::Escape)?;
        self.wait_attended(self.config.navigation_delay + MENU_TRANSITION)?;
        self.issue_input(InputCommand::Escape)?;
        self.wait_attended(self.config.navigation_delay + MENU_TRANSITION)
    }

    fn ensure_attended(&self) -> HsrResult<()> {
        if self.device.is_cancelled() {
            return Err(HsrError::new(
                "HSR-SCAN-CANCELLED",
                hints::CANCELLED,
                "cancellation became active during an attended scanner operation",
            ));
        }
        if !self.device.is_foreground() {
            return Err(HsrError::new(
                "HSR-SCAN-FOCUS",
                hints::FOCUS_REQUIRED,
                "foreground ownership was lost during an attended scanner operation",
            ));
        }
        Ok(())
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
    annotator::begin_item(category, 0);
    annotator::add_image("full", frame);
}

fn dump_parsed_item<T: std::fmt::Debug>(
    category: &str,
    index: usize,
    frame: &RgbImage,
    parse: impl FnOnce() -> HsrResult<T>,
) -> HsrResult<T> {
    annotator::begin_item(category, index);
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
        .all(|rect| {
            match (rect.crop(left), rect.crop(right)) {
                (Ok(left_crop), Ok(right_crop)) => frames_similar(&left_crop, &right_crop),
                _ => false,
            }
        })
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
        self.wait_attended(self.config.navigation_delay)?;
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

        let selected_index = selected_cell(&selected.grid, &frame).map(|entry| entry.0);
        let before_fingerprint =
            frame_fingerprint(&selected.parsed_frame, selected.panel.immutable_panel())?;
        let current_fingerprint = frame_fingerprint(&frame, selected.panel.immutable_panel())?;
        if selected_index != Some(selected.cell_index) || before_fingerprint != current_fingerprint
        {
            return Err(HsrError::new(
                "HSR-MANAGER-PRECLICK-DRIFT",
                hints::SCREEN_INVALID,
                format!(
                    "immediate pre-click frame did not preserve selected cell and immutable panel fingerprint; expectedCell={}, actualCell={selected_index:?}",
                    selected.cell_index
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
    cell_index: usize,
    parsed_frame: RgbImage,
}

struct InventorySession {
    cursor: InventoryCursor,
    grid: GridGeometry,
    panel: StatsPanelLayout,
    frame: RgbImage,
    quantity_reads: [usize; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NextCellEvidence {
    NoCandidate,
    StayedOnLast,
    ExtraItem,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct InventoryCompletionEvidence {
    primary_quantity: usize,
    confirmation_quantity: usize,
    terminal_quantity: Option<usize>,
    next_cell: NextCellEvidence,
    terminal_scrolls: [ScrollEvidence; 2],
    scrollbar_bottom_confidence: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InventoryCompletionAssessment {
    coverage: CoverageLevel,
    reason: &'static str,
}

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

fn is_inventory_boundary_uncertainty(code: &str) -> bool {
    matches!(
        code,
        "HSR-SCAN-EARLY-END"
            | "HSR-SCAN-SELECTION"
            | "HSR-SCAN-SCROLL-DISTANCE"
            | "HSR-SCAN-SCROLL-UNCERTAIN"
            | "HSR-GRID-BOUNDS"
    )
}

fn is_omittable_ambiguity(code: &str) -> bool {
    matches!(code, "HSR-OCR-GEAR-AMBIGUOUS" | "HSR-OCR-STAT-AMBIGUOUS")
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
    columns: usize,
    visible_rows: usize,
    visible_start_row: usize,
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

    fn last_selected_cell(&self) -> Option<usize> {
        let ordinal = self.next_ordinal.checked_sub(1)?;
        let viewport_start = self.visible_start_row * self.columns;
        (ordinal >= viewport_start).then_some(ordinal - viewport_start)
    }

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

fn luma_variance(image: &RgbImage) -> f64 {
    let values: Vec<f64> = image
        .pixels()
        .map(|pixel| (pixel[0] as f64 + pixel[1] as f64 + pixel[2] as f64) / 3.0)
        .collect();
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64
}

fn gold_ratio(image: &RgbImage) -> f64 {
    let gold = image
        .pixels()
        .filter(|pixel| {
            let [r, g, b] = pixel.0;
            r > 150 && g > 105 && r > b.saturating_add(35)
        })
        .count();
    gold as f64 / (image.width() as usize * image.height() as usize).max(1) as f64
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
        config.panel_timeout = Duration::from_millis(40);
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
    fn defaults_are_bounded_and_wgc() {
        let config = ScanConfig::default();
        assert_eq!(config.capture_method, CaptureMethod::Wgc);
        assert!(config.max_inventory_items <= 4_000);
        assert!(config.max_characters <= 200);
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
    fn character_navigation_requires_visible_identity_change() {
        let previous = RgbImage::from_pixel(1280, 720, Rgb([24, 28, 35]));
        let mut changed = previous.clone();
        paint_rect(
            &mut changed,
            crate::vision::Point::new(0.30, 0.12),
            0.20,
            0.08,
            Rgb([180, 120, 90]),
        );
        let config = ScanConfig {
            next_character_key: 'n',
            ..ScanConfig::default()
        };
        let mut scanner =
            scanner_with_frames(vec![changed.clone(), changed.clone()], config.clone());
        let next = scanner.advance_character(&previous, 0).unwrap();
        assert_eq!(next, changed);
        assert_eq!(scanner.device().commands(), &[InputCommand::Key('n')]);

        let mut stuck = scanner_with_frames(vec![previous.clone(), previous.clone()], config);
        let error = stuck.advance_character(&previous, 0).unwrap_err();
        assert_eq!(error.code(), "HSR-CHAR-ADVANCE");
        assert_eq!(stuck.device().commands(), &[InputCommand::Key('n')]);
    }

    fn character_frame(color: Rgb<u8>) -> RgbImage {
        let mut frame = RgbImage::from_pixel(1280, 720, Rgb([24, 28, 35]));
        paint_rect(
            &mut frame,
            crate::vision::Point::new(0.78, 0.08),
            0.25,
            0.08,
            color,
        );
        frame
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
        expected: Option<usize>,
        maximum: usize,
    ) -> CharacterScan {
        let reader = ScriptedOcrReader::default()
            .with(OcrField::CharacterName, names.iter().copied())
            .with(
                OcrField::CharacterLevel,
                std::iter::repeat_n("等级 80/80", names.len()),
            );
        let config = ScanConfig {
            panel_timeout: Duration::from_millis(40),
            max_characters: maximum,
            expected_characters: expected,
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
    fn character_repetition_without_expected_count_remains_unknown() {
        let first = character_frame(Rgb([90, 120, 170]));
        let visually_changed_same_id = character_frame(Rgb([170, 90, 120]));
        let frames = vec![
            first.clone(),
            first.clone(),
            first.clone(),
            first,
            visually_changed_same_id.clone(),
            visually_changed_same_id.clone(),
            visually_changed_same_id.clone(),
            visually_changed_same_id,
        ];
        let scan = run_character_simulation(frames, &["三月七", "三月七"], None, 3);
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.coverage, CoverageLevel::Unknown);
    }

    #[test]
    fn expected_character_undercount_with_one_more_stable_panel_remains_unknown() {
        let first = character_frame(Rgb([90, 120, 170]));
        let second = character_frame(Rgb([170, 90, 120]));
        let extra = character_frame(Rgb([90, 170, 120]));
        let frames = vec![
            first.clone(),
            first.clone(),
            first.clone(),
            first,
            second.clone(),
            second.clone(),
            second.clone(),
            second.clone(),
            extra.clone(),
            extra,
        ];
        let scan = run_character_simulation(frames, &["三月七", "希儿"], Some(2), 3);
        assert_eq!(scan.items.len(), 2);
        assert_eq!(scan.coverage, CoverageLevel::Unknown);
    }

    #[test]
    fn expected_character_count_plus_stable_wrap_proves_complete() {
        let first = character_frame(Rgb([90, 120, 170]));
        let second = character_frame(Rgb([170, 90, 120]));
        let frames = vec![
            first.clone(),
            first.clone(),
            first.clone(),
            first.clone(),
            second.clone(),
            second.clone(),
            second.clone(),
            second,
            first.clone(),
            first,
        ];
        let scan = run_character_simulation(frames, &["三月七", "希儿"], Some(2), 3);
        assert_eq!(scan.items.len(), 2);
        assert_eq!(scan.coverage, CoverageLevel::Complete);
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
            grid,
            cell_index: 0,
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
            panel_timeout: Duration::from_millis(40),
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
            panel_timeout: Duration::from_millis(40),
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
            panel_timeout: Duration::from_millis(40),
            expected_characters: Some(1),
            ..ScanConfig::default()
        };
        let mut scanner = HsrScanner::new(
            ReplayDevice::new(
                1280,
                720,
                vec![frame.clone(), frame.clone(), frame.clone(), frame],
            ),
            ScriptedOcrReader::default().with(OcrField::CharacterName, ["三月七"]),
            references,
            config,
        );
        let scan = scanner.scan_characters().unwrap();
        assert!(scan.items.is_empty());
        assert_eq!(scan.coverage, CoverageLevel::Unknown);
    }
}
