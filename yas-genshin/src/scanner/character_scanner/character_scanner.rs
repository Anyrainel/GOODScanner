use std::rc::Rc;
use std::time::SystemTime;

use anyhow::Result;
use image::RgbImage;
use log::{error, info, warn};
use regex::Regex;

use yas::capture::{Capturer, GenericCapturer};
use yas::game_info::GameInfo;
use yas::ocr::{ImageToText, PPOCRModel};
use yas::positioning::Pos;
use yas::system_control::SystemControl;
use yas::utils;
use yas::window_info::{FromWindowInfoRepository, WindowInfoRepository};

use crate::character::{zh_cn_to_good_key, element_from_zh_cn};

use super::CharacterScannerWindowInfo;
use super::GenshinCharacterScannerConfig;
use super::GenshinCharacterScanResult;

pub struct GenshinCharacterScanner {
    config: GenshinCharacterScannerConfig,
    window_info: CharacterScannerWindowInfo,
    game_info: GameInfo,
    capturer: Rc<dyn Capturer<RgbImage>>,
    system_control: SystemControl,
    ocr_model: Box<dyn ImageToText<RgbImage> + Send>,
}

impl GenshinCharacterScanner {
    fn get_image_to_text(backend: &str) -> Result<Box<dyn ImageToText<RgbImage> + Send>> {
        match backend.to_lowercase().as_str() {
            "paddle" | "ppocrv5" => {
                let model_bytes = include_bytes!("./models/PP-OCRv5_mobile_rec.onnx");
                let dict_str = include_str!("./models/ppocrv5_dict.txt");
                let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.trim().to_string()).collect();
                dict_vec.push(String::from(" "));
                let model = PPOCRModel::new(model_bytes, dict_vec)?;
                Ok(Box::new(model))
            },
            "paddlev3" | "ppocrv3" => {
                let model_bytes = include_bytes!("./models/ch_PP-OCRv3_rec_infer.onnx");
                let dict_str = include_str!("./models/ppocr_keys_v1.txt");
                let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.trim().to_string()).collect();
                dict_vec.push(String::from(" "));
                let model = PPOCRModel::new(model_bytes, dict_vec)?;
                Ok(Box::new(model))
            },
            _ => {
                let model_bytes = include_bytes!("./models/PP-OCRv5_mobile_rec.onnx");
                let dict_str = include_str!("./models/ppocrv5_dict.txt");
                let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.trim().to_string()).collect();
                dict_vec.push(String::from(" "));
                let model = PPOCRModel::new(model_bytes, dict_vec)?;
                Ok(Box::new(model))
            }
        }
    }

    fn get_capturer() -> Result<Rc<dyn Capturer<RgbImage>>> {
        Ok(Rc::new(GenericCapturer::new()?))
    }

    pub fn new(
        window_info_repo: &WindowInfoRepository,
        config: GenshinCharacterScannerConfig,
        game_info: GameInfo,
    ) -> Result<Self> {
        let ocr_model = Self::get_image_to_text(&config.ocr_backend)?;

        Ok(Self {
            config,
            window_info: CharacterScannerWindowInfo::from_window_info_repository(
                game_info.window.to_rect_usize().size(),
                game_info.ui,
                game_info.platform,
                window_info_repo,
            )?,
            game_info,
            capturer: Self::get_capturer()?,
            system_control: SystemControl::new(),
            ocr_model,
        })
    }
}

impl GenshinCharacterScanner {
    /// Capture a region relative to the game window origin
    fn capture_region(&self, rect: &yas::positioning::Rect<f64>) -> Result<RgbImage> {
        self.capturer.capture_relative_to(
            rect.to_rect_i32(),
            self.game_info.window.origin(),
        )
    }

    /// Click a position relative to the game window
    fn click_pos(&mut self, pos: &Pos<f64>) {
        let origin = self.game_info.window;
        let x = origin.left as f64 + pos.x;
        let y = origin.top as f64 + pos.y;
        self.system_control.mouse_move_to(x as i32, y as i32).unwrap();
        utils::sleep(50);
        self.system_control.mouse_click().unwrap();
    }

    /// Press a keyboard key
    fn press_key(&mut self, key: enigo::Key) {
        self.system_control.key_press(key).unwrap();
    }

    /// OCR a region and return text
    fn ocr_region(&self, rect: &yas::positioning::Rect<f64>) -> Result<String> {
        let im = self.capture_region(rect)?;
        self.ocr_model.image_to_text(&im, false)
    }

    /// Scan character name and element from the name region.
    /// The region typically shows "Element / CharacterName" in Chinese.
    fn scan_name_and_element(&self) -> Result<(String, String)> {
        let raw = self.ocr_region(&self.window_info.name_rect)?;
        info!("角色名称OCR: {}", raw);

        // Try to parse "元素 / 名字" format or just the name
        // Common formats: "冰·神里绫华", "火/胡桃", "冰/神里绫华"
        let name_str = raw.trim();

        // Try splitting by common separators
        let (element_zh, char_name) = if let Some(idx) = name_str.find('·') {
            let e = &name_str[..idx];
            let n = &name_str[idx + '·'.len_utf8()..];
            (e.trim(), n.trim())
        } else if let Some(idx) = name_str.find('/') {
            let e = &name_str[..idx];
            let n = &name_str[idx + 1..];
            (e.trim(), n.trim())
        } else {
            // No separator found, treat entire text as name
            ("", name_str)
        };

        let element = element_from_zh_cn(element_zh).unwrap_or("").to_string();
        let good_key = zh_cn_to_good_key(char_name);

        if good_key.is_empty() {
            // Try fuzzy match using edit distance
            warn!("无法精确匹配角色名: '{}', 使用原始文本", char_name);
            Ok((char_name.to_string(), element))
        } else {
            Ok((good_key.to_string(), element))
        }
    }

    /// Scan level from "Lv.XX/YY" format, returns (level, ascension)
    fn scan_level(&self) -> Result<(i32, i32)> {
        let raw = self.ocr_region(&self.window_info.level_rect)?;
        info!("角色等级OCR: {}", raw);

        let re = Regex::new(r"(\d+)\s*/\s*(\d+)")?;
        if let Some(caps) = re.captures(&raw) {
            let level: i32 = caps[1].parse().unwrap_or(1);
            let max_level: i32 = caps[2].parse().unwrap_or(20);

            let ascension = ascension_from_max_level(max_level);
            info!("等级: {}, 突破: {}", level, ascension);
            Ok((level, ascension))
        } else {
            // Try just extracting numbers
            let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 2 {
                let level: i32 = digits[..2.min(digits.len())].parse().unwrap_or(1);
                warn!("等级解析不完整，使用: {}", level);
                Ok((level, 0))
            } else {
                warn!("无法解析等级: '{}', 默认为 Lv.1", raw);
                Ok((1, 0))
            }
        }
    }

    /// Scan constellations by clicking each constellation icon and checking activation.
    /// Returns constellation count (0-6).
    fn scan_constellations(&mut self) -> Result<i32> {
        // Click constellation tab
        self.click_pos(&self.window_info.tab_constellation_pos.clone());
        utils::sleep(self.config.tab_delay);

        let constellation_positions = [
            self.window_info.constellation_1_pos.clone(),
            self.window_info.constellation_2_pos.clone(),
            self.window_info.constellation_3_pos.clone(),
            self.window_info.constellation_4_pos.clone(),
            self.window_info.constellation_5_pos.clone(),
            self.window_info.constellation_6_pos.clone(),
        ];

        let mut count = 0;

        for (i, pos) in constellation_positions.iter().enumerate() {
            if utils::is_rmb_down() {
                info!("用户中断命座扫描");
                break;
            }

            self.click_pos(pos);
            utils::sleep(self.config.node_delay);

            // Capture the activate button region and check if constellation is activated
            let im = self.capture_region(&self.window_info.constellation_activate_rect)?;
            let activated = self.is_constellation_activated(&im);

            if self.config.verbose {
                info!("命座 {} {}", i + 1, if activated { "已激活" } else { "未激活" });
            }

            if activated {
                count += 1;
            } else {
                // Once we find an unactivated constellation, all subsequent ones are also unactivated
                break;
            }
        }

        // Press Escape to go back to the main character screen
        self.press_key(enigo::Key::Escape);
        utils::sleep(self.config.tab_delay);

        info!("命座数量: {}", count);
        Ok(count)
    }

    /// Check if a constellation is activated by analyzing the activate button region.
    /// Activated constellations show the constellation details directly;
    /// Unactivated constellations show a bright "Activate" button.
    fn is_constellation_activated(&self, im: &RgbImage) -> bool {
        // Sample the average brightness of the region
        // If there's a bright "Activate" button, the area will be relatively bright/white
        // If the constellation is already activated, the area will be darker
        let pixels = im.as_raw();
        let pixel_count = pixels.len() / 3;
        if pixel_count == 0 {
            return false;
        }

        let mut total_brightness: f64 = 0.0;
        for i in 0..pixel_count {
            let r = pixels[i * 3] as f64;
            let g = pixels[i * 3 + 1] as f64;
            let b = pixels[i * 3 + 2] as f64;
            total_brightness += (r + g + b) / 3.0;
        }

        let avg_brightness = total_brightness / pixel_count as f64;

        if self.config.verbose {
            info!("命座区域平均亮度: {:.1}", avg_brightness);
        }

        // A bright "Activate" button means NOT activated
        // Threshold: if average brightness > 180, the button is visible (not activated)
        avg_brightness <= 180.0
    }

    /// Scan talent levels by clicking each talent and OCR-ing the level number.
    /// Returns (auto, skill, burst) base talent levels.
    fn scan_talents(&mut self) -> Result<(i32, i32, i32)> {
        // Click talents tab
        self.click_pos(&self.window_info.tab_talents_pos.clone());
        utils::sleep(self.config.tab_delay);

        let talent_positions = [
            self.window_info.talent_auto_pos.clone(),
            self.window_info.talent_skill_pos.clone(),
            self.window_info.talent_burst_pos.clone(),
        ];

        let mut levels = [1i32; 3];

        for (i, pos) in talent_positions.iter().enumerate() {
            if utils::is_rmb_down() {
                info!("用户中断天赋扫描");
                break;
            }

            self.click_pos(pos);
            utils::sleep(self.config.node_delay);

            let raw = self.ocr_region(&self.window_info.talent_level_rect)?;
            info!("天赋 {} OCR: {}", i + 1, raw);

            // Extract the level number - look for "Lv.X" or just digits
            let re = Regex::new(r"(?i)(?:lv\.?\s*)?(\d+)")?;
            if let Some(caps) = re.captures(&raw) {
                levels[i] = caps[1].parse().unwrap_or(1);
            } else {
                warn!("无法解析天赋等级: '{}', 默认为 1", raw);
                levels[i] = 1;
            }

            if self.config.verbose {
                info!("天赋 {} 等级: {}", i + 1, levels[i]);
            }
        }

        // Press Escape to go back
        self.press_key(enigo::Key::Escape);
        utils::sleep(self.config.tab_delay);

        info!("天赋等级: 普攻={}, 战技={}, 爆发={}", levels[0], levels[1], levels[2]);
        Ok((levels[0], levels[1], levels[2]))
    }

    /// Navigate to the next character by clicking the right arrow
    fn go_to_next_character(&mut self) {
        self.click_pos(&self.window_info.next_pos.clone());
        utils::sleep(self.config.delay);
    }

    /// Main scan entry point. Opens the character screen and scans all characters.
    pub fn scan(&mut self) -> Result<Vec<GenshinCharacterScanResult>> {
        info!("开始扫描角色，使用鼠标右键中断扫描");

        let now = SystemTime::now();

        // Press C to open character screen
        info!("打开角色界面...");
        self.press_key(enigo::Key::Layout('c'));
        utils::sleep(1500);

        // Click attributes tab first
        self.click_pos(&self.window_info.tab_attributes_pos.clone());
        utils::sleep(self.config.tab_delay);

        let mut results: Vec<GenshinCharacterScanResult> = Vec::new();
        let mut first_character_name = String::new();
        let max_characters = 100; // safety limit

        for idx in 0..max_characters {
            if utils::is_rmb_down() {
                info!("用户中断角色扫描");
                break;
            }

            info!("--- 扫描第 {} 个角色 ---", idx + 1);

            // Scan name and element (from attributes tab view)
            let (name, element) = match self.scan_name_and_element() {
                Ok(v) => v,
                Err(e) => {
                    error!("扫描角色名失败: {}", e);
                    self.go_to_next_character();
                    continue;
                }
            };

            // Loop detection: if we've scanned at least one character and see the first one again
            if idx == 0 {
                first_character_name = name.clone();
            } else if name == first_character_name {
                info!("检测到已回到第一个角色 '{}', 扫描结束", name);
                break;
            }

            // Scan level
            let (level, ascension) = match self.scan_level() {
                Ok(v) => v,
                Err(e) => {
                    error!("扫描角色等级失败: {}", e);
                    (1, 0)
                }
            };

            // Scan constellations
            let constellation = match self.scan_constellations() {
                Ok(v) => v,
                Err(e) => {
                    error!("扫描命座失败: {}", e);
                    0
                }
            };

            // Need to click back to attributes tab first since constellation scanning leaves us elsewhere
            self.click_pos(&self.window_info.tab_attributes_pos.clone());
            utils::sleep(self.config.tab_delay / 2);

            // Scan talents
            let (talent_auto, talent_skill, talent_burst) = match self.scan_talents() {
                Ok(v) => v,
                Err(e) => {
                    error!("扫描天赋失败: {}", e);
                    (1, 1, 1)
                }
            };

            let result = GenshinCharacterScanResult {
                name: name.clone(),
                element: element.clone(),
                level,
                ascension,
                constellation,
                talent_auto,
                talent_skill,
                talent_burst,
            };

            if self.config.verbose {
                info!("角色扫描结果: {:?}", result);
            }

            results.push(result);

            // Navigate to next character
            // Need to be back on attributes tab first
            self.click_pos(&self.window_info.tab_attributes_pos.clone());
            utils::sleep(self.config.tab_delay / 2);

            self.go_to_next_character();
        }

        // Close character screen
        self.press_key(enigo::Key::Escape);
        utils::sleep(500);

        info!("角色扫描完成，共扫描 {} 个角色，耗时: {:?}", results.len(), now.elapsed()?);

        Ok(results)
    }
}

/// Derive ascension phase from the max level shown in "Lv.XX/YY"
fn ascension_from_max_level(max_level: i32) -> i32 {
    match max_level {
        20 => 0,
        40 => 1,
        50 => 2,
        60 => 3,
        70 => 4,
        80 => 5,
        90 => 6,
        _ => 0,
    }
}
