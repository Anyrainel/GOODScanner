use std::{cell::RefCell, ops::{Coroutine, CoroutineState}, pin::Pin, rc::Rc, sync::mpsc::{self, Sender}, time::SystemTime};

use anyhow::Result;
use clap::FromArgMatches;
use image::RgbImage;
use log::{error, info};

use yas::capture::{Capturer, GenericCapturer};
use yas::game_info::GameInfo;
use yas::ocr::{ImageToText, PPOCRModel};
use yas::positioning::Pos;
use yas::window_info::FromWindowInfoRepository;
use yas::window_info::WindowInfoRepository;

use crate::scanner::weapon_scanner::weapon_scanner_worker::WeaponScannerWorker;
use crate::scanner::weapon_scanner::message_items::WeaponSendItem;
use crate::scanner::weapon_scanner::scan_result::GenshinWeaponScanResult;
use crate::scanner_controller::repository_layout::{
    GenshinRepositoryScanController,
    GenshinRepositoryScannerLogicConfig,
    ReturnResult as GenshinRepositoryControllerReturnResult,
};

use super::weapon_scanner_config::GenshinWeaponScannerConfig;
use super::WeaponScannerWindowInfo;

fn color_distance(c1: &image::Rgb<u8>, c2: &image::Rgb<u8>) -> usize {
    let x = c1.0[0] as i32 - c2.0[0] as i32;
    let y = c1.0[1] as i32 - c2.0[1] as i32;
    let z = c1.0[2] as i32 - c2.0[2] as i32;
    (x * x + y * y + z * z) as usize
}

pub struct GenshinWeaponScanner {
    scanner_config: GenshinWeaponScannerConfig,
    window_info: WeaponScannerWindowInfo,
    game_info: GameInfo,
    item_count_image_to_text: Box<dyn ImageToText<RgbImage> + Send>,
    controller: Rc<RefCell<GenshinRepositoryScanController>>,
    capturer: Rc<dyn Capturer<RgbImage>>,
}

impl GenshinWeaponScanner {
    pub const MAX_COUNT: usize = 2400;
}

// constructor
impl GenshinWeaponScanner {
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
        config: GenshinWeaponScannerConfig,
        controller_config: GenshinRepositoryScannerLogicConfig,
        game_info: GameInfo,
    ) -> Result<Self> {
        let item_count_image_to_text = Self::get_image_to_text(&config.ocr_backend)?;

        Ok(Self {
            scanner_config: config,
            window_info: WeaponScannerWindowInfo::from_window_info_repository(
                game_info.window.to_rect_usize().size(),
                game_info.ui,
                game_info.platform,
                window_info_repo,
            )?,
            controller: Rc::new(RefCell::new(
                // is_artifact = false for weapons (no artifact_panel_offset)
                GenshinRepositoryScanController::new(window_info_repo, controller_config, game_info.clone(), false)?
            )),
            game_info,
            item_count_image_to_text,
            capturer: Self::get_capturer()?,
        })
    }

    pub fn from_arg_matches(
        window_info_repo: &WindowInfoRepository,
        arg_matches: &clap::ArgMatches,
        game_info: GameInfo,
    ) -> Result<Self> {
        let window_info = WeaponScannerWindowInfo::from_window_info_repository(
            game_info.window.to_rect_usize().size(),
            game_info.ui,
            game_info.platform,
            window_info_repo,
        )?;
        let config = GenshinWeaponScannerConfig::from_arg_matches(arg_matches)?;
        let item_count_image_to_text = Self::get_image_to_text(&config.ocr_backend)?;

        Ok(GenshinWeaponScanner {
            scanner_config: config,
            window_info,
            controller: Rc::new(RefCell::new(
                // is_artifact = false for weapons (no artifact_panel_offset)
                GenshinRepositoryScanController::from_arg_matches(window_info_repo, arg_matches, game_info.clone(), false)?
            )),
            game_info,
            item_count_image_to_text,
            capturer: Self::get_capturer()?,
        })
    }
}

impl GenshinWeaponScanner {
    pub fn capture_panel(&self) -> Result<RgbImage> {
        self.capturer.capture_relative_to(
            self.window_info.panel_rect.to_rect_i32(),
            self.game_info.window.origin(),
        )
    }

    pub fn get_star(&self) -> Result<usize> {
        let pos: Pos<i32> = Pos {
            x: self.game_info.window.left + self.window_info.star_pos.x as i32,
            y: self.game_info.window.top + self.window_info.star_pos.y as i32,
        };
        let color = self.capturer.capture_color(pos)?;

        let match_colors = [
            image::Rgb([113, 119, 139]),
            image::Rgb([42, 143, 114]),
            image::Rgb([81, 127, 203]),
            image::Rgb([161, 86, 224]),
            image::Rgb([188, 105, 50]),
        ];

        let mut min_dis: usize = 0xdeadbeef;
        let mut ret: usize = 1;
        for (i, match_color) in match_colors.iter().enumerate() {
            let dis2 = color_distance(match_color, &color);
            if dis2 < min_dis {
                min_dis = dis2;
                ret = i + 1;
            }
        }

        anyhow::Ok(ret)
    }

    pub fn get_item_count(&self) -> Result<i32> {
        let count = self.scanner_config.number;

        let max_count = Self::MAX_COUNT as i32;
        if count > 0 {
            return Ok(max_count.min(count));
        }

        let im = self.capturer.capture_relative_to(
            self.window_info.item_count_rect.to_rect_i32(),
            self.game_info.window.origin(),
        )?;

        if self.scanner_config.save_images {
            let debug_dir = std::path::Path::new("./debug_images");
            if !debug_dir.exists() {
                std::fs::create_dir_all(debug_dir)?;
            }
            let image_path = debug_dir.join("weapon_item_count.png");
            im.save(&image_path)?;
            info!("Debug image saved: {}", image_path.display());
        }

        let s = self.item_count_image_to_text.image_to_text(&im, false)?;

        info!("武器信息: {}", s);

        let filtered: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '/').collect();
        let count_part = filtered.split('/').next().unwrap_or("");

        if let Ok(v) = count_part.parse::<usize>() {
            info!("识别到数量: {}", v);
            Ok((v as i32).min(max_count))
        } else {
            info!("无法解析数量，使用默认最大值: {}", max_count);
            Ok(max_count)
        }
    }

    pub fn scan(&mut self) -> Result<Vec<GenshinWeaponScanResult>> {
        info!("开始扫描武器，使用鼠标右键中断扫描");

        let now = SystemTime::now();
        let (tx, rx) = mpsc::channel::<Option<WeaponSendItem>>();
        let count = self.get_item_count()?;
        let worker = WeaponScannerWorker::new(
            self.window_info.clone(),
            self.scanner_config.clone(),
        )?;

        let join_handle = worker.run(rx, count as usize);
        info!("Worker created");

        self.send(&tx, count);

        match tx.send(None) {
            Ok(_) => info!("扫描结束，等待识别线程结束，请勿关闭程序"),
            Err(_) => info!("扫描结束，识别已完成"),
        }

        match join_handle.join() {
            Ok(v) => {
                info!("识别耗时: {:?}", now.elapsed()?);
                Ok(v)
            }
            Err(_) => Err(anyhow::anyhow!("识别线程出现错误")),
        }
    }

    fn is_page_first_weapon(&self, cur_index: i32) -> bool {
        let col = self.window_info.col;
        let row = self.window_info.row;

        let page_size = col * row;
        return cur_index % page_size == 0;
    }

    /// Get the starting row in the page where `cur_index` is in
    fn get_start_row(&self, max_count: i32, cur_index: i32) -> i32 {
        let col = self.window_info.col;
        let row = self.window_info.row;

        let page_size = col * row;
        if max_count - cur_index >= page_size {
            return 0;
        } else {
            let remain = max_count - cur_index;
            let remain_row = (remain + col - 1) / col;
            let scroll_row = remain_row.min(row);
            return row - scroll_row;
        }
    }

    fn send(&mut self, tx: &Sender<Option<WeaponSendItem>>, count: i32) {
        let mut generator = GenshinRepositoryScanController::get_generator(self.controller.clone(), count as usize);
        let mut weapon_index: i32 = 0;

        loop {
            let pinned_generator = Pin::new(&mut generator);
            match pinned_generator.resume(()) {
                CoroutineState::Yielded(_) => {
                    if self.scanner_config.delay > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(self.scanner_config.delay as u64));
                    }

                    let image = self.capture_panel().unwrap();
                    let star = self.get_star().unwrap();

                    // Weapons do not have artifact_panel_offset, so use scan_margin_pos directly
                    let list_image = if self.is_page_first_weapon(weapon_index) {
                        let origin = self.game_info.window;
                        let margin = self.window_info.scan_margin_pos;
                        let gap = self.window_info.item_gap_size;
                        let size = self.window_info.item_size;

                        let left = (origin.left as f64 + margin.x) as i32;
                        let top = (origin.top as f64
                            + margin.y
                            + (gap.height + size.height)
                            * self.get_start_row(count, weapon_index) as f64)
                            as i32;
                        let width = (origin.width as f64 - margin.x) as i32;
                        let height = (origin.height as f64
                            - margin.y
                            - (gap.height + size.height)
                            * self.get_start_row(count, weapon_index) as f64)
                            as i32;

                        let game_image = self
                            .capturer
                            .capture_rect(yas::positioning::Rect {
                                left,
                                top,
                                width,
                                height,
                            })
                            .unwrap();
                        Some(game_image)
                    } else {
                        None
                    };

                    weapon_index = weapon_index + 1;

                    // Check min star requirement
                    if (star as i32) < self.scanner_config.min_star {
                        info!(
                            "找到满足最低星级要求 {} 的武器，准备退出……",
                            self.scanner_config.min_star
                        );
                        break;
                    }

                    if tx
                        .send(Some(WeaponSendItem {
                            panel_image: image,
                            star,
                            list_image,
                        }))
                        .is_err()
                    {
                        break;
                    }
                }
                CoroutineState::Complete(result) => {
                    match result {
                        Err(e) => error!("扫描发生错误：{}", e),
                        Ok(value) => {
                            match value {
                                GenshinRepositoryControllerReturnResult::Interrupted => info!("用户中断"),
                                GenshinRepositoryControllerReturnResult::Finished => ()
                            }
                        }
                    }

                    break;
                }
            }
        }
    }
}
