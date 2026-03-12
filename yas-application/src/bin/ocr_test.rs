//! Quick CLI tool to OCR a given image using ppocrv4 and ppocrv5.
//!
//! Usage: ocr_test <image_path> [--crop-right <pixels>]
//!
//! Runs both ppocrv4 and ppocrv5 on the image and prints results.
//! Optionally crops N pixels from the right side before OCR.

use anyhow::Result;
use image::RgbImage;
use yas::ocr::ImageToText;
use yas_genshin::scanner::common::ocr_factory::create_ocr_model;

fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: ocr_test <image_path> [--crop-right <pixels>]");
        std::process::exit(1);
    }

    let image_path = &args[1];
    let mut crop_right: u32 = 0;
    if args.len() >= 4 && args[2] == "--crop-right" {
        crop_right = args[3].parse().unwrap_or(0);
    }

    // Load image
    let img = image::open(image_path)?.to_rgb8();
    println!("Image: {} ({}x{})", image_path, img.width(), img.height());

    // Optionally crop right side
    let img = if crop_right > 0 && crop_right < img.width() {
        let new_w = img.width() - crop_right;
        println!("Cropping {}px from right -> {}x{}", crop_right, new_w, img.height());
        image::imageops::crop_imm(&img, 0, 0, new_w, img.height()).to_image()
    } else {
        img
    };

    // Load models
    println!("Loading models...");
    let v4 = create_ocr_model("ppocrv4")?;
    let v5 = create_ocr_model("ppocrv5")?;

    // Run OCR
    let result_v4 = v4.image_to_text(&img, false)?;
    let result_v5 = v5.image_to_text(&img, false)?;

    println!();
    println!("ppocrv4: {:?}", result_v4);
    println!("ppocrv5: {:?}", result_v5);

    // Also try stat parsing
    let parsed_v4 = yas_genshin::scanner::common::stat_parser::parse_stat_from_text(result_v4.trim());
    let parsed_v5 = yas_genshin::scanner::common::stat_parser::parse_stat_from_text(result_v5.trim());

    println!();
    if let Some(p) = &parsed_v4 {
        println!("ppocrv4 parsed: key={}, value={}, inactive={}", p.key, p.value, p.inactive);
    } else {
        println!("ppocrv4 parsed: None");
    }
    if let Some(p) = &parsed_v5 {
        println!("ppocrv5 parsed: key={}, value={}, inactive={}", p.key, p.value, p.inactive);
    } else {
        println!("ppocrv5 parsed: None");
    }

    Ok(())
}
