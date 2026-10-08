use genshin_scanner::scanner::common::{annotator, coord_scaler::CoordScaler, dump_paths};
use image::{Rgb, RgbImage};

// A separate integration-test process keeps the global dump setting isolated
// from scanner unit tests, which intentionally initialize it with dumping off.
#[test]
fn observation_boundaries_capture_retries_errors_and_standalone_checks_once() {
    let image = RgbImage::from_pixel(11, 7, Rgb([3, 4, 5]));
    annotator::init_for("hsr", true);
    let root = dump_paths::run_dir("debug_images", "hsr");
    annotator::begin_item_for("hsr", "observation_test", 0, &CoordScaler::new(1920, 1080));
    for text in ["", "name"] {
        let result: Result<String, &str> = annotator::observe_ocr("name", &image, || {
            annotator::observe_ocr("nested_model", &image, || Ok(text.into()))
        });
        assert_eq!(result.unwrap(), text);
    }
    let failed: Result<String, &str> =
        annotator::observe_ocr("level", &image, || Err("inference failed"));
    assert!(failed.is_err());
    annotator::observe_detection("lock", &image, || (Some(false), 1.0));
    annotator::finalize_error(None, "panel failed");
    // A domain check before any item context must still be captured.
    annotator::observe_detection("standalone_lock", &image, || false);
    annotator::flush();
    let item = root.join("observation_test/0000");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(item.join("ocr_fields.json")).unwrap())
            .unwrap();
    let fields = manifest["fields"].as_array().unwrap();
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0]["field"], "name");
    assert_eq!(fields[1]["crop"], "name_2.png");
    assert_eq!(fields[2]["inference_error"], true);
    assert_eq!(manifest["detections"][0]["result"], "(Some(false), 1.0)");
    for field in fields {
        assert_eq!(
            image::open(item.join(field["crop"].as_str().unwrap()))
                .unwrap()
                .to_rgb8(),
            image
        );
    }
    let standalone = root.join("observation_standalone_lock/0000");
    assert!(standalone.join("standalone_lock.png").exists());
    assert!(!standalone.join("full.png").exists());
    annotator::init(false);
    let disabled: Result<String, &str> =
        annotator::observe_ocr("disabled", &image, || Ok("text".into()));
    assert_eq!(disabled.unwrap(), "text");
    assert!(!root.join("observation_disabled").exists());
    std::fs::remove_dir_all(root).unwrap();
}
