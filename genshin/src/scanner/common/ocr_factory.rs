use std::time::Duration;

use anyhow::{Context, Result};
use image::RgbImage;

use yas::ocr::ImageToText;

const PPOCRV4: &str = "ppocrv4";
const PPOCRV5: &str = "ppocrv5";
const PPOCRV6_TINY: &str = "ppocrv6tiny";

/// Resolve a user-facing backend name (including aliases) to the model it loads.
/// Unknown names fall back to v5, matching `create_ocr_model`.
pub fn canonical_backend(backend: &str) -> &'static str {
    match backend.to_lowercase().as_str() {
        "paddlev4" | "ppocrv4" => PPOCRV4,
        "paddlev6tiny" | "ppocrv6tiny" | "ppocrv6" => PPOCRV6_TINY,
        _ => PPOCRV5,
    }
}

/// True only when both engines are known to run the same weights, so a second
/// inference on the same crop cannot produce a different result.
pub fn same_model(a: &dyn ImageToText<RgbImage>, b: &dyn ImageToText<RgbImage>) -> bool {
    matches!((a.model_id(), b.model_id()), (Some(x), Some(y)) if x == y)
}

struct IdentifiedModel {
    id: &'static str,
    inner: yas::ocr::PPOCRModel,
}

impl ImageToText<RgbImage> for IdentifiedModel {
    fn image_to_text(&self, image: &RgbImage, is_preprocessed: bool) -> Result<String> {
        super::annotator::observe_ocr(&format!("ocr_{}", self.id), image, || {
            self.inner.image_to_text(image, is_preprocessed)
        })
    }

    fn get_average_inference_time(&self) -> Option<Duration> {
        self.inner.get_average_inference_time()
    }

    fn model_id(&self) -> Option<&str> {
        Some(self.id)
    }
}

/// Create an OCR model for the specified backend.
///
/// Supported backends:
/// - `"ppocrv4"` / `"paddlev4"`: PaddleOCR v4 (11M, best for substats)
/// - `"ppocrv6tiny"` / `"ppocrv6"`: PaddleOCR v6 tiny (1.1M, CTC 48×320)
/// - `"ppocrv5"` / `"paddlev5"` / default: PaddleOCR v5 (16M, best for names/text)
///
/// All model weights are embedded at compile time via `include_bytes!`.
pub fn create_ocr_model(backend: &str) -> Result<Box<dyn ImageToText<RgbImage> + Send>> {
    let id = canonical_backend(backend);
    let (model_bytes, dict_str, label): (&[u8], &str, &str) = match id {
        PPOCRV4 => (
            include_bytes!("models/ch_PP-OCRv4_rec_infer.onnx"),
            include_str!("models/ppocr_keys_v1.txt"),
            "v4",
        ),
        PPOCRV6_TINY => (
            include_bytes!("models/PP-OCRv6_tiny_rec.onnx"),
            include_str!("models/ppocrv6_tiny_dict.txt"),
            "v6 tiny",
        ),
        _ => (
            include_bytes!("models/PP-OCRv5_mobile_rec.onnx"),
            include_str!("models/ppocrv5_dict.txt"),
            "v5",
        ),
    };
    let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.trim().to_string()).collect();
    dict_vec.push(String::from(" "));
    let inner = yas::ocr::PPOCRModel::new(model_bytes, dict_vec).with_context(|| {
        format!(
            "ONNX {label}模型初始化失败，请确认onnxruntime.dll存在且版本正确\
             / ONNX {label} model init failed — ensure onnxruntime.dll exists and is the correct version"
        )
    })?;
    Ok(Box::new(IdentifiedModel { id, inner }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_resolve_to_the_model_they_load() {
        assert_eq!(canonical_backend("paddlev4"), canonical_backend("PPOCRv4"));
        assert_eq!(
            canonical_backend("ppocrv6"),
            canonical_backend("ppocrv6tiny")
        );
        assert_eq!(canonical_backend("paddlev5"), canonical_backend("ppocrv5"));
        assert_ne!(canonical_backend("ppocrv4"), canonical_backend("ppocrv5"));
    }
}
