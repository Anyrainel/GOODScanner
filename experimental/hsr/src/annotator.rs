//! Thread-local OCR dump helper matching GOODScanner's annotator contract.
//!
//! Scanner and parser code call [`record_ocr`] as a side effect. Those calls
//! are no-ops unless [`init`] enabled dumping, so dump support does not belong
//! in the click/wait control flow.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use image::{Rgb, RgbImage};

use crate::vision::{NormRect, Point};

static ENABLED: AtomicBool = AtomicBool::new(false);
static PENDING_WRITES: Mutex<Vec<std::thread::JoinHandle<()>>> = Mutex::new(Vec::new());

thread_local! {
    static CONTEXT: RefCell<Option<DumpContext>> = const { RefCell::new(None) };
}

struct DumpContext {
    dir: PathBuf,
    images: Vec<(String, RgbImage)>,
    entries: Vec<DumpEntry>,
    current_img: usize,
}

enum DumpEntry {
    Ocr {
        img: usize,
        field: String,
        rect: NormRect,
        raw: String,
        final_result: String,
    },
    Pixel {
        img: usize,
        field: String,
        point: Point,
        rgb: [u8; 3],
        text: String,
    },
}

pub fn init(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn begin_item(category: &str, index: usize) {
    if !is_enabled() {
        return;
    }
    let dir = Path::new("debug_images")
        .join(category)
        .join(format!("{index:04}"));
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = std::fs::create_dir_all(&dir);
    CONTEXT.with(|context| {
        *context.borrow_mut() = Some(DumpContext {
            dir,
            images: Vec::new(),
            entries: Vec::new(),
            current_img: 0,
        });
    });
}

pub fn add_image(label: &str, image: &RgbImage) {
    with_ctx(|context| {
        context.current_img = context.images.len();
        context.images.push((label.to_string(), image.clone()));
    });
}

pub fn record_ocr(field: &str, rect: NormRect, raw_text: &str) {
    with_ctx(|context| {
        context.entries.push(DumpEntry::Ocr {
            img: context.current_img,
            field: field.to_string(),
            rect,
            raw: raw_text.to_string(),
            final_result: String::new(),
        });
    });
}

pub fn set_final(field: &str, result: &str) {
    with_ctx(|context| {
        for entry in context.entries.iter_mut().rev() {
            if let DumpEntry::Ocr {
                field: name,
                final_result,
                ..
            } = entry
            {
                if name == field {
                    *final_result = result.to_string();
                    return;
                }
            }
        }
    });
}

pub fn record_pixel(field: &str, point: Point, rgb: [u8; 3], text: &str) {
    with_ctx(|context| {
        context.entries.push(DumpEntry::Pixel {
            img: context.current_img,
            field: field.to_string(),
            point,
            rgb,
            text: text.to_string(),
        });
    });
}

pub fn finalize_success(result_json: &str) {
    finalize(Some(result_json), None);
}

pub fn finalize_error(partial_json: Option<&str>, error: &str) {
    finalize(partial_json, Some(error));
}

pub fn flush() {
    if let Ok(mut pending) = PENDING_WRITES.lock() {
        for handle in pending.drain(..) {
            let _ = handle.join();
        }
    }
}

fn finalize(result_json: Option<&str>, error: Option<&str>) {
    if !is_enabled() {
        return;
    }
    let json = result_json.map(ToOwned::to_owned);
    let err = error.map(ToOwned::to_owned);
    CONTEXT.with(|context| {
        if let Some(dump) = context.borrow_mut().take() {
            spawn_write(move || dump.write(json.as_deref(), err.as_deref()));
        }
    });
}

fn with_ctx(f: impl FnOnce(&mut DumpContext)) {
    if !is_enabled() {
        return;
    }
    CONTEXT.with(|context| {
        if let Some(ref mut dump) = *context.borrow_mut() {
            f(dump);
        }
    });
}

fn spawn_write(f: impl FnOnce() + Send + 'static) {
    let handle = std::thread::spawn(f);
    if let Ok(mut pending) = PENDING_WRITES.lock() {
        pending.retain(|handle| !handle.is_finished());
        pending.push(handle);
    }
}

impl DumpContext {
    fn write(self, result_json: Option<&str>, error: Option<&str>) {
        let mut result = String::new();
        if let Some(error) = error {
            result.push_str("ERROR: ");
            result.push_str(error);
            result.push('\n');
        }
        if let Some(json) = result_json {
            result.push_str(json);
            result.push('\n');
        }
        result.push('\n');

        for (img_idx, (label, image)) in self.images.iter().enumerate() {
            let full_name = if self.images.len() == 1 {
                "full.png".to_string()
            } else {
                format!("full_{label}.png")
            };
            let _ = image.save(self.dir.join(&full_name));

            let mut annotated = image.clone();
            for entry in &self.entries {
                match entry {
                    DumpEntry::Ocr {
                        img,
                        field,
                        rect,
                        raw,
                        final_result,
                    } if *img == img_idx => {
                        let pixel = rect_pixels(rect, image.width(), image.height());
                        draw_rect(&mut annotated, pixel, Rgb([220, 60, 60]), 2);
                        let crop = crop_pixels(image, pixel);
                        let _ = crop.save(self.dir.join(format!("{field}.png")));
                        result.push_str(&format!(
                            "{field}: {:?} raw={raw:?} final={final_result:?}\n",
                            pixel
                        ));
                    },
                    DumpEntry::Pixel {
                        img,
                        field,
                        point,
                        rgb,
                        text,
                    } if *img == img_idx => {
                        let x = (point.x * image.width() as f64).round() as i32;
                        let y = (point.y * image.height() as f64).round() as i32;
                        draw_cross(&mut annotated, x, y, Rgb([40, 200, 80]));
                        result.push_str(&format!("{field}: ({x},{y}) rgb={rgb:?} {text}\n"));
                    },
                    _ => {},
                }
            }
            let annotated_name = if self.images.len() == 1 {
                "annotated.png".to_string()
            } else {
                format!("annotated_{label}.png")
            };
            let _ = annotated.save(self.dir.join(annotated_name));
        }
        let _ = std::fs::write(self.dir.join("result.txt"), result);
    }
}

fn rect_pixels(rect: &NormRect, width: u32, height: u32) -> (i32, i32, i32, i32) {
    let x = (rect.x * width as f64).floor() as i32;
    let y = (rect.y * height as f64).floor() as i32;
    let w = ((rect.x + rect.width) * width as f64).ceil() as i32 - x;
    let h = ((rect.y + rect.height) * height as f64).ceil() as i32 - y;
    (x, y, w.max(1), h.max(1))
}

fn crop_pixels(image: &RgbImage, rect: (i32, i32, i32, i32)) -> RgbImage {
    let (x, y, w, h) = rect;
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + w).max(0) as u32;
    let y1 = (y + h).max(0) as u32;
    let width = x1.min(image.width()).saturating_sub(x0).max(1);
    let height = y1.min(image.height()).saturating_sub(y0).max(1);
    image::imageops::crop_imm(image, x0, y0, width, height).to_image()
}

fn draw_rect(image: &mut RgbImage, rect: (i32, i32, i32, i32), color: Rgb<u8>, thickness: i32) {
    let (x, y, w, h) = rect;
    let iw = image.width() as i32;
    let ih = image.height() as i32;
    for t in 0..thickness {
        for px in x..x + w {
            for py in [y + t, y + h - 1 - t] {
                if px >= 0 && px < iw && py >= 0 && py < ih {
                    image.put_pixel(px as u32, py as u32, color);
                }
            }
        }
        for py in y..y + h {
            for px in [x + t, x + w - 1 - t] {
                if px >= 0 && px < iw && py >= 0 && py < ih {
                    image.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}

fn draw_cross(image: &mut RgbImage, x: i32, y: i32, color: Rgb<u8>) {
    let iw = image.width() as i32;
    let ih = image.height() as i32;
    for d in -8..=8 {
        for (px, py) in [(x + d, y), (x, y + d)] {
            if px >= 0 && px < iw && py >= 0 && py < ih {
                image.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}
