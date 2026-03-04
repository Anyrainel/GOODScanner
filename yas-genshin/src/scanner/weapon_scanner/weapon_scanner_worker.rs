use std::collections::{HashSet, BTreeMap};
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use image::Rgb;
use image::{GenericImageView, RgbImage};
use log::{error, info, warn};
use indicatif::{ProgressBar, ProgressStyle};

use yas::ocr::ImageToText;
use yas::ocr::PPOCRModel;
use yas::positioning::{Pos, Rect};
use yas::utils::color_distance;

use crate::scanner::weapon_scanner::weapon_scanner_window_info::WeaponScannerWindowInfo;
use crate::scanner::weapon_scanner::GenshinWeaponScannerConfig;
use crate::scanner::weapon_scanner::message_items::WeaponSendItem;
use crate::scanner::weapon_scanner::scan_result::GenshinWeaponScanResult;

/// Save image for debugging purposes
fn save_debug_image(image: &RgbImage, weapon_index: usize, region_tag: &str) -> Result<()> {
    let debug_dir = Path::new("./debug_images");
    if !debug_dir.exists() {
        std::fs::create_dir_all(debug_dir)?;
    }

    let full_filename = format!("weapon_{}_{}.png", weapon_index, region_tag);
    let image_path = debug_dir.join(full_filename);

    image.save(&image_path)?;
    info!("Debug image saved: {}", image_path.display());

    Ok(())
}

/// run in a separate thread, accept captured image and get a weapon
pub struct WeaponScannerWorker {
    model: Box<dyn ImageToText<RgbImage> + Send + Sync>,
    paddle_model: Box<dyn ImageToText<RgbImage> + Send + Sync>,
    window_info: WeaponScannerWindowInfo,
    config: GenshinWeaponScannerConfig,
}

impl WeaponScannerWorker {
    fn get_model_for_backend(backend: &str) -> Result<Box<dyn ImageToText<RgbImage> + Send + Sync>> {
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

    pub fn new(
        window_info: WeaponScannerWindowInfo,
        config: GenshinWeaponScannerConfig,
    ) -> Result<Self> {
        let model = Self::get_model_for_backend(&config.ocr_backend)?;
        let paddle_model = Self::get_model_for_backend("paddle")?;

        Ok(WeaponScannerWorker {
            model,
            paddle_model,
            window_info,
            config,
        })
    }

    /// the captured_img is a panel of the weapon, the rect is a region of the panel
    fn model_inference(&self, rect: Rect<f64>, captured_img: &RgbImage, weapon_index: usize, region_tag: &str) -> Result<String> {
        let relative_rect = rect.translate(Pos {
            x: -self.window_info.panel_rect.left,
            y: -self.window_info.panel_rect.top,
        });

        let raw_img = captured_img.view(
            relative_rect.left as u32, relative_rect.top as u32, relative_rect.width as u32, relative_rect.height as u32,
        ).to_image();

        if self.config.save_images {
            if let Err(e) = save_debug_image(&raw_img, weapon_index, region_tag) {
                warn!("Failed to save region debug image: {}", e);
            }
        }

        let inference_result = self.model.image_to_text(&raw_img, false);

        match inference_result {
            Ok(text) => Ok(text),
            Err(e) => {
                error!("OCR识别失败: weapon_index={}, region_tag='{}', 错误: {:?}", weapon_index, region_tag, e);
                Err(e)
            }
        }
    }

    /// Use paddle model for inference (better for Chinese text like title and equip)
    fn paddle_inference(&self, rect: Rect<f64>, captured_img: &RgbImage, weapon_index: usize, region_tag: &str) -> Result<String> {
        let relative_rect = rect.translate(Pos {
            x: -self.window_info.panel_rect.left,
            y: -self.window_info.panel_rect.top,
        });

        let raw_img = captured_img.view(
            relative_rect.left as u32, relative_rect.top as u32, relative_rect.width as u32, relative_rect.height as u32,
        ).to_image();

        if self.config.save_images {
            if let Err(e) = save_debug_image(&raw_img, weapon_index, region_tag) {
                warn!("Failed to save region debug image: {}", e);
            }
        }

        let inference_result = self.paddle_model.image_to_text(&raw_img, false);

        match inference_result {
            Ok(text) => Ok(text),
            Err(e) => {
                error!("OCR识别失败: weapon_index={}, region_tag='{}', 错误: {:?}", weapon_index, region_tag, e);
                Err(e)
            }
        }
    }

    /// Parse the captured result (of type WeaponSendItem) to a scanned weapon
    fn scan_item_image(&self, item: WeaponSendItem, lock: bool, weapon_index: usize) -> Result<GenshinWeaponScanResult> {
        let image = &item.panel_image;

        // Use PaddleOCR for title (weapon name) recognition
        let str_title = self.paddle_inference(self.window_info.title_rect, image, weapon_index, "title")?;

        let str_level = self.model_inference(self.window_info.level_rect, image, weapon_index, "level")?;
        let str_refinement = self.model_inference(self.window_info.refinement_rect, image, weapon_index, "refinement")?;

        // Use PaddleOCR for equip (character name) recognition
        let str_equip = {
            let relative_rect = self.window_info.equip_rect.translate(Pos {
                x: -self.window_info.panel_rect.left,
                y: -self.window_info.panel_rect.top,
            });
            let raw_img = image.view(
                relative_rect.left as u32,
                relative_rect.top as u32,
                relative_rect.width as u32,
                relative_rect.height as u32,
            ).to_image();

            if self.config.save_images {
                if let Err(e) = save_debug_image(&raw_img, weapon_index, "equip") {
                    warn!("Failed to save region debug image: {}", e);
                }
            }

            // Resize equip region to fixed size (320x48) to match preprocessing
            let fixed = image::imageops::resize(&raw_img, 320, 48, image::imageops::FilterType::Triangle);
            self.paddle_model.image_to_text(&fixed, true)?
        };

        Ok(GenshinWeaponScanResult {
            name: str_title,
            level: str_level,
            refinement: str_refinement,
            equip: str_equip,
            star: item.star as i32,
            lock,
            index: weapon_index,
        })
    }

    /// Get all lock state from a list image
    fn get_page_locks(&self, list_image: &RgbImage) -> Vec<bool> {
        let mut result = Vec::new();

        let row = self.window_info.row;
        let col = self.window_info.col;
        let gap = self.window_info.item_gap_size;
        let size = self.window_info.item_size;
        let lock_pos = self.window_info.lock_pos;

        if self.config.save_images {
            if let Err(e) = save_debug_image(list_image, 0, "list_page") {
                warn!("Failed to save list page debug image: {}", e);
            }
        }

        for r in 0..row {
            if ((gap.height + size.height) * (r as f64)) as u32 > list_image.height() {
                break;
            }
            for c in 0..col {
                let pos_x = (gap.width + size.width) * (c as f64) + lock_pos.x;
                let pos_y = (gap.height + size.height) * (r as f64) + lock_pos.y;

                let mut locked = false;
                'sq: for dx in -1..1 {
                    for dy in -10..10 {
                        if pos_y as i32 + dy < 0 || (pos_y as i32 + dy) as u32 >= list_image.height() {
                            continue;
                        }

                        let color = list_image
                            .get_pixel((pos_x as i32 + dx) as u32, (pos_y as i32 + dy) as u32);

                        if color_distance(color, &Rgb([255, 138, 117])) < 30 {
                            locked = true;
                            break 'sq;
                        }
                    }
                }
                result.push(locked);
            }
        }
        result
    }

    pub fn run(self, rx: Receiver<Option<WeaponSendItem>>, total_count: usize) -> JoinHandle<Vec<GenshinWeaponScanResult>> {
        let worker = Arc::new(self);
        std::thread::spawn(move || {
            let mut results = Vec::new();
            let mut hash: HashSet<GenshinWeaponScanResult> = HashSet::new();
            let mut consecutive_dup_count = 0;

            let is_verbose = worker.config.verbose;
            let info = worker.window_info.clone();

            let pb = ProgressBar::new(total_count as u64);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos:>7}/{len:7} ({eta})")
                .unwrap()
                .progress_chars("#>-"));

            let (tx_ocr, rx_ocr) = channel();
            let worker_clone = worker.clone();

            // Launch a thread to receive images and dispatch OCR tasks
            std::thread::spawn(move || {
                let mut locks = Vec::new();
                let mut items_count = 0;
                for item in rx {
                    let item = match item {
                        Some(v) => v,
                        None => break,
                    };

                    if let Some(v) = item.list_image.as_ref() {
                        locks.extend(worker_clone.get_page_locks(v));
                    }

                    let index = items_count;
                    items_count += 1;
                    let lock = if index < locks.len() { locks[index] } else { false };

                    let worker_inner = worker_clone.clone();
                    let tx_ocr_inner = tx_ocr.clone();

                    rayon::spawn(move || {
                        let res = worker_inner.scan_item_image(item, lock, index + 1);
                        let _ = tx_ocr_inner.send((index, res));
                    });
                }
            });

            let mut results_map = BTreeMap::new();
            let mut next_index = 0;

            // Process OCR results in order
            loop {
                while !results_map.contains_key(&next_index) {
                    match rx_ocr.recv() {
                        Ok((i, res)) => {
                            results_map.insert(i, res);
                        },
                        Err(_) => break,
                    }
                }

                let result = match results_map.remove(&next_index) {
                    Some(res) => {
                        next_index += 1;
                        match res {
                            Ok(v) => v,
                            Err(e) => {
                                error!("识别错误: {}", e);
                                pb.inc(1);
                                continue;
                            }
                        }
                    },
                    None => break,
                };

                if is_verbose {
                    info!("{:?}", result);
                }

                if hash.contains(&result) {
                    consecutive_dup_count += 1;
                    warn!("识别到重复武器: {:#?}", result);
                } else {
                    consecutive_dup_count = 0;
                    hash.insert(result.clone());
                    results.push(result);
                }

                pb.inc(1);

                if consecutive_dup_count >= info.col && !worker.config.ignore_dup {
                    error!(
                        "扫描终止：识别到连续 {} 个重复武器，可能为翻页错误，或者为非背包顶部开始扫描，已扫描 {} 个。",
                        consecutive_dup_count, next_index
                    );
                    pb.finish_with_message("重复终止");
                    break;
                }

                if next_index >= total_count {
                    break;
                }
            }

            pb.finish_with_message("完成");
            info!("识别结束，非重复武器数量: {}", hash.len());

            results
        })
    }
}
