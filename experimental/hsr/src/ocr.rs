use std::collections::{BTreeMap, VecDeque};

use image::RgbImage;
use regex::Regex;
use yas::ocr::{ImageToText, PPOCRChV4RecInfer};

use crate::{
    annotator,
    error::{hints, HsrError, HsrResult},
    layout,
    model::{
        CharacterReference, GearReference, ObservedCharacter, ObservedGear, ObservedLightCone,
        ObservedSubstat, StatReference, StatValueKind,
    },
    reference::ReferenceCache,
    vision::NormRect,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OcrField {
    InventoryQuantity,
    CharacterName,
    CharacterLevel,
    GearName,
    GearLevel,
    GearMainName,
    GearMainValue,
    GearSubName(usize),
    GearSubValue(usize),
    GearEquipped,
    LightConeName,
    LightConeLevel,
    LightConeSuperimposition,
    LightConeEquipped,
}

impl OcrField {
    pub fn dump_name(self) -> String {
        match self {
            Self::InventoryQuantity => "quantity".to_string(),
            Self::CharacterName => "character_name".to_string(),
            Self::CharacterLevel => "character_level".to_string(),
            Self::GearName => "gear_name".to_string(),
            Self::GearLevel => "gear_level".to_string(),
            Self::GearMainName => "gear_main_name".to_string(),
            Self::GearMainValue => "gear_main_value".to_string(),
            Self::GearSubName(index) => format!("gear_sub_name_{index}"),
            Self::GearSubValue(index) => format!("gear_sub_value_{index}"),
            Self::GearEquipped => "gear_equipped".to_string(),
            Self::LightConeName => "light_cone_name".to_string(),
            Self::LightConeLevel => "light_cone_level".to_string(),
            Self::LightConeSuperimposition => "light_cone_superimposition".to_string(),
            Self::LightConeEquipped => "light_cone_equipped".to_string(),
        }
    }
}

pub trait OcrReader {
    fn read(&mut self, field: OcrField, image: &RgbImage) -> HsrResult<String>;
}

pub struct PaddleOcrReader {
    model: PPOCRChV4RecInfer,
}

impl PaddleOcrReader {
    pub fn new() -> HsrResult<Self> {
        PPOCRChV4RecInfer::new()
            .map(|model| Self { model })
            .map_err(|error| {
                HsrError::new(
                    "HSR-OCR-MODEL",
                    hints::OCR_FAILED,
                    format!("PaddleOCR v4 initialization failed; cause={error}"),
                )
            })
    }
}

impl OcrReader for PaddleOcrReader {
    fn read(&mut self, field: OcrField, image: &RgbImage) -> HsrResult<String> {
        let infer = |model: &PPOCRChV4RecInfer, image: &RgbImage| {
            model.image_to_text(image, false).map_err(|error| {
                HsrError::new(
                    "HSR-OCR-INFERENCE",
                    hints::OCR_FAILED,
                    format!("OCR inference failed; cause={error}"),
                )
            })
        };
        let text = infer(&self.model, image)?.trim().to_string();
        if !text.is_empty() {
            return Ok(text);
        }
        // Retry only empty results. A successful first read is never replaced.
        yas::log_debug!(
            "OCR {} 第一次为空，正在重试同一裁剪。",
            "OCR {} was empty on the first read; retrying the same crop.",
            field.dump_name()
        );
        Ok(infer(&self.model, image)?.trim().to_string())
    }
}

#[derive(Default)]
pub struct ScriptedOcrReader {
    values: BTreeMap<OcrField, VecDeque<String>>,
    calls: Vec<OcrField>,
    crop_sizes: Vec<(OcrField, (u32, u32))>,
}

impl ScriptedOcrReader {
    pub fn with(
        mut self,
        field: OcrField,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.values
            .entry(field)
            .or_default()
            .extend(values.into_iter().map(Into::into));
        self
    }

    pub fn calls(&self) -> &[OcrField] {
        &self.calls
    }

    pub fn crop_sizes(&self) -> &[(OcrField, (u32, u32))] {
        &self.crop_sizes
    }
}

impl OcrReader for ScriptedOcrReader {
    fn read(&mut self, field: OcrField, _image: &RgbImage) -> HsrResult<String> {
        self.calls.push(field);
        self.crop_sizes.push((field, _image.dimensions()));
        self.values
            .get_mut(&field)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| {
                HsrError::new(
                    "HSR-OCR-SCRIPT",
                    hints::OCR_FAILED,
                    format!("no scripted OCR value for field={field:?}"),
                )
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatsPanelLayout {
    pub panel: NormRect,
}

impl StatsPanelLayout {
    pub const MAINTAINED_SEED: Self = Self {
        panel: layout::STATS_PANEL,
    };

    pub fn candidates() -> Vec<Self> {
        let mut candidates = Vec::new();
        for dx in [0.0, -0.012, 0.012, -0.024, 0.024] {
            for dy in [0.0, -0.012, 0.012] {
                candidates.push(Self {
                    panel: NormRect::new(
                        layout::STATS_PANEL.x + dx,
                        layout::STATS_PANEL.y + dy,
                        layout::STATS_PANEL.width,
                        layout::STATS_PANEL.height,
                    ),
                });
            }
        }
        candidates
    }

    fn crop(self, frame: &RgbImage, relative: NormRect) -> HsrResult<RgbImage> {
        self.panel.relative(relative).crop(frame)
    }

    pub fn immutable_panel(self) -> NormRect {
        self.panel.relative(NormRect::new(0.0, 0.0, 1.0, 0.76))
    }

    pub fn lock_button(self, kind: InventoryKind) -> NormRect {
        match kind {
            InventoryKind::LightCone => self.panel.relative(layout::LIGHT_CONE_LOCK),
            InventoryKind::Gear => self.panel.relative(layout::RELIC_LOCK),
        }
    }

    pub fn discard_button(self) -> NormRect {
        self.panel.relative(layout::RELIC_DISCARD)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryKind {
    LightCone,
    Gear,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedGearPanel {
    pub observation: ObservedGear,
    pub reference: GearReference,
    pub equipped: Option<bool>,
    pub location_key: Option<String>,
    pub icon_confidence: f64,
}

/// Minimum joint lock/discard icon confidence that the live manager may treat
/// as actionable. The icon classifier reports `0.0..=1.0`; `0.60` requires a
/// dominant-minus-competing classified-pixel margin of at least five percent.
/// Competing gold/neutral evidence at or above thirty percent of the dominant
/// signal is unknown regardless of its raw pixel count. This maintained
/// conservative boundary is still subject to the explicitly documented
/// real-client calibration pass.
pub const MANAGED_ICON_CONFIDENCE_THRESHOLD: f64 = 0.60;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLightConePanel {
    pub observation: ObservedLightCone,
    pub equipped: Option<bool>,
    pub location_key: Option<String>,
    pub icon_confidence: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCharacterPanel {
    pub observation: ObservedCharacter,
    pub reference: CharacterReference,
}

pub struct PanelParser<R> {
    reader: R,
}

impl<R: OcrReader> PanelParser<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn reader(&self) -> &R {
        &self.reader
    }

    pub fn reader_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    pub fn discover_inventory_panel(
        &mut self,
        frame: &RgbImage,
        kind: InventoryKind,
        references: &ReferenceCache,
    ) -> HsrResult<StatsPanelLayout> {
        for layout in StatsPanelLayout::candidates() {
            let field = match kind {
                InventoryKind::LightCone => OcrField::LightConeName,
                InventoryKind::Gear => OcrField::GearName,
            };
            let name_rect = match kind {
                InventoryKind::LightCone => layout::LIGHT_CONE_NAME,
                InventoryKind::Gear => layout::RELIC_NAME,
            };
            let title = self.reader.read(field, &layout.crop(frame, name_rect)?)?;
            annotator::record_ocr(
                field.dump_name().as_str(),
                layout.panel.relative(name_rect),
                &title,
            );
            let resolved = match kind {
                InventoryKind::LightCone => references.resolve_light_cone_name(&title).is_some(),
                InventoryKind::Gear => references.resolve_gear_name_any_rarity(&title).is_some(),
            };
            if resolved {
                return Ok(layout);
            }
        }
        Err(HsrError::new(
            "HSR-OCR-PANEL",
            hints::SCREEN_INVALID,
            "no candidate detail panel produced a unique reference-backed item name",
        ))
    }

    pub fn parse_gear(
        &mut self,
        frame: &RgbImage,
        layout: StatsPanelLayout,
        references: &ReferenceCache,
    ) -> HsrResult<ParsedGearPanel> {
        let name_text = self.read_crop(OcrField::GearName, frame, layout, layout::RELIC_NAME)?;
        let rarity_rect = layout.panel.relative(layout::RELIC_RARITY);
        let rarity_crop = rarity_rect.crop(frame)?;
        let name_color_crop = layout.crop(frame, layout::RELIC_NAME)?;
        let rarity = detect_rarity(&rarity_crop)
            .or_else(|| detect_rarity_from_name_color(&name_color_crop));
        annotator::record_ocr(
            "gear_rarity",
            rarity_rect,
            &rarity.map(|value| value.to_string()).unwrap_or_default(),
        );
        let rarity = rarity.ok_or_else(|| ocr_semantic("gear rarity unreadable"))?;
        let level_text = self.read_crop(OcrField::GearLevel, frame, layout, layout::RELIC_LEVEL)?;
        let level = parse_first_u8(&level_text, 15, "gear level")?;

        let mut main_name = self.read_crop(
            OcrField::GearMainName,
            frame,
            layout,
            layout::RELIC_MAIN_NAME,
        )?;
        if main_name.trim().is_empty() {
            let candidates = references.gear_name_candidates(&name_text, rarity);
            let fixed_slot = candidates
                .first()
                .map(|candidate| candidate.slot)
                .filter(|slot| candidates.iter().all(|candidate| candidate.slot == *slot));
            main_name = match fixed_slot {
                Some(crate::model::GearSlot::Head) => "生命值".to_string(),
                Some(crate::model::GearSlot::Hands) => "攻击力".to_string(),
                _ => main_name,
            };
        }
        let main_value_text = self.read_crop(
            OcrField::GearMainValue,
            frame,
            layout,
            layout::RELIC_MAIN_VALUE,
        )?;
        let main_value = parse_display_number(&main_value_text, "main stat value")?;
        let main_stat = resolve_stat_with_context(references, &main_name, &main_value_text)
            .ok_or_else(|| {
            ocr_resolution(
                "HSR-OCR-STAT-AMBIGUOUS",
                "main stat label did not resolve uniquely after applying the visible percent marker and StatValueKind",
            )
        })?;
        let reference = references
            .resolve_gear_name_with_main_stat(
                &name_text,
                rarity,
                &main_stat.key,
                level,
                main_value,
            )
            .or_else(|| {
                // Legacy normalized fixtures predate main-affix progression.
                // They remain usable only when name/rarity is independently
                // unique; duplicate legacy definitions still fail closed.
                references
                    .resolve_gear_name(&name_text, rarity)
                    .filter(|gear| gear.main_affix_group == 0)
            })
            .cloned()
            .ok_or_else(|| {
                ocr_resolution(
                    "HSR-OCR-GEAR-AMBIGUOUS",
                    "gear name/rarity/main-stat progression did not select one unique or canonically visible-equivalent public definition",
                )
            })?;
        annotator::set_final(
            OcrField::GearName.dump_name().as_str(),
            &reference.game_id.to_string(),
        );
        // The visible number is used above to validate the candidate against
        // UI formatting, but full live references export the authoritative
        // GIlore progression value. This keeps matcher semantics stable across
        // harmless UI rounding. Legacy fixtures without progression retain
        // their explicitly non-live OCR value.
        let main_value = references
            .relic_main_stat_value_for_piece(&reference, &main_stat.key, level)
            .unwrap_or(main_value);

        let mut substats = Vec::new();
        for index in 0..4 {
            let name = self.read_crop(
                OcrField::GearSubName(index),
                frame,
                layout,
                layout::relic_sub_name(index),
            )?;
            let value = self.read_crop(
                OcrField::GearSubValue(index),
                frame,
                layout,
                layout::relic_sub_value(index),
            )?;
            if name.trim().is_empty() && value.trim().is_empty() {
                continue;
            }
            let stat = resolve_stat_with_context(references, strip_inactive_suffix(&name), &value)
                .ok_or_else(|| {
                    ocr_resolution(
                        "HSR-OCR-STAT-AMBIGUOUS",
                        format!(
                            "substat line={} label/value marker did not resolve uniquely",
                            index + 1
                        ),
                    )
                })?;
            substats.push(ObservedSubstat {
                stat_key: stat.key.clone(),
                value: parse_display_number(&value, "substat value")?,
            });
        }
        substats.sort_by(|left, right| left.stat_key.cmp(&right.stat_key));
        if substats
            .windows(2)
            .any(|pair| pair[0].stat_key == pair[1].stat_key)
        {
            return Err(ocr_semantic("duplicate resolved substat key"));
        }

        let equip_crop = layout.crop(frame, layout::RELIC_EQUIPPED)?;
        let equip_text = self.reader.read(OcrField::GearEquipped, &equip_crop)?;
        annotator::record_ocr(
            OcrField::GearEquipped.dump_name().as_str(),
            layout.panel.relative(layout::RELIC_EQUIPPED),
            &equip_text,
        );
        let (equipped, location_key) = parse_equipped(&equip_text, &equip_crop, references);
        let lock_rect = layout.lock_button(InventoryKind::Gear);
        let lock_crop = lock_rect.crop(frame)?;
        let (lock, lock_confidence) = detect_icon_state(&lock_crop);
        record_icon("gear_lock", lock_rect, &lock_crop, lock, lock_confidence);
        let discard_rect = layout.discard_button();
        let discard_crop = discard_rect.crop(frame)?;
        let (discard, discard_confidence) = detect_discard_state(&discard_crop);
        record_icon(
            "gear_discard",
            discard_rect,
            &discard_crop,
            discard,
            discard_confidence,
        );

        Ok(ParsedGearPanel {
            observation: ObservedGear {
                piece_id: reference.game_id,
                level,
                main_stat_key: main_stat.key.clone(),
                main_stat_value: main_value,
                substats,
                equipped_character_id: location_key
                    .as_deref()
                    .and_then(|key| key.parse::<u32>().ok()),
                lock,
                discard,
            },
            reference,
            equipped,
            location_key,
            icon_confidence: lock_confidence.min(discard_confidence),
        })
    }

    pub fn parse_light_cone(
        &mut self,
        frame: &RgbImage,
        layout: StatsPanelLayout,
        references: &ReferenceCache,
    ) -> HsrResult<ParsedLightConePanel> {
        let name = self.read_crop(
            OcrField::LightConeName,
            frame,
            layout,
            layout::LIGHT_CONE_NAME,
        )?;
        let reference = references
            .resolve_light_cone_name(&name)
            .ok_or_else(|| ocr_semantic("Light Cone name did not resolve uniquely"))?;
        annotator::set_final(
            OcrField::LightConeName.dump_name().as_str(),
            &reference.game_id.to_string(),
        );
        let level_text = self.read_crop(
            OcrField::LightConeLevel,
            frame,
            layout,
            layout::LIGHT_CONE_LEVEL,
        )?;
        let (level, cap) = parse_level_and_cap(&level_text)?;
        let superimposition_text = self.read_crop(
            OcrField::LightConeSuperimposition,
            frame,
            layout,
            layout::LIGHT_CONE_SUPERIMPOSITION,
        )?;
        let superimposition = parse_first_u8(&superimposition_text, 5, "superimposition")?;
        let equip_crop = layout.crop(frame, layout::LIGHT_CONE_EQUIPPED)?;
        let equip_text = self.reader.read(OcrField::LightConeEquipped, &equip_crop)?;
        annotator::record_ocr(
            OcrField::LightConeEquipped.dump_name().as_str(),
            layout.panel.relative(layout::LIGHT_CONE_EQUIPPED),
            &equip_text,
        );
        let (equipped, location_key) = parse_equipped(&equip_text, &equip_crop, references);
        let lock_rect = layout.lock_button(InventoryKind::LightCone);
        let lock_crop = lock_rect.crop(frame)?;
        let (lock, icon_confidence) = detect_icon_state(&lock_crop);
        record_icon(
            "light_cone_lock",
            lock_rect,
            &lock_crop,
            lock,
            icon_confidence,
        );
        Ok(ParsedLightConePanel {
            observation: ObservedLightCone {
                light_cone_id: reference.game_id,
                level,
                ascension: ascension_from_cap(cap),
                superimposition,
                equipped_character_id: location_key
                    .as_deref()
                    .and_then(|key| key.parse::<u32>().ok()),
                lock,
            },
            equipped,
            location_key,
            icon_confidence,
        })
    }

    pub fn parse_character_details(
        &mut self,
        frame: &RgbImage,
        references: &ReferenceCache,
        eidolon: u8,
    ) -> HsrResult<ParsedCharacterPanel> {
        let name_crop = layout::CHARACTER_NAME.crop(frame)?;
        let name_text = self.reader.read(OcrField::CharacterName, &name_crop)?;
        annotator::record_ocr(
            OcrField::CharacterName.dump_name().as_str(),
            layout::CHARACTER_NAME,
            &name_text,
        );
        let (path_text, character_name) = character_header_segments(&name_text);
        let reference = path_text
            .map_or_else(
                || references.resolve_character_name(character_name),
                |path| references.resolve_character_name_and_path(character_name, path),
            )
            .cloned()
            .ok_or_else(|| {
                ocr_resolution(
                    "HSR-OCR-CHARACTER-AMBIGUOUS",
                    "character name did not resolve uniquely; duplicate variants and renameable characters require independent path/variant evidence or explicit user configuration",
                )
            })?;
        annotator::set_final(
            OcrField::CharacterName.dump_name().as_str(),
            &reference.game_id.to_string(),
        );
        let level_crop = layout::CHARACTER_LEVEL.crop(frame)?;
        let level_text = self.reader.read(OcrField::CharacterLevel, &level_crop)?;
        annotator::record_ocr(
            OcrField::CharacterLevel.dump_name().as_str(),
            layout::CHARACTER_LEVEL,
            &level_text,
        );
        let (level, cap) = parse_level_and_cap(&level_text)?;
        Ok(ParsedCharacterPanel {
            observation: ObservedCharacter {
                character_id: reference.game_id,
                level,
                ascension: ascension_from_cap(cap),
                eidolon,
            },
            reference,
        })
    }

    fn read_crop(
        &mut self,
        field: OcrField,
        frame: &RgbImage,
        layout: StatsPanelLayout,
        rect: NormRect,
    ) -> HsrResult<String> {
        let text = self.reader.read(field, &layout.crop(frame, rect)?)?;
        let absolute = layout.panel.relative(rect);
        annotator::record_ocr(&field.dump_name(), absolute, &text);
        yas::log_debug!("OCR {} = {}", "OCR {} = {}", field.dump_name(), text);
        Ok(text)
    }
}

fn parse_first_u8(text: &str, maximum: u8, label: &str) -> HsrResult<u8> {
    let regex = Regex::new(r"\d+").expect("static regex");
    let value = regex
        .find(text)
        .and_then(|capture| capture.as_str().parse::<u8>().ok())
        .filter(|value| *value <= maximum)
        .ok_or_else(|| ocr_semantic(format!("{label} was missing or outside 0..={maximum}")))?;
    Ok(value)
}

fn parse_level_and_cap(text: &str) -> HsrResult<(u8, u8)> {
    let regex = Regex::new(r"\d+").expect("static regex");
    let values: Vec<u8> = regex
        .find_iter(text)
        .filter_map(|capture| capture.as_str().parse().ok())
        .collect();
    let level = *values
        .first()
        .ok_or_else(|| ocr_semantic("level was unreadable"))?;
    let cap = values.get(1).copied().unwrap_or_else(|| infer_cap(level));
    if !(1..=100).contains(&level) || !(20..=100).contains(&cap) || level > cap {
        return Err(ocr_semantic("level/cap values are mechanically invalid"));
    }
    Ok((level, cap))
}

fn infer_cap(level: u8) -> u8 {
    match level {
        0..=20 => 20,
        21..=30 => 30,
        31..=40 => 40,
        41..=50 => 50,
        51..=60 => 60,
        61..=70 => 70,
        _ => 80,
    }
}

fn ascension_from_cap(cap: u8) -> u8 {
    cap.saturating_sub(20) / 10
}

fn parse_display_number(text: &str, label: &str) -> HsrResult<f64> {
    let normalized = text
        .replace([',', '，'], "")
        .replace('％', "%")
        .replace('S', "5");
    let regex = Regex::new(r"[-+]?\d+(?:\.\d+)?").expect("static regex");
    regex
        .find(&normalized)
        .and_then(|capture| capture.as_str().trim_start_matches('+').parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .ok_or_else(|| ocr_semantic(format!("{label} was unreadable")))
}

fn resolve_stat_with_context<'a>(
    references: &'a ReferenceCache,
    label: &str,
    value_text: &str,
) -> Option<&'a StatReference> {
    let expected_kind = if value_implies_percent(label, value_text) {
        StatValueKind::Ratio
    } else {
        StatValueKind::Flat
    };
    references
        .resolve_stat_name_by_kind(label, expected_kind)
        .or_else(|| {
            references.resolve_stat_name(label).filter(|candidate| {
                candidate.value_kind == StatValueKind::Unknown
                    || candidate.value_kind == expected_kind
            })
        })
}

/// Percent is trusted when the glyph is present. The only additional case is a
/// dropped '%' on HP/ATK/DEF: in-game flats are integers, so a single-digit
/// decimal like "4.8" cannot be a good flat reading.
fn value_implies_percent(label: &str, value_text: &str) -> bool {
    if value_text.contains(['%', '％']) {
        return true;
    }
    is_hp_atk_def_label(label) && is_single_decimal(value_text)
}

fn is_single_decimal(value_text: &str) -> bool {
    let compact: String = value_text
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '+' && *ch != ',' && *ch != '，')
        .map(|ch| if ch == 'S' { '5' } else { ch })
        .collect();
    let bytes = compact.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_digit() && bytes[1] == b'.' && bytes[2].is_ascii_digit()
}

fn is_hp_atk_def_label(label: &str) -> bool {
    let normalized: String = label
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "生命值" | "攻击力" | "防御力" | "hp" | "atk" | "def"
    )
}

fn strip_inactive_suffix(value: &str) -> &str {
    value.split(['(', '（']).next().unwrap_or(value).trim()
}

fn character_header_segments(value: &str) -> (Option<&str>, &str) {
    let segments = value
        .split(['/', '／'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [name] => (None, name),
        [path, .., name] => (Some(path), name),
        _ => (None, value.trim()),
    }
}

fn parse_equipped(
    text: &str,
    crop: &RgbImage,
    references: &ReferenceCache,
) -> (Option<bool>, Option<String>) {
    let normalized: String = text
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect();
    let lower = normalized.to_ascii_lowercase();
    let says_unequipped = matches!(
        lower.as_str(),
        "equip" | "unequipped" | "notequipped" | "装备" | "裝備" | "未装备" | "未裝備"
    );
    let says_equipped = !says_unequipped
        && (lower.contains("equipped")
            || normalized.contains("装备中")
            || normalized.contains("裝備中")
            || normalized.contains("已装备")
            || normalized.contains("已裝備"));
    if says_equipped {
        let stripped = normalized
            .replace("Equipped", "")
            .replace("equipped", "")
            .replace("装备中", "")
            .replace("裝備中", "")
            .replace("已装备", "")
            .replace("已裝備", "");
        let location = references
            .resolve_character_name(&stripped)
            .map(|reference| reference.key.clone());
        return (Some(true), location);
    }
    // A blank or low-contrast crop can be an OCR miss and must never authorize
    // manager mutation. `false` requires both an explicit localized
    // unequipped/equip-action label and independently visible glyph/button
    // edges in the maintained narrow crop.
    if says_unequipped && edge_density(crop) >= 0.025 {
        (Some(false), None)
    } else {
        (None, None)
    }
}

/// Tri-state icon classifier. Gold pixels identify an active lock/mark; a
/// clear neutral-white glyph identifies the inactive state. Low-contrast or
/// mixed evidence remains unknown and is never coerced to false.
pub fn detect_icon_state(image: &RgbImage) -> (Option<bool>, f64) {
    if image.as_raw().is_empty() {
        return (None, 0.0);
    }
    let mut gold = 0_usize;
    let mut neutral = 0_usize;
    for pixel in image.pixels() {
        let [r, g, b] = pixel.0;
        if r > 155 && g > 105 && r > b.saturating_add(35) && g > b.saturating_add(15) {
            gold += 1;
        }
        if r > 165 && g > 165 && b > 165 && r.abs_diff(g) < 28 && g.abs_diff(b) < 28 {
            neutral += 1;
        }
    }
    let total = image.width() as usize * image.height() as usize;
    let gold_ratio = gold as f64 / total.max(1) as f64;
    let neutral_ratio = neutral as f64 / total.max(1) as f64;
    let (state, dominant, competing, recognition_floor) = if gold_ratio > neutral_ratio {
        (true, gold_ratio, neutral_ratio, 0.018)
    } else {
        (false, neutral_ratio, gold_ratio, 0.022)
    };

    // A strong total count is not useful when the two mutually-exclusive UI
    // states both have substantial support. Unknown gets zero confidence so a
    // mixed lock crop also makes the joint lock/discard manager evidence
    // non-actionable rather than allowing the other icon through by itself.
    if dominant <= recognition_floor || competing >= dominant * 0.30 {
        return (None, 0.0);
    }

    let confidence = ((dominant - competing) * 12.0).min(1.0);
    (Some(state), confidence)
}

/// Discard sits on the 5-star gold card. Card chrome is not a discard mark.
/// kel-z template-matches the lit trash icon; we keep that idea fail-closed:
/// red/orange fill is discarded, a gray/white glyph without that fill is not,
/// and gold-only chrome stays unknown.
pub fn detect_discard_state(image: &RgbImage) -> (Option<bool>, f64) {
    if image.as_raw().is_empty() {
        return (None, 0.0);
    }
    let mut red = 0_usize;
    let mut gray = 0_usize;
    let mut white = 0_usize;
    let mut card_gold = 0_usize;
    for pixel in image.pixels() {
        let [r, g, b] = pixel.0;
        let is_red = r > 170 && r > g.saturating_add(50) && g < 140 && b < 120;
        if is_red {
            red += 1;
            continue;
        }
        if r > 165 && g > 165 && b > 165 && r.abs_diff(g) < 28 && g.abs_diff(b) < 28 {
            white += 1;
            continue;
        }
        if r.abs_diff(g) < 25 && g.abs_diff(b) < 25 && (60..160).contains(&r) {
            gray += 1;
            continue;
        }
        if r > 155 && g > 105 && r > b.saturating_add(35) && g > b.saturating_add(15) {
            card_gold += 1;
        }
    }
    let total = (image.width() as usize * image.height() as usize).max(1) as f64;
    let red_ratio = red as f64 / total;
    let gray_ratio = gray as f64 / total;
    let white_ratio = white as f64 / total;
    let gold_ratio = card_gold as f64 / total;

    if red_ratio >= 0.08 && red_ratio >= gray_ratio && red_ratio >= white_ratio {
        return (Some(true), (red_ratio * 8.0).min(1.0));
    }
    if white_ratio >= 0.022 && white_ratio > gold_ratio && white_ratio >= red_ratio * 3.0 {
        return (
            Some(false),
            ((white_ratio - red_ratio) * 12.0).min(1.0),
        );
    }
    if gray_ratio >= 0.08 && red_ratio < 0.03 && gray_ratio >= gold_ratio * 0.30 {
        return (Some(false), (gray_ratio * 6.0).min(1.0));
    }
    (None, 0.0)
}

fn edge_density(image: &RgbImage) -> f64 {
    if image.width() < 2 || image.height() < 2 {
        return 0.0;
    }
    let mut edges = 0_usize;
    let mut total = 0_usize;
    for y in 1..image.height() {
        for x in 1..image.width() {
            let pixel = image.get_pixel(x, y);
            let left = image.get_pixel(x - 1, y);
            let above = image.get_pixel(x, y - 1);
            let diff = (0..3)
                .map(|channel| {
                    pixel[channel].abs_diff(left[channel]) as u16
                        + pixel[channel].abs_diff(above[channel]) as u16
                })
                .sum::<u16>();
            edges += usize::from(diff > 120);
            total += 1;
        }
    }
    edges as f64 / total.max(1) as f64
}

fn detect_rarity(image: &RgbImage) -> Option<u8> {
    // Count separated bright/gold runs along the horizontal star strip. This
    // intentionally ignores exact RGB constants and scales with the crop.
    let mut active_columns = Vec::with_capacity(image.width() as usize);
    for x in 0..image.width() {
        let count = (0..image.height())
            .filter(|y| {
                let [r, g, b] = image.get_pixel(x, *y).0;
                (r > 175 && g > 135 && b < 175) || (r > 210 && g > 210 && b > 210)
            })
            .count();
        active_columns.push(count > (image.height() as usize / 12).max(1));
    }
    let minimum_gap = (image.width() / 30).max(1) as usize;
    let mut runs = 0_u8;
    let mut gap = minimum_gap;
    let mut in_run = false;
    for active in active_columns {
        if active {
            if !in_run && gap >= minimum_gap {
                runs = runs.saturating_add(1);
            }
            in_run = true;
            gap = 0;
        } else {
            in_run = false;
            gap += 1;
        }
    }
    (1..=5).contains(&runs).then_some(runs)
}

/// Current HSR relic panels no longer draw a star strip; 5/4/3-star names use
/// gold/purple/blue. This is a fallback only after the star-run detector
/// returns nothing, so a real star crop is never overridden.
fn detect_rarity_from_name_color(image: &RgbImage) -> Option<u8> {
    let mut gold = 0_u32;
    let mut purple = 0_u32;
    let mut blue = 0_u32;
    for pixel in image.pixels() {
        let [r, g, b] = pixel.0;
        if r > 190 && (110..210).contains(&g) && b < 130 {
            gold += 1;
        } else if r > 110 && b > 150 && g < 150 {
            purple += 1;
        } else if b > 160 && b > r.saturating_add(25) && b > g.saturating_add(25) {
            blue += 1;
        }
    }
    let dominant = gold.max(purple).max(blue);
    if dominant < 24 {
        return None;
    }
    if gold >= purple && gold >= blue {
        Some(5)
    } else if purple >= blue {
        Some(4)
    } else {
        Some(3)
    }
}

fn record_icon(field: &str, rect: NormRect, crop: &RgbImage, state: Option<bool>, confidence: f64) {
    let summary = format!("state={state:?} confidence={confidence:.2}");
    annotator::record_ocr(field, rect, &summary);
    let pixel = crop.get_pixel(crop.width() / 2, crop.height() / 2).0;
    annotator::record_pixel(field, rect.center(), pixel, &summary);
}

fn ocr_semantic(detail: impl Into<String>) -> HsrError {
    HsrError::new("HSR-OCR-SEMANTIC", hints::OCR_FAILED, detail)
}

fn ocr_resolution(code: &'static str, detail: impl Into<String>) -> HsrError {
    HsrError::new(code, hints::OCR_FAILED, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    use crate::{
        localization::BilingualName,
        model::{
            GearCategory, ReferenceSnapshot, RelicMainAffixReference, StatReference, StatValueKind,
        },
        reference::GiloreBundleReferenceProvider,
    };

    fn references() -> ReferenceCache {
        let snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        ReferenceCache::from_snapshot(snapshot).unwrap()
    }

    fn gilore_references() -> ReferenceCache {
        ReferenceCache::from_provider(&GiloreBundleReferenceProvider::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gilore_bundle"),
        ))
        .unwrap()
    }

    fn collision_references() -> ReferenceCache {
        let mut snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();

        let hp = snapshot
            .stats
            .iter_mut()
            .find(|stat| stat.key == "HPDelta")
            .unwrap();
        hp.value_kind = StatValueKind::Flat;
        let mut hp_ratio = hp.clone();
        hp_ratio.key = "HPAddedRatio".to_string();
        hp_ratio.value_kind = StatValueKind::Ratio;
        snapshot.stats.push(hp_ratio);

        let attack = snapshot
            .stats
            .iter_mut()
            .find(|stat| stat.key == "AttackAddedRatio")
            .unwrap();
        attack.value_kind = StatValueKind::Ratio;
        let mut attack_flat = attack.clone();
        attack_flat.key = "AttackDelta".to_string();
        attack_flat.value_kind = StatValueKind::Flat;
        snapshot.stats.push(attack_flat);

        snapshot.stats.extend([
            StatReference {
                key: "DefenceDelta".to_string(),
                name: BilingualName {
                    zh_cn: "防御力".to_string(),
                    en: "DEF".to_string(),
                },
                value_kind: StatValueKind::Flat,
            },
            StatReference {
                key: "DefenceAddedRatio".to_string(),
                name: BilingualName {
                    zh_cn: "防御力".to_string(),
                    en: "DEF".to_string(),
                },
                value_kind: StatValueKind::Ratio,
            },
        ]);

        snapshot.gear_pieces[0].main_affix_group = 51;
        snapshot.gear_pieces[0].max_level = 15;
        snapshot.gear_pieces[1].main_affix_group = 55;
        snapshot.gear_pieces[1].max_level = 15;
        snapshot.relic_main_affixes = vec![
            RelicMainAffixReference {
                group_id: 51,
                property_id: "HPDelta".to_string(),
                max_level: 15,
                level_values: vec![705.6; 16],
            },
            RelicMainAffixReference {
                group_id: 55,
                property_id: "AttackAddedRatio".to_string(),
                max_level: 15,
                level_values: vec![0.432; 16],
            },
        ];
        ReferenceCache::from_snapshot(snapshot).unwrap()
    }

    fn paint_rect(image: &mut RgbImage, rect: NormRect, color: Rgb<u8>) {
        let x0 = (rect.x * image.width() as f64).floor() as u32;
        let y0 = (rect.y * image.height() as f64).floor() as u32;
        let x1 = ((rect.x + rect.width) * image.width() as f64)
            .ceil()
            .min(image.width() as f64) as u32;
        let y1 = ((rect.y + rect.height) * image.height() as f64)
            .ceil()
            .min(image.height() as f64) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                image.put_pixel(x, y, color);
            }
        }
    }

    fn generated_panel_frame(lock: Rgb<u8>, discard: Rgb<u8>) -> RgbImage {
        generated_panel_frame_with_rarity(lock, discard, 5)
    }

    fn generated_panel_frame_with_rarity(
        lock: Rgb<u8>,
        discard: Rgb<u8>,
        rarity_count: usize,
    ) -> RgbImage {
        let layout = StatsPanelLayout::MAINTAINED_SEED;
        let mut frame = RgbImage::from_pixel(1920, 1080, Rgb([24, 28, 35]));
        let rarity = layout.panel.relative(layout::RELIC_RARITY);
        for index in 0..rarity_count {
            paint_rect(
                &mut frame,
                NormRect::new(
                    rarity.x + rarity.width * (index as f64 + 0.08) / 5.0,
                    rarity.y + rarity.height * 0.16,
                    rarity.width * 0.45 / 5.0,
                    rarity.height * 0.68,
                ),
                Rgb([230, 185, 70]),
            );
        }
        paint_rect(&mut frame, layout.lock_button(InventoryKind::Gear), lock);
        paint_rect(&mut frame, layout.discard_button(), discard);
        paint_text_evidence(&mut frame, layout.panel.relative(layout::RELIC_EQUIPPED));
        frame
    }

    fn paint_text_evidence(image: &mut RgbImage, rect: NormRect) {
        let x0 = (rect.x * image.width() as f64).floor() as u32;
        let y0 = (rect.y * image.height() as f64).floor() as u32;
        let x1 = ((rect.x + rect.width) * image.width() as f64)
            .ceil()
            .min(image.width() as f64) as u32;
        let y1 = ((rect.y + rect.height) * image.height() as f64)
            .ceil()
            .min(image.height() as f64) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                if ((x - x0) / 4 + (y - y0) / 4) % 2 == 0 {
                    image.put_pixel(x, y, Rgb([225, 225, 225]));
                }
            }
        }
    }

    fn icon_image_with_counts(gold: usize, neutral: usize) -> RgbImage {
        let mut image = RgbImage::from_pixel(100, 100, Rgb([24, 28, 35]));
        assert!(gold + neutral <= image.pixels().count());
        for (index, pixel) in image.pixels_mut().enumerate() {
            if index < gold {
                *pixel = Rgb([220, 170, 70]);
            } else if index < gold + neutral {
                *pixel = Rgb([220, 220, 220]);
            }
        }
        image
    }

    fn paint_icon_ratios(
        image: &mut RgbImage,
        rect: NormRect,
        gold_ratio: f64,
        neutral_ratio: f64,
    ) {
        let x0 = (rect.x * image.width() as f64).floor() as u32;
        let y0 = (rect.y * image.height() as f64).floor() as u32;
        let x1 = ((rect.x + rect.width) * image.width() as f64)
            .ceil()
            .min(image.width() as f64) as u32;
        let y1 = ((rect.y + rect.height) * image.height() as f64)
            .ceil()
            .min(image.height() as f64) as u32;
        let width = x1 - x0;
        let total = width as usize * (y1 - y0) as usize;
        let gold = (total as f64 * gold_ratio).round() as usize;
        let neutral = (total as f64 * neutral_ratio).round() as usize;
        assert!(gold + neutral <= total);

        for index in 0..total {
            let x = x0 + (index % width as usize) as u32;
            let y = y0 + (index / width as usize) as u32;
            let color = if index < gold {
                Rgb([220, 170, 70])
            } else if index < gold + neutral {
                Rgb([220, 220, 220])
            } else {
                Rgb([24, 28, 35])
            };
            image.put_pixel(x, y, color);
        }
    }

    fn gear_reader(name: &str, main_name: &str, main_value: &str) -> ScriptedOcrReader {
        ScriptedOcrReader::default()
            .with(OcrField::GearName, [name])
            .with(OcrField::GearLevel, ["+15"])
            .with(OcrField::GearMainName, [main_name])
            .with(OcrField::GearMainValue, [main_value])
            .with(OcrField::GearSubName(0), ["暴击率"])
            .with(OcrField::GearSubValue(0), ["2.9%"])
            .with(OcrField::GearSubName(1), ["速度"])
            .with(OcrField::GearSubValue(1), ["2"])
            .with(OcrField::GearSubName(2), [""])
            .with(OcrField::GearSubValue(2), [""])
            .with(OcrField::GearSubName(3), [""])
            .with(OcrField::GearSubValue(3), [""])
            .with(OcrField::GearEquipped, ["装备"])
    }

    fn assert_only_narrow_crops(reader: &ScriptedOcrReader, full: (u32, u32)) {
        assert!(!reader.crop_sizes().is_empty());
        for (field, (width, height)) in reader.crop_sizes() {
            assert!(*width < full.0 && *height < full.1, "field={field:?}");
            assert!(
                u64::from(*width) * u64::from(*height) < u64::from(full.0) * u64::from(full.1) / 4,
                "OCR crop is unexpectedly broad for field={field:?}: {width}x{height}"
            );
        }
    }

    #[test]
    fn numbers_and_caps_are_strict() {
        assert_eq!(parse_level_and_cap("Lv. 70 / 80").unwrap(), (70, 80));
        assert_eq!(parse_display_number("+38.8%", "value").unwrap(), 38.8);
        assert!(parse_level_and_cap("garbled").is_err());
        assert!(parse_first_u8("S9", 5, "superimposition").is_err());
    }

    #[test]
    fn name_color_rarity_is_only_used_when_stars_are_absent() {
        let gold_name = RgbImage::from_pixel(80, 24, Rgb([220, 160, 70]));
        assert_eq!(detect_rarity_from_name_color(&gold_name), Some(5));
        let blank = RgbImage::from_pixel(80, 24, Rgb([24, 28, 35]));
        assert_eq!(detect_rarity_from_name_color(&blank), None);
        let star_strip = RgbImage::from_pixel(120, 20, Rgb([24, 28, 35]));
        assert_eq!(detect_rarity(&star_strip), None);
        assert_eq!(
            detect_rarity(&star_strip).or_else(|| detect_rarity_from_name_color(&gold_name)),
            Some(5)
        );
    }

    #[test]
    fn icon_state_preserves_unknown() {
        let blank = RgbImage::from_pixel(40, 40, Rgb([30, 30, 30]));
        assert_eq!(detect_icon_state(&blank).0, None);
        let gold = RgbImage::from_pixel(40, 40, Rgb([220, 170, 70]));
        assert_eq!(detect_icon_state(&gold).0, Some(true));
        let white = RgbImage::from_pixel(40, 40, Rgb([220, 220, 220]));
        assert_eq!(detect_icon_state(&white).0, Some(false));
    }

    #[test]
    fn icon_confidence_boundary_uses_the_dominant_minus_competing_margin() {
        for (gold, neutral, expected) in [
            (599, 100, Some(true)),
            (600, 100, Some(true)),
            (100, 599, Some(false)),
            (100, 600, Some(false)),
        ] {
            let (state, confidence) = detect_icon_state(&icon_image_with_counts(gold, neutral));
            assert_eq!(state, expected);
            if gold.max(neutral) - gold.min(neutral) == 500 {
                assert!(
                    confidence >= MANAGED_ICON_CONFIDENCE_THRESHOLD,
                    "exact five-percent net margin must meet the documented boundary: {confidence}"
                );
            } else {
                assert!(
                    confidence < MANAGED_ICON_CONFIDENCE_THRESHOLD,
                    "a net margin below five percent must not be actionable: {confidence}"
                );
            }
        }
    }

    #[test]
    fn competing_gold_and_neutral_evidence_is_unknown_in_both_directions() {
        for (gold, neutral) in [(400, 600), (600, 400), (150, 500), (500, 150)] {
            let (state, confidence) = detect_icon_state(&icon_image_with_counts(gold, neutral));
            assert_eq!(state, None, "gold={gold}, neutral={neutral}");
            assert_eq!(confidence, 0.0, "gold={gold}, neutral={neutral}");
        }
    }

    #[test]
    fn five_star_card_gold_around_a_gray_trash_is_not_discarded() {
        let mut image = RgbImage::from_pixel(40, 40, Rgb([220, 170, 70]));
        for y in 10..30 {
            for x in 10..30 {
                image.put_pixel(x, y, Rgb([90, 90, 90]));
            }
        }
        assert_eq!(detect_icon_state(&image).0, Some(true));
        assert_eq!(detect_discard_state(&image).0, Some(false));

        let gold_only = RgbImage::from_pixel(40, 40, Rgb([220, 170, 70]));
        assert_eq!(detect_discard_state(&gold_only).0, None);
    }

    #[test]
    fn mixed_icon_crops_fail_closed_for_both_gear_status_paths() {
        let layout = StatsPanelLayout::MAINTAINED_SEED;
        let mut frame = generated_panel_frame(Rgb([24, 28, 35]), Rgb([24, 28, 35]));
        paint_icon_ratios(
            &mut frame,
            layout.lock_button(InventoryKind::Gear),
            0.04,
            0.06,
        );
        paint_icon_ratios(&mut frame, layout.discard_button(), 0.06, 0.04);

        let parsed = PanelParser::new(gear_reader("过客的逢春木簪", "生命值", "705"))
            .parse_gear(&frame, layout, &references())
            .unwrap();
        assert_eq!(parsed.observation.lock, None);
        assert_eq!(parsed.observation.discard, None);
        assert_eq!(parsed.icon_confidence, 0.0);
    }

    #[test]
    fn generated_character_screenshot_uses_narrow_name_and_level_crops() {
        let frame = RgbImage::from_pixel(1920, 1080, Rgb([24, 28, 35]));
        let reader = ScriptedOcrReader::default()
            .with(OcrField::CharacterName, ["三月七"])
            .with(OcrField::CharacterLevel, ["等级 80/80"]);
        let mut parser = PanelParser::new(reader);
        let parsed = parser
            .parse_character_details(&frame, &references(), 6)
            .unwrap();
        assert_eq!(parsed.observation.character_id, 1001);
        assert_eq!(parsed.observation.level, 80);
        assert_eq!(parsed.observation.eidolon, 6);
        assert_eq!(
            parser.reader().calls(),
            &[OcrField::CharacterName, OcrField::CharacterLevel]
        );
        assert_only_narrow_crops(parser.reader(), frame.dimensions());
    }

    #[test]
    fn generated_light_cone_screenshot_parses_fields_without_full_frame_ocr() {
        let layout = StatsPanelLayout::MAINTAINED_SEED;
        let mut frame = RgbImage::from_pixel(1920, 1080, Rgb([24, 28, 35]));
        paint_rect(
            &mut frame,
            layout.lock_button(InventoryKind::LightCone),
            Rgb([220, 220, 220]),
        );
        paint_text_evidence(&mut frame, layout.panel.relative(layout::RELIC_EQUIPPED));
        let reader = ScriptedOcrReader::default()
            .with(OcrField::LightConeName, ["制胜的瞬间"])
            .with(OcrField::LightConeLevel, ["等级 80/80"])
            .with(OcrField::LightConeSuperimposition, ["叠影 1"])
            .with(OcrField::LightConeEquipped, ["装备"]);
        let mut parser = PanelParser::new(reader);
        let parsed = parser
            .parse_light_cone(&frame, layout, &references())
            .unwrap();
        assert_eq!(parsed.observation.light_cone_id, 23005);
        assert_eq!(parsed.observation.lock, Some(false));
        assert_eq!(parsed.equipped, Some(false));
        assert_only_narrow_crops(parser.reader(), frame.dimensions());
    }

    #[test]
    fn generated_gear_screens_cover_cavern_and_planar_reference_categories() {
        let layout = StatsPanelLayout::MAINTAINED_SEED;
        let frame = generated_panel_frame(Rgb([230, 180, 65]), Rgb([220, 220, 220]));
        let mut cavern = PanelParser::new(gear_reader("过客的逢春木簪", "生命值", "705"));
        let cavern_result = cavern.parse_gear(&frame, layout, &references()).unwrap();
        assert_eq!(cavern_result.reference.category, GearCategory::Relic);
        assert_eq!(cavern_result.observation.lock, Some(true));
        assert_eq!(cavern_result.observation.discard, Some(false));
        assert_eq!(cavern_result.observation.substats.len(), 2);
        assert_only_narrow_crops(cavern.reader(), frame.dimensions());

        let mut planar = PanelParser::new(gear_reader("「黑塔」的空间站点", "攻击力", "43.2%"));
        let planar_result = planar.parse_gear(&frame, layout, &references()).unwrap();
        assert_eq!(
            planar_result.reference.category,
            GearCategory::PlanarOrnament
        );
        assert_eq!(planar_result.observation.main_stat_value, 43.2);
        assert_only_narrow_crops(planar.reader(), frame.dimensions());
    }

    #[test]
    fn stat_label_collisions_use_percent_kind_and_main_affix_context() {
        let layout = StatsPanelLayout::MAINTAINED_SEED;
        let frame = generated_panel_frame(Rgb([230, 180, 65]), Rgb([220, 220, 220]));
        let refs = collision_references();

        let mut cavern = PanelParser::new(
            ScriptedOcrReader::default()
                .with(OcrField::GearName, ["过客的逢春木簪"])
                .with(OcrField::GearLevel, ["+15"])
                .with(OcrField::GearMainName, ["生命值"])
                .with(OcrField::GearMainValue, ["705"])
                .with(OcrField::GearSubName(0), ["生命值"])
                .with(OcrField::GearSubValue(0), ["3.8%"])
                .with(OcrField::GearSubName(1), ["攻击力"])
                .with(OcrField::GearSubValue(1), ["38"])
                .with(OcrField::GearSubName(2), ["防御力"])
                .with(OcrField::GearSubValue(2), ["4.3%"])
                .with(OcrField::GearSubName(3), ["防御力"])
                .with(OcrField::GearSubValue(3), ["19"])
                .with(OcrField::GearEquipped, ["装备"]),
        );
        let parsed = cavern.parse_gear(&frame, layout, &refs).unwrap();
        assert_eq!(parsed.observation.main_stat_key, "HPDelta");
        assert_eq!(parsed.observation.main_stat_value, 705.6);
        assert_eq!(
            parsed
                .observation
                .substats
                .iter()
                .map(|stat| stat.stat_key.as_str())
                .collect::<Vec<_>>(),
            vec![
                "AttackDelta",
                "DefenceAddedRatio",
                "DefenceDelta",
                "HPAddedRatio"
            ]
        );

        let mut planar = PanelParser::new(gear_reader("「黑塔」的空间站点", "攻击力", "43.2%"));
        let parsed = planar.parse_gear(&frame, layout, &refs).unwrap();
        assert_eq!(parsed.observation.main_stat_key, "AttackAddedRatio");
    }

    #[test]
    fn dropped_percent_on_hp_atk_def_is_inferred_without_rewriting_integer_flats() {
        let layout = StatsPanelLayout::MAINTAINED_SEED;
        let frame = generated_panel_frame(Rgb([230, 180, 65]), Rgb([220, 220, 220]));
        let refs = collision_references();

        let mut missing_percent = PanelParser::new(
            ScriptedOcrReader::default()
                .with(OcrField::GearName, ["过客的逢春木簪"])
                .with(OcrField::GearLevel, ["+15"])
                .with(OcrField::GearMainName, ["生命值"])
                .with(OcrField::GearMainValue, ["705"])
                .with(OcrField::GearSubName(0), ["生命值"])
                .with(OcrField::GearSubValue(0), ["3.8"])
                .with(OcrField::GearSubName(1), ["攻击力"])
                .with(OcrField::GearSubValue(1), ["38"])
                .with(OcrField::GearSubName(2), ["防御力"])
                .with(OcrField::GearSubValue(2), ["4.3"])
                .with(OcrField::GearSubName(3), ["防御力"])
                .with(OcrField::GearSubValue(3), ["19"])
                .with(OcrField::GearEquipped, ["装备"]),
        );
        let parsed = missing_percent.parse_gear(&frame, layout, &refs).unwrap();
        assert_eq!(parsed.observation.main_stat_key, "HPDelta");
        assert_eq!(
            parsed
                .observation
                .substats
                .iter()
                .map(|stat| stat.stat_key.as_str())
                .collect::<Vec<_>>(),
            vec![
                "AttackDelta",
                "DefenceAddedRatio",
                "DefenceDelta",
                "HPAddedRatio"
            ]
        );
    }

    #[test]
    fn visible_equivalent_gear_uses_canonical_id_and_authoritative_progression_value() {
        let frame = generated_panel_frame_with_rarity(Rgb([230, 180, 65]), Rgb([220, 220, 220]), 4);
        let reader = ScriptedOcrReader::default()
            .with(OcrField::GearName, ["过客的残绣风衣"])
            .with(OcrField::GearLevel, ["+12"])
            .with(OcrField::GearMainName, ["治疗量加成"])
            .with(OcrField::GearMainValue, ["23.0%"])
            .with(OcrField::GearSubName(0), ["暴击率"])
            .with(OcrField::GearSubValue(0), ["2.9%"])
            .with(OcrField::GearSubName(1), ["速度"])
            .with(OcrField::GearSubValue(1), ["2"])
            .with(OcrField::GearSubName(2), [""])
            .with(OcrField::GearSubValue(2), [""])
            .with(OcrField::GearSubName(3), [""])
            .with(OcrField::GearSubValue(3), [""])
            .with(OcrField::GearEquipped, ["装备"]);
        let parsed = PanelParser::new(reader)
            .parse_gear(
                &frame,
                StatsPanelLayout::MAINTAINED_SEED,
                &gilore_references(),
            )
            .unwrap();
        assert_eq!(parsed.reference.game_id, 51013);
        assert_eq!(parsed.observation.piece_id, 51013);
        assert!(
            (parsed.observation.main_stat_value - 23.0033).abs() < 1e-9,
            "visible 23.0% must export the authoritative progression value"
        );
    }

    #[test]
    fn duplicate_character_and_gear_names_never_select_an_arbitrary_id() {
        let mut character_snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let mut character_variant = character_snapshot.characters[0].clone();
        character_variant.game_id = 1224;
        character_variant.key = "1224".to_string();
        character_variant.path = "Memory".to_string();
        character_snapshot.characters.push(character_variant);
        let character_refs = ReferenceCache::from_snapshot(character_snapshot).unwrap();
        let frame = RgbImage::from_pixel(1920, 1080, Rgb([24, 28, 35]));

        for (header, expected_id) in [("存护 / 三月七", 1001), ("记忆 / 三月七", 1224)] {
            let mut path_parser = PanelParser::new(
                ScriptedOcrReader::default()
                    .with(OcrField::CharacterName, [header])
                    .with(OcrField::CharacterLevel, ["等级 80/80"]),
            );
            assert_eq!(
                path_parser
                    .parse_character_details(&frame, &character_refs, 0)
                    .unwrap()
                    .observation
                    .character_id,
                expected_id
            );
        }

        let mut character_parser = PanelParser::new(
            ScriptedOcrReader::default()
                .with(OcrField::CharacterName, ["三月七"])
                .with(OcrField::CharacterLevel, ["等级 80/80"]),
        );
        assert_eq!(
            character_parser
                .parse_character_details(&frame, &character_refs, 0)
                .unwrap_err()
                .code(),
            "HSR-OCR-CHARACTER-AMBIGUOUS"
        );

        let mut gear_snapshot: ReferenceSnapshot =
            serde_json::from_str(include_str!("../tests/fixtures/reference_cache.json")).unwrap();
        let mut gear_variant = gear_snapshot.gear_pieces[0].clone();
        gear_variant.game_id = 55001;
        gear_variant.key = "55001".to_string();
        gear_snapshot.gear_pieces.push(gear_variant);
        let gear_refs = ReferenceCache::from_snapshot(gear_snapshot).unwrap();
        let mut gear_parser = PanelParser::new(gear_reader("过客的逢春木簪", "生命值", "705"));
        assert_eq!(
            gear_parser
                .parse_gear(
                    &generated_panel_frame(Rgb([230, 180, 65]), Rgb([220, 220, 220])),
                    StatsPanelLayout::MAINTAINED_SEED,
                    &gear_refs,
                )
                .unwrap_err()
                .code(),
            "HSR-OCR-GEAR-AMBIGUOUS"
        );
    }

    #[test]
    fn equipped_parser_is_localized_and_blank_evidence_fails_closed() {
        let refs = references();
        let blank = RgbImage::from_pixel(180, 60, Rgb([24, 28, 35]));
        assert_eq!(parse_equipped("", &blank, &refs), (None, None));
        assert_eq!(parse_equipped("Equip", &blank, &refs), (None, None));

        let mut visible = blank.clone();
        paint_text_evidence(&mut visible, NormRect::new(0.0, 0.0, 1.0, 1.0));
        assert_eq!(parse_equipped("装备", &visible, &refs), (Some(false), None));
        assert_eq!(
            parse_equipped("Equip", &visible, &refs),
            (Some(false), None)
        );
        assert_eq!(
            parse_equipped("装备中 三月七", &visible, &refs),
            (Some(true), Some("1001".to_string()))
        );
        assert_eq!(
            parse_equipped("Equipped March 7th", &visible, &refs),
            (Some(true), Some("1001".to_string()))
        );
    }
}
