use std::collections::{BTreeMap, VecDeque};

use genshin_scanner::scanner::common::ocr_factory::create_ocr_model;
use image::RgbImage;
use regex::Regex;
use yas::ocr::ImageToText;

use crate::{
    annotator,
    error::{hints, HsrError, HsrResult},
    layout,
    model::{
        CharacterReference, GearReference, ObservedCharacter, ObservedGear, ObservedLightCone,
        ObservedSubstat, StatReference, StatValueKind, TrailblazerIdentity,
    },
    reference::ReferenceCache,
    scanner_export::CharacterDetails,
    vision::NormRect,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OcrField {
    Uid,
    MenuTitle,
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
    CharacterSkill(&'static str),
}

impl OcrField {
    pub fn dump_name(self) -> String {
        match self {
            Self::Uid => "uid".to_string(),
            Self::MenuTitle => "menu_title".to_string(),
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
            Self::CharacterSkill(name) => format!("character_skill_{name}"),
        }
    }
}

pub trait OcrReader {
    fn read(&mut self, field: OcrField, image: &RgbImage) -> HsrResult<String>;
}

/// Picks the recognizer for each field.
pub trait OcrModels {
    fn model(&self, field: OcrField) -> &dyn ImageToText<RgbImage>;
}

type BoxedOcrModel = Box<dyn ImageToText<RgbImage> + Send>;

/// Per-field routing from a live eval against a packet-capture export
/// (`examples/ocr_field_eval.rs`, 80 Character / 300 Relic / 275 Light Cone
/// panels). Character name and trace levels were within one read of v4, and
/// stay on v6 tiny with the other text fields. Fields the eval does not cover
/// stay on v4.
pub struct DefaultOcrModels {
    v4: BoxedOcrModel,
    v5: BoxedOcrModel,
    v6_tiny: BoxedOcrModel,
}

impl DefaultOcrModels {
    pub fn new() -> HsrResult<Self> {
        Ok(Self {
            v4: load_model("ppocrv4")?,
            v5: load_model("ppocrv5")?,
            v6_tiny: load_model("ppocrv6tiny")?,
        })
    }
}

fn load_model(backend: &str) -> HsrResult<BoxedOcrModel> {
    create_ocr_model(backend).map_err(|error| {
        HsrError::new(
            "HSR-OCR-MODEL",
            hints::OCR_FAILED,
            format!("PaddleOCR {backend} initialization failed; cause={error}"),
        )
    })
}

impl OcrModels for DefaultOcrModels {
    fn model(&self, field: OcrField) -> &dyn ImageToText<RgbImage> {
        match field {
            // A light streak from the 3D preview often crosses the watermark;
            // v5 read 13/20 such frames, v4 7/20, v6 tiny 6/20.
            OcrField::Uid => self.v5.as_ref(),
            // v6 tiny: all six Relic fields 300/300 vs v4 299/300; Light Cone
            // names 258 vs 257 (v4 read 广 as 廣); Character levels 79 vs 77.
            // Character names were 77/78 and trace levels were tied at 77/77;
            // both stay on v6 tiny so one model covers the close text fields.
            OcrField::GearName
            | OcrField::GearLevel
            | OcrField::GearMainName
            | OcrField::GearMainValue
            | OcrField::GearSubName(_)
            | OcrField::GearSubValue(_)
            | OcrField::LightConeName
            | OcrField::CharacterLevel
            | OcrField::CharacterName
            | OcrField::CharacterSkill(_) => self.v6_tiny.as_ref(),
            OcrField::MenuTitle
            | OcrField::InventoryQuantity
            | OcrField::GearEquipped
            | OcrField::LightConeLevel
            | OcrField::LightConeSuperimposition
            | OcrField::LightConeEquipped => self.v4.as_ref(),
        }
    }
}

pub struct PaddleOcrReader<M = DefaultOcrModels> {
    models: M,
}

impl PaddleOcrReader {
    pub fn new() -> HsrResult<Self> {
        DefaultOcrModels::new().map(Self::with_models)
    }
}

impl<M: OcrModels> PaddleOcrReader<M> {
    pub fn with_models(models: M) -> Self {
        Self { models }
    }
}

impl<M: OcrModels> OcrReader for PaddleOcrReader<M> {
    fn read(&mut self, field: OcrField, image: &RgbImage) -> HsrResult<String> {
        let model = self.models.model(field);
        let infer = |image: &RgbImage| {
            genshin_scanner::scanner::common::annotator::observe_ocr(
                &field.dump_name(),
                image,
                || model.image_to_text(image, false),
            )
            .map_err(|error| {
                HsrError::new(
                    "HSR-OCR-INFERENCE",
                    hints::OCR_FAILED,
                    format!("OCR inference failed; cause={error}"),
                )
            })
        };
        let text = infer(image)?.trim().to_string();
        if !text.is_empty() {
            return Ok(text);
        }
        // Retry only empty results. A successful first read is never replaced.
        yas::log_debug!(
            "OCR {} 第一次为空，正在重试同一裁剪。",
            "OCR {} was empty on the first read; retrying the same crop.",
            field.dump_name()
        );
        Ok(infer(image)?.trim().to_string())
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
/// as actionable. A crop that matches a live icon cluster reports `1.0`.
/// A crop that matches none reports `0.0`.
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
        let rarity =
            detect_rarity(&rarity_crop).or_else(|| detect_rarity_from_name_color(&name_color_crop));
        annotator::set_final(
            "rarity_stars",
            &rarity.map(|value| value.to_string()).unwrap_or_default(),
        );
        let rarity = rarity.ok_or_else(|| ocr_semantic("gear rarity unreadable"))?;
        let level_text = self.read_crop(OcrField::GearLevel, frame, layout, layout::RELIC_LEVEL)?;
        // "+0" is white on the card. The recognizer often turns the plus into a
        // leading 4 ("40") or returns nothing. Enhanced pieces ("+15") already
        // parse, so the second read only runs when the first text is unusable.
        let level = match parse_gear_level(&level_text) {
            Some(level) => level,
            None => {
                let boosted = dark_glyphs_on_white(&layout.crop(frame, layout::RELIC_LEVEL)?);
                let retry = self.reader.read(OcrField::GearLevel, &boosted)?;
                annotator::record_ocr(
                    "gear_level",
                    layout.panel.relative(layout::RELIC_LEVEL),
                    &retry,
                );
                parse_gear_level(&retry).ok_or_else(|| {
                    ocr_semantic(format!(
                        "gear level was missing or outside 0..=15; text={level_text:?}; retry={retry:?}"
                    ))
                })?
            },
        };

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
            let value_image = layout.crop(frame, layout::relic_sub_value(index))?;
            // Lines are a contiguous prefix. A blank value column is the end of
            // the list; the set-bonus paragraph below it is not another substat.
            if !substat_value_ink(&value_image) {
                break;
            }
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
            if value.trim().is_empty() {
                break;
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
        let discard_rect = layout.discard_button();
        let discard_crop = discard_rect.crop(frame)?;
        let (discard, discard_confidence) = detect_discard_state(&discard_crop);

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
        let reference = references.resolve_light_cone_name(&name).ok_or_else(|| {
            ocr_semantic(format!(
                "Light Cone name did not resolve uniquely; text={name}"
            ))
        })?;
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
        trailblazer: Option<&TrailblazerIdentity>,
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
        let reference = references
            .resolve_character_header(character_name, path_text, trailblazer)
            .cloned()
            .ok_or_else(|| {
                ocr_resolution(
                    "HSR-OCR-CHARACTER-AMBIGUOUS",
                    format!("character header {name_text:?} did not resolve uniquely; duplicate variants need readable path evidence and the Trailblazer needs the configured nickname and gender"),
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

    /// Whether the top-left menu title contains any of `needles` (lowercase).
    pub fn shows_menu_title(&mut self, frame: &RgbImage, needles: &[&str]) -> HsrResult<bool> {
        let text = self
            .reader
            .read(OcrField::MenuTitle, &layout::MENU_TITLE.crop(frame)?)?;
        annotator::record_ocr(
            OcrField::MenuTitle.dump_name().as_str(),
            layout::MENU_TITLE,
            &text,
        );
        let text = text.to_lowercase();
        Ok(needles.iter().any(|needle| text.contains(needle)))
    }

    pub fn read_uid(&mut self, frame: &RgbImage) -> HsrResult<Option<u64>> {
        let text = self.reader.read(OcrField::Uid, &layout::UID.crop(frame)?)?;
        annotator::record_ocr(OcrField::Uid.dump_name().as_str(), layout::UID, &text);
        Ok(parse_uid(&text))
    }

    pub fn parse_character_traces(
        &mut self,
        frame: &RgbImage,
        path: &str,
    ) -> HsrResult<CharacterDetails> {
        let layout = layout::traces_for_path(path)
            .ok_or_else(|| ocr_semantic(format!("no traces layout for path={path}")))?;
        let mut skills = BTreeMap::new();
        let mut memosprite = BTreeMap::new();
        for &(key, x, y) in layout.skills {
            let rect = layout::skill_level_rect(key, x, y);
            let crop = rect.crop(frame)?;
            let field = OcrField::CharacterSkill(key);
            let text = self.reader.read(field, &crop)?;
            annotator::record_ocr(&field.dump_name(), rect, &text);
            let level = parse_trace_level(&text, key)?;
            if let Some(memo_key) = key.strip_prefix("memosprite_") {
                memosprite.insert(memo_key.to_string(), u32::from(level));
            } else {
                skills.insert(key.to_string(), u32::from(level));
            }
        }
        let mut traces = BTreeMap::new();
        for &(key, x, y) in layout.unlocks {
            let unlocked = if key.starts_with("ability_") {
                ability_node_unlocked(frame, x, y)
            } else {
                stat_node_unlocked(frame, x, y)
            };
            traces.insert(key.to_string(), unlocked);
        }
        Ok(CharacterDetails {
            ability_version: None,
            skills,
            traces,
            memosprite: (!memosprite.is_empty()).then_some(memosprite),
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

/// The `UID` prefix can OCR as `U1D`, so only a single nine- or ten-digit run
/// counts; a stray prefix digit stays a separate run.
fn parse_uid(text: &str) -> Option<u64> {
    let runs: Vec<&str> = text
        .split(|character: char| !character.is_ascii_digit())
        .filter(|run| (9..=10).contains(&run.len()))
        .collect();
    match runs.as_slice() {
        [uid] => uid.parse().ok(),
        _ => None,
    }
}

/// Eidolons raise both the displayed level and its maximum (`8/12`, `5/7`).
/// Exports carry the base level, so the bonus `shown_max - base_max` is
/// removed. OCR often reads the slash as `1` (`6/6` → `616`); a slashless
/// reading is accepted only when exactly one `1` split yields a legal pair.
fn parse_trace_level(text: &str, key: &str) -> HsrResult<u8> {
    let base_max: u8 = match key {
        "basic" | "memosprite_skill" | "memosprite_talent" => 6,
        _ => 10,
    };
    let legal = |shown: u8, shown_max: u8| {
        (base_max..=base_max + 2).contains(&shown_max)
            && shown >= 1 + (shown_max - base_max)
            && shown <= shown_max
    };
    let compact: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '/')
        .collect();
    let pairs: Vec<(u8, u8)> = match compact.split_once('/') {
        Some((shown, shown_max)) => shown
            .parse()
            .ok()
            .zip(shown_max.parse().ok())
            .into_iter()
            .collect(),
        None => compact
            .char_indices()
            .filter(|&(index, c)| c == '1' && index > 0 && index + 1 < compact.len())
            .filter_map(|(index, _)| {
                Some((
                    compact[..index].parse().ok()?,
                    compact[index + 1..].parse().ok()?,
                ))
            })
            .filter(|&(shown, shown_max)| legal(shown, shown_max))
            .collect(),
    };
    match pairs.as_slice() {
        [(shown, shown_max)] if legal(*shown, *shown_max) => Ok(shown - (shown_max - base_max)),
        _ => Err(ocr_semantic(format!(
            "trace level for {key} is not a legal level/max pair; got {text:?}"
        ))),
    }
}

/// Unlocked stat nodes are white disks around a dark icon; some icons (the
/// Hunt shield and heart) cover the whole center, so brightness at the center
/// is unreliable. Within 20px, 623 live unlocked nodes were ≥35% white pixels
/// and 157 locked (gray) ones ≤16%.
fn stat_node_unlocked(image: &RgbImage, x: f64, y: f64) -> bool {
    annotator::record_node("trace_stat_node", image, x, y, 21.0 / 1080.0);
    const RADIUS: f64 = 20.0 / 1080.0;
    const MIN_WHITE_FRACTION: f64 = 0.25;
    let height = f64::from(image.height());
    let (cx, cy) = (x * f64::from(image.width()), y * height);
    let reach = (RADIUS * height).round() as i32;
    let (mut white, mut count) = (0_u32, 0_u32);
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            if dx * dx + dy * dy > reach * reach {
                continue;
            }
            if let Some(pixel) = pixel_at(image, cx + f64::from(dx), cy + f64::from(dy)) {
                white += u32::from(pixel.iter().all(|&channel| channel > 200));
                count += 1;
            }
        }
    }
    count > 0 && f64::from(white) / f64::from(count) >= MIN_WHITE_FRACTION
}

/// Ability nodes are large icons whose center brightness depends on the art.
/// Unlocked ones carry a white ring at r≈18px: across 78 live Characters the
/// best white fraction was ≥0.35 when unlocked and ≤0.19 when locked.
fn ability_node_unlocked(image: &RgbImage, x: f64, y: f64) -> bool {
    annotator::record_node("trace_ability_node", image, x, y, 23.0 / 1080.0);
    const RADIUS: f64 = 18.0 / 1080.0;
    const MIN_WHITE_FRACTION: f64 = 0.3;
    const SAMPLES: usize = 72;
    const SHIFT: i32 = 3;
    let height = f64::from(image.height());
    let (cx, cy) = (x * f64::from(image.width()), y * height);
    let radius = RADIUS * height;
    let mut best = 0.0_f64;
    for dy in -SHIFT..=SHIFT {
        for dx in -SHIFT..=SHIFT {
            let white = (0..SAMPLES)
                .filter(|&k| {
                    let angle = std::f64::consts::TAU * k as f64 / SAMPLES as f64;
                    pixel_at(
                        image,
                        cx + f64::from(dx) + radius * angle.cos(),
                        cy + f64::from(dy) + radius * angle.sin(),
                    )
                    .is_some_and(|pixel| pixel.iter().all(|&channel| channel > 190))
                })
                .count();
            best = best.max(white as f64 / SAMPLES as f64);
        }
    }
    best >= MIN_WHITE_FRACTION
}

fn pixel_at(image: &RgbImage, x: f64, y: f64) -> Option<[u8; 3]> {
    let (x, y) = (x.round(), y.round());
    if x < 0.0 || y < 0.0 || x >= f64::from(image.width()) || y >= f64::from(image.height()) {
        return None;
    }
    Some(image.get_pixel(x as u32, y as u32).0)
}

/// Relic levels are `+0` through `+15`. A leading plus is often read as `4`,
/// so `+0` arrives as `40` and `+12` as `412`.
fn substat_value_ink(image: &RgbImage) -> bool {
    annotator::record_detection("substat_value_ink", image);
    let ink = image
        .pixels()
        .filter(|pixel| {
            let [r, g, b] = pixel.0;
            let white = r > 200 && g > 200 && b > 200;
            let orange = r > 175 && g > 100 && r > b.saturating_add(40);
            white || orange
        })
        .count();
    ink >= 12
}

fn parse_gear_level(text: &str) -> Option<u8> {
    let regex = Regex::new(r"\d+").expect("static regex");
    let numbers: Vec<u32> = regex
        .find_iter(text)
        .filter_map(|capture| capture.as_str().parse().ok())
        .collect();
    if let Some(value) = numbers.iter().copied().find(|value| *value <= 15) {
        return Some(value as u8);
    }
    for value in numbers {
        let digits = value.to_string();
        let Some(rest) = digits.strip_prefix('4') else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        if let Ok(level) = rest.parse::<u32>() {
            if level <= 15 {
                return Some(level as u8);
            }
        }
    }
    // The white "+0" glyph is read as "+古" (or "+口" after a contrast retry).
    // A real level is "+1"…"+"15", so a plus and one zero-like character is 0.
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('+') {
        let rest = rest.trim();
        const ZERO_GLYPHS: &[char] = &['古', '口', '〇', '○', '日', '曰', 'Ｏ', 'O', 'o', '０'];
        if rest.chars().count() == 1 && ZERO_GLYPHS.contains(&rest.chars().next().unwrap_or('\0')) {
            return Some(0);
        }
    }
    None
}

fn dark_glyphs_on_white(image: &RgbImage) -> RgbImage {
    let mut output = image.clone();
    for pixel in output.pixels_mut() {
        let [r, g, b] = pixel.0;
        let glyph = r > 210 && g > 210 && b > 210;
        pixel.0 = if glyph { [0, 0, 0] } else { [255, 255, 255] };
    }
    output
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
    let runs: Vec<&str> = regex
        .find_iter(text)
        .map(|capture| capture.as_str())
        .collect();
    let first = runs
        .first()
        .ok_or_else(|| ocr_semantic("level was unreadable"))?;
    let (level, cap) = match first.parse::<u8>().ok().filter(|level| *level <= 100) {
        Some(level) => (
            level,
            runs.get(1)
                .and_then(|cap| cap.parse().ok())
                .unwrap_or_else(|| infer_cap(level)),
        ),
        None => {
            split_misread_level_slash(first).ok_or_else(|| ocr_semantic("level was unreadable"))?
        },
    };
    if !(1..=100).contains(&level) || !(20..=100).contains(&cap) || level > cap {
        return Err(ocr_semantic("level/cap values are mechanically invalid"));
    }
    Ok((level, cap))
}

/// The dim cap after the slash is often truncated, and the slash itself is
/// either dropped (`1/20` → `120`) or read as `7` or `1` (`80/80` → `8078`).
/// Accept a split only when exactly one reading gives a level inside its
/// ascension band (cap−10..=cap) and a cap whose visible prefix matches.
fn split_misread_level_slash(digits: &str) -> Option<(u8, u8)> {
    const CAPS: [u8; 7] = [20, 30, 40, 50, 60, 70, 80];
    let mut readings = (1..digits.len().min(3))
        .flat_map(|at| {
            let slash_misread = matches!(digits.as_bytes()[at], b'7' | b'1');
            [(at, at), (at, at + 1)]
                .into_iter()
                .filter(move |&(end, resume)| resume == end || slash_misread)
        })
        .flat_map(|(end, resume)| {
            let level = digits[..end].parse::<u8>().ok();
            let suffix = &digits[resume..];
            CAPS.iter().filter_map(move |&cap| {
                let level = level?;
                let floor = if cap == 20 { 1 } else { cap - 10 };
                ((floor..=cap).contains(&level) && cap.to_string().starts_with(suffix))
                    .then_some((level, cap))
            })
        });
    let reading = readings.next()?;
    readings.next().is_none().then_some(reading)
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

/// Closed versus open padlock.
///
/// On relics the closed lock is a white keycap with a black body, and the
/// open lock is a thin light stroke on the gold card (or a dim tan outline
/// once the relic is marked for discard). On light cones both states are a
/// white glyph on a black disk; the closed body fills about three times as
/// many pixels as the open outline. Gold from the relic card is not a lock.
pub fn detect_icon_state(image: &RgbImage) -> (Option<bool>, f64) {
    genshin_scanner::scanner::common::annotator::observe_detection("lock_icon", image, || {
        detect_icon_pixels(image)
    })
}

fn detect_icon_pixels(image: &RgbImage) -> (Option<bool>, f64) {
    if image.as_raw().is_empty() {
        return (None, 0.0);
    }
    let mut white = 0_usize;
    let mut dark = 0_usize;
    let mut gold = 0_usize;
    for pixel in image.pixels() {
        let [r, g, b] = pixel.0;
        if r > 200 && g > 200 && b > 200 {
            white += 1;
        }
        if r < 48 && g < 48 && b < 48 {
            dark += 1;
        }
        if r > 155 && g > 105 && r > b.saturating_add(35) && g > b.saturating_add(15) {
            gold += 1;
        }
    }
    let total = (image.width() as usize * image.height() as usize).max(1) as f64;
    let white_ratio = white as f64 / total;
    let dark_ratio = dark as f64 / total;
    let gold_ratio = gold as f64 / total;
    if white_ratio >= 0.22 && dark_ratio >= 0.04 {
        return (Some(true), 1.0);
    }
    if (0.08..0.22).contains(&white_ratio) {
        return (Some(false), 1.0);
    }
    // Discarded relics draw the open padlock in the card's own tan, with no
    // white keycap. A flat gold or flat white crop has no outline.
    if white_ratio < 0.08 && gold_ratio >= 0.45 && edge_density(image) >= 0.05 {
        return (Some(false), 1.0);
    }
    (None, 0.0)
}

/// Trash-can mark.
///
/// Gray is the muted can on a locked relic: discard stays off until the relic
/// is unlocked. White is the same can on an unlocked relic that is not marked;
/// the button is available. A red can on a white disk is the marked state.
/// The gold card around the can is not a mark.
pub fn detect_discard_state(image: &RgbImage) -> (Option<bool>, f64) {
    genshin_scanner::scanner::common::annotator::observe_detection("discard_icon", image, || {
        detect_discard_pixels(image)
    })
}

fn detect_discard_pixels(image: &RgbImage) -> (Option<bool>, f64) {
    if image.as_raw().is_empty() {
        return (None, 0.0);
    }
    let mut red = 0_usize;
    let mut gray = 0_usize;
    let mut white = 0_usize;
    for pixel in image.pixels() {
        let [r, g, b] = pixel.0;
        if r > 150 && g < 110 && b < 110 && r > g.saturating_add(40) {
            red += 1;
            continue;
        }
        if r > 185 && g > 185 && b > 185 {
            white += 1;
            continue;
        }
        if r.abs_diff(g) < 18 && g.abs_diff(b) < 18 && (70..180).contains(&r) {
            gray += 1;
        }
    }
    let total = (image.width() as usize * image.height() as usize).max(1) as f64;
    let red_ratio = red as f64 / total;
    let gray_ratio = gray as f64 / total;
    let white_ratio = white as f64 / total;
    if red_ratio >= 0.06 {
        return (Some(true), 1.0);
    }
    if gray_ratio >= 0.10 || white_ratio >= 0.08 {
        return (Some(false), 1.0);
    }
    (None, 0.0)
}

fn edge_density(image: &RgbImage) -> f64 {
    annotator::record_detection("glyph_edge_density", image);
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
    annotator::record_detection("rarity_stars", image);
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
    annotator::record_detection("rarity_name_color", image);
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

    #[test]
    fn uid_reads_the_single_long_digit_run() {
        assert_eq!(parse_uid("UID:600732506"), Some(600_732_506));
        assert_eq!(parse_uid("U1D 600732506"), Some(600_732_506));
        assert_eq!(parse_uid("UID:1234567890"), Some(1_234_567_890));
        assert_eq!(parse_uid("UID:6007325"), None);
        assert_eq!(parse_uid(""), None);
    }

    #[test]
    fn trace_level_removes_the_eidolon_bonus_and_recovers_a_misread_slash() {
        assert_eq!(parse_trace_level("6/6", "basic").unwrap(), 6);
        assert_eq!(parse_trace_level("5/7", "basic").unwrap(), 4);
        assert_eq!(parse_trace_level("9/12", "skill").unwrap(), 7);
        assert_eq!(parse_trace_level("10/10", "ult").unwrap(), 10);
        assert_eq!(parse_trace_level("12/12", "talent").unwrap(), 10);
        assert_eq!(parse_trace_level("616", "basic").unwrap(), 6);
        assert_eq!(parse_trace_level("417", "basic").unwrap(), 3);
        assert_eq!(parse_trace_level("10110", "skill").unwrap(), 10);
        assert!(parse_trace_level("7/6", "basic").is_err());
        assert!(parse_trace_level("1/12", "skill").is_err());
        assert!(parse_trace_level("66", "basic").is_err());
    }

    #[test]
    fn gear_level_treats_a_leading_four_as_a_misread_plus() {
        assert_eq!(parse_gear_level("+15"), Some(15));
        assert_eq!(parse_gear_level("15"), Some(15));
        assert_eq!(parse_gear_level("0"), Some(0));
        assert_eq!(parse_gear_level("+0"), Some(0));
        assert_eq!(parse_gear_level("40"), Some(0));
        assert_eq!(parse_gear_level("412"), Some(12));
        assert_eq!(parse_gear_level("415"), Some(15));
        assert_eq!(parse_gear_level("+古"), Some(0));
        assert_eq!(parse_gear_level("+口"), Some(0));
        assert_eq!(parse_gear_level(""), None);
        assert_eq!(parse_gear_level("99"), None);
    }

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

    fn rect_bounds(image: &RgbImage, rect: NormRect) -> (u32, u32, u32, u32) {
        let x0 = (rect.x * image.width() as f64).floor() as u32;
        let y0 = (rect.y * image.height() as f64).floor() as u32;
        let x1 = ((rect.x + rect.width) * image.width() as f64)
            .ceil()
            .min(image.width() as f64) as u32;
        let y1 = ((rect.y + rect.height) * image.height() as f64)
            .ceil()
            .min(image.height() as f64) as u32;
        (x0, y0, x1, y1)
    }

    fn paint_closed_padlock(image: &mut RgbImage, rect: NormRect) {
        paint_rect(image, rect, Rgb([210, 168, 90]));
        let (x0, y0, x1, y1) = rect_bounds(image, rect);
        let width = x1.saturating_sub(x0);
        let height = y1.saturating_sub(y0);
        let inset_x = width / 5;
        let inset_y = height / 5;
        for y in y0 + inset_y..y1.saturating_sub(inset_y) {
            for x in x0 + inset_x..x1.saturating_sub(inset_x) {
                image.put_pixel(x, y, Rgb([236, 236, 236]));
            }
        }
        let body_x = width / 3;
        let body_y = height / 3;
        for y in y0 + body_y..y1.saturating_sub(body_y) {
            for x in x0 + body_x..x1.saturating_sub(body_x) {
                image.put_pixel(x, y, Rgb([18, 18, 18]));
            }
        }
    }

    fn paint_open_padlock(image: &mut RgbImage, rect: NormRect) {
        paint_rect(image, rect, Rgb([12, 12, 16]));
        let (x0, y0, x1, y1) = rect_bounds(image, rect);
        let band = ((y1 - y0) / 8).max(1);
        for y in y0..y0 + band {
            for x in x0..x1 {
                image.put_pixel(x, y, Rgb([230, 230, 230]));
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
        for index in 0..4 {
            paint_text_evidence(
                &mut frame,
                layout.panel.relative(layout::relic_sub_value(index)),
            );
        }
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
    fn level_recovers_a_slash_read_as_a_digit() {
        assert_eq!(parse_level_and_cap("等级8078.").unwrap(), (80, 80));
        assert_eq!(parse_level_and_cap("等级70780").unwrap(), (70, 80));
        assert_eq!(parse_level_and_cap("等级6017").unwrap(), (60, 70));
        assert_eq!(parse_level_and_cap("等级807").unwrap(), (80, 80));
        assert_eq!(parse_level_and_cap("等级：120.").unwrap(), (1, 20));
        assert_eq!(parse_level_and_cap("等级2020").unwrap(), (20, 20));
        assert!(parse_level_and_cap("等级4078").is_err());
    }

    #[test]
    fn character_traces_read_visible_levels_and_unlock_pixels() {
        let frame = RgbImage::from_pixel(1920, 1080, Rgb([255, 255, 255]));
        let mut parser = PanelParser::new(
            ScriptedOcrReader::default()
                .with(OcrField::CharacterSkill("basic"), ["1/6"])
                .with(OcrField::CharacterSkill("skill"), ["5/10"])
                .with(OcrField::CharacterSkill("ult"), ["8/10"])
                .with(OcrField::CharacterSkill("talent"), ["8/10"]),
        );
        let details = parser.parse_character_traces(&frame, "Knight").unwrap();
        assert_eq!(details.skills["basic"], 1);
        assert_eq!(details.skills["skill"], 5);
        assert!(details.ability_version.is_none());
        assert_eq!(details.traces.len(), 13);
        assert!(details.traces.values().all(|unlocked| *unlocked));
        assert!(details.memosprite.is_none());
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
    fn live_lock_keycap_is_locked_even_on_a_gold_card() {
        let mut image = RgbImage::from_pixel(40, 40, Rgb([210, 168, 90]));
        for y in 8..32 {
            for x in 8..32 {
                image.put_pixel(x, y, Rgb([236, 236, 236]));
            }
        }
        for y in 16..26 {
            for x in 16..24 {
                image.put_pixel(x, y, Rgb([18, 18, 18]));
            }
        }
        let (state, confidence) = detect_icon_state(&image);
        assert_eq!(state, Some(true));
        assert!(confidence >= 0.6, "confidence={confidence}");
    }

    #[test]
    fn icon_state_preserves_unknown() {
        let blank = RgbImage::from_pixel(40, 40, Rgb([30, 30, 30]));
        assert_eq!(detect_icon_state(&blank).0, None);
        let gold = RgbImage::from_pixel(40, 40, Rgb([220, 170, 70]));
        assert_eq!(detect_icon_state(&gold).0, None);
        let white = RgbImage::from_pixel(40, 40, Rgb([220, 220, 220]));
        assert_eq!(detect_icon_state(&white).0, None);
    }

    #[test]
    fn open_padlock_is_unlocked_on_relics_and_light_cones() {
        let mut relic = RgbImage::from_pixel(40, 40, Rgb([210, 168, 90]));
        for y in 0..5 {
            for x in 0..40 {
                relic.put_pixel(x, y, Rgb([230, 230, 230]));
            }
        }
        let (state, confidence) = detect_icon_state(&relic);
        assert_eq!(state, Some(false));
        assert!(confidence >= MANAGED_ICON_CONFIDENCE_THRESHOLD);

        let mut cone = RgbImage::from_pixel(40, 40, Rgb([12, 12, 16]));
        for y in 0..5 {
            for x in 0..40 {
                cone.put_pixel(x, y, Rgb([230, 230, 230]));
            }
        }
        assert_eq!(detect_icon_state(&cone).0, Some(false));

        let mut dim = RgbImage::from_pixel(40, 40, Rgb([210, 168, 90]));
        for y in 8..32 {
            for x in 8..32 {
                if x < 12 || x >= 28 || y < 12 || y >= 28 {
                    dim.put_pixel(x, y, Rgb([120, 90, 40]));
                }
            }
        }
        assert_eq!(detect_icon_state(&dim).0, Some(false));
    }

    #[test]
    fn five_star_card_gold_around_a_gray_trash_is_not_discarded() {
        let mut image = RgbImage::from_pixel(40, 40, Rgb([220, 170, 70]));
        for y in 10..30 {
            for x in 10..30 {
                image.put_pixel(x, y, Rgb([90, 90, 90]));
            }
        }
        assert_eq!(detect_discard_state(&image).0, Some(false));

        let gold_only = RgbImage::from_pixel(40, 40, Rgb([220, 170, 70]));
        assert_eq!(detect_discard_state(&gold_only).0, None);

        let mut white_can = RgbImage::from_pixel(40, 40, Rgb([210, 168, 90]));
        for y in 10..30 {
            for x in 16..24 {
                white_can.put_pixel(x, y, Rgb([230, 230, 230]));
            }
        }
        assert_eq!(detect_discard_state(&white_can).0, Some(false));

        let mut marked = RgbImage::from_pixel(40, 40, Rgb([236, 236, 236]));
        for y in 12..28 {
            for x in 14..26 {
                marked.put_pixel(x, y, Rgb([210, 40, 40]));
            }
        }
        let (state, confidence) = detect_discard_state(&marked);
        assert_eq!(state, Some(true));
        assert!(confidence >= MANAGED_ICON_CONFIDENCE_THRESHOLD);
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
            .parse_character_details(&frame, &references(), None, 6)
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
        paint_open_padlock(&mut frame, layout.lock_button(InventoryKind::LightCone));
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
        let mut frame = generated_panel_frame(Rgb([230, 180, 65]), Rgb([220, 220, 220]));
        paint_closed_padlock(&mut frame, layout.lock_button(InventoryKind::Gear));
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
                    .parse_character_details(&frame, &character_refs, None, 0)
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
                .parse_character_details(&frame, &character_refs, None, 0)
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
