use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use image::RgbImage;

use crate::scanner::common::ocr_factory;
use yas::ocr::ImageToText;
use yas::{log_debug, log_error};

/// A pool of OCR model instances for true parallel OCR.
///
/// Each `PPOCRModel` uses `Mutex<Session>` internally, so a single instance
/// serializes all OCR calls. By creating N instances (~16MB each), N rayon
/// tasks can run OCR simultaneously.
///
/// Uses a crossbeam bounded channel as the pool: checkout blocks until a
/// model is available, and the `OcrGuard` returns it on drop.
pub struct OcrPool {
    checkout: crossbeam_channel::Receiver<Box<dyn ImageToText<RgbImage> + Send>>,
    checkin: crossbeam_channel::Sender<Box<dyn ImageToText<RgbImage> + Send>>,
}

impl OcrPool {
    /// Create a pool with `count` model instances.
    ///
    /// `create_fn` is called `count` times to create independent model instances.
    pub fn new<F>(create_fn: F, count: usize) -> Result<Self>
    where
        F: Fn() -> Result<Box<dyn ImageToText<RgbImage> + Send>>,
    {
        let (checkin, checkout) = crossbeam_channel::bounded(count);
        for _ in 0..count {
            checkin
                .send(create_fn()?)
                .map_err(|_| anyhow::anyhow!("OCR池通道已关闭 / Pool channel closed"))?;
        }
        Ok(Self { checkout, checkin })
    }

    /// Checkout a model from the pool. Blocks until one is available.
    /// The model is returned to the pool when the guard is dropped.
    pub fn get(&self) -> OcrGuard {
        let model = self
            .checkout
            .recv()
            .expect("OCR池通道已关闭 / OCR pool channel closed");
        OcrGuard {
            model: Some(model),
            checkin: self.checkin.clone(),
        }
    }
}

/// RAII guard that returns the OCR model to the pool on drop.
pub struct OcrGuard {
    model: Option<Box<dyn ImageToText<RgbImage> + Send>>,
    checkin: crossbeam_channel::Sender<Box<dyn ImageToText<RgbImage> + Send>>,
}

impl ImageToText<RgbImage> for OcrGuard {
    fn image_to_text(&self, image: &RgbImage, is_preprocessed: bool) -> Result<String> {
        self.model
            .as_ref()
            .expect("OCR模型已被取走 / OcrGuard model already taken")
            .image_to_text(image, is_preprocessed)
    }

    fn get_average_inference_time(&self) -> Option<Duration> {
        self.model
            .as_ref()
            .and_then(|m| m.get_average_inference_time())
    }
}

// Safety: OcrGuard holds a Box<dyn ImageToText<RgbImage> + Send> which is Send.
// The crossbeam Sender is Send + Sync. OcrGuard is only used from the thread
// that checked it out, but we need Sync for the ImageToText trait bound.
unsafe impl Sync for OcrGuard {}

impl Drop for OcrGuard {
    fn drop(&mut self) {
        if let Some(model) = self.model.take() {
            let _ = self.checkin.send(model);
        }
    }
}

/// OCR pool sizing based on available system memory.
///
/// Two tiers:
/// - Normal (≥8 GB available): 2 v5 + 4 v4
/// - Small  (<8 GB or unknown): 1 v5 + 1 v4
#[derive(Clone, Debug)]
pub struct OcrPoolConfig {
    pub v5_count: usize,
    pub v4_count: usize,
}

impl OcrPoolConfig {
    /// Detect available memory and choose pool sizes.
    pub fn detect() -> Self {
        const EIGHT_GB: u64 = 8 * 1024 * 1024 * 1024;
        const FOUR_GB: u64 = 4 * 1024 * 1024 * 1024;

        let available = yas::utils::available_memory_bytes();
        let (v5_count, v4_count) = match available {
            Some(bytes) if bytes >= EIGHT_GB => {
                let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                log_debug!(
                    "可用内存 {:.1} GB ≥ 8 GB，使用大型OCR池 (2×v5 + 4×v4)",
                    "Available memory {:.1} GB ≥ 8 GB, using large OCR pool (2×v5 + 4×v4)",
                    gb,
                );
                (2, 4)
            },
            Some(bytes) if bytes >= FOUR_GB => {
                let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                log_debug!(
                    "可用内存 {:.1} GB ≥ 4 GB，使用中型OCR池 (2×v5 + 2×v4)",
                    "Available memory {:.1} GB ≥ 4 GB, using medium OCR pool (2×v5 + 2×v4)",
                    gb,
                );
                (2, 2)
            },
            Some(bytes) => {
                let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                log_debug!(
                    "可用内存 {:.1} GB < 4 GB，使用小型OCR池 (1×v5 + 1×v4)",
                    "Available memory {:.1} GB < 4 GB, using small OCR pool (1×v5 + 1×v4)",
                    gb,
                );
                (1, 1)
            },
            None => {
                log_debug!(
                    "无法检测内存，使用小型OCR池 (1×v5 + 1×v4)",
                    "Cannot detect memory, using small OCR pool (1×v5 + 1×v4)",
                );
                (1, 1)
            },
        };

        Self { v5_count, v4_count }
    }
}

/// Character screens: v4 reads names/levels best, v5 cross-checks names and
/// levels, and v6 tiny cannot read 魈.
pub const DEFAULT_CHARACTER_OCR: &str = "ppocrv4";
pub const DEFAULT_CHARACTER_SECONDARY_OCR: &str = "ppocrv5";
/// Weapons and artifacts: v6 tiny beat v4/v5 on every inventory field and tied
/// on levels, so it serves both slots.
pub const DEFAULT_WEAPON_OCR: &str = "ppocrv6tiny";
pub const DEFAULT_WEAPON_SECONDARY_OCR: &str = "ppocrv6tiny";
pub const DEFAULT_ARTIFACT_OCR: &str = "ppocrv6tiny";
pub const DEFAULT_ARTIFACT_LEVEL_OCR: &str = "ppocrv6tiny";

/// Backends for one scan category's two pool slots.
///
/// The slots keep their historical names: `v4` is the general text engine,
/// `v5` the secondary one (character name/level cross-check, weapon equip
/// fallback, artifact level).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrSlotBackends {
    pub v5: String,
    pub v4: String,
}

/// Per-category OCR backends for a scan session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrBackends {
    pub character: OcrSlotBackends,
    pub weapon: OcrSlotBackends,
    pub artifact: OcrSlotBackends,
}

impl OcrBackends {
    /// `secondary_override` replaces every category's secondary (v5-slot)
    /// backend; each `*_ocr` is that category's general (v4-slot) backend.
    pub fn resolve(
        secondary_override: Option<&str>,
        character_ocr: &str,
        weapon_ocr: &str,
        artifact_ocr: &str,
    ) -> Self {
        let slots = |general: &str, secondary: &str| OcrSlotBackends {
            v5: secondary_override.unwrap_or(secondary).to_string(),
            v4: general.to_string(),
        };
        Self {
            character: slots(character_ocr, DEFAULT_CHARACTER_SECONDARY_OCR),
            weapon: slots(weapon_ocr, DEFAULT_WEAPON_SECONDARY_OCR),
            artifact: slots(artifact_ocr, DEFAULT_ARTIFACT_LEVEL_OCR),
        }
    }
}

impl Default for OcrBackends {
    fn default() -> Self {
        Self::resolve(
            None,
            DEFAULT_CHARACTER_OCR,
            DEFAULT_WEAPON_OCR,
            DEFAULT_ARTIFACT_OCR,
        )
    }
}

/// The two pools one scan category draws from.
///
/// Workers hold a guard from each slot at the same time, so the slots are
/// always distinct pools even when both name the same backend (sharing one
/// pool would deadlock once every instance is checked out).
#[derive(Clone)]
pub struct OcrPoolPair {
    v5_pool: Arc<OcrPool>,
    v4_pool: Arc<OcrPool>,
}

impl OcrPoolPair {
    /// Secondary engine; also used for one-off OCR such as the backpack item count.
    pub fn v5(&self) -> &Arc<OcrPool> {
        &self.v5_pool
    }

    /// General text engine.
    pub fn v4(&self) -> &Arc<OcrPool> {
        &self.v4_pool
    }
}

/// Shared OCR model pools for the entire scan session.
///
/// Created once, passed by reference to all scanners and managers.
/// Eliminates per-scanner pool creation/destruction overhead and
/// prevents OOM on low-memory systems. Categories whose slots name the same
/// backend share the same pool instances.
pub struct SharedOcrPools {
    character: OcrPoolPair,
    weapon: OcrPoolPair,
    artifact: OcrPoolPair,
    config: OcrPoolConfig,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Slot {
    V5,
    V4,
}

impl SharedOcrPools {
    /// Create shared pools with the given config and per-category backends.
    ///
    /// The first `create_ocr_model` call triggers `ort`'s lazy DLL load.
    /// If the DLL or its dependencies (VC++ runtime) are missing, `ort`
    /// **panics** rather than returning an error.  We catch this with
    /// `catch_unwind` and convert it to a diagnosed error.
    pub fn new(config: OcrPoolConfig, backends: &OcrBackends) -> Result<Self> {
        let mut created: HashMap<(Slot, String), Arc<OcrPool>> = HashMap::new();
        let mut pool = |slot: Slot, backend: &str| -> Result<Arc<OcrPool>> {
            if let Some(existing) = created.get(&(slot, backend.to_string())) {
                return Ok(existing.clone());
            }
            let (count, label) = match slot {
                Slot::V5 => (config.v5_count, "v5"),
                Slot::V4 => (config.v4_count, "v4"),
            };
            let be = backend.to_string();
            let new_pool = Arc::new(create_pool_caught(
                move || ocr_factory::create_ocr_model(&be),
                count,
                label,
            )?);
            created.insert((slot, backend.to_string()), new_pool.clone());
            Ok(new_pool)
        };
        let mut pair = |slots: &OcrSlotBackends| -> Result<OcrPoolPair> {
            Ok(OcrPoolPair {
                v5_pool: pool(Slot::V5, &slots.v5)?,
                v4_pool: pool(Slot::V4, &slots.v4)?,
            })
        };
        let character = pair(&backends.character)?;
        let weapon = pair(&backends.weapon)?;
        let artifact = pair(&backends.artifact)?;

        log_debug!(
            "OCR池已创建: {:?} (v5槽×{}, v4槽×{})",
            "OCR pools created: {:?} (v5 slot×{}, v4 slot×{})",
            backends,
            config.v5_count,
            config.v4_count,
        );

        Ok(Self {
            character,
            weapon,
            artifact,
            config,
        })
    }

    pub fn character(&self) -> &OcrPoolPair {
        &self.character
    }

    pub fn weapon(&self) -> &OcrPoolPair {
        &self.weapon
    }

    pub fn artifact(&self) -> &OcrPoolPair {
        &self.artifact
    }

    pub fn config(&self) -> &OcrPoolConfig {
        &self.config
    }
}

/// Create an OcrPool, catching both `Result::Err` and panics from `ort`.
///
/// `ort`'s `load-dynamic` feature panics (not errors) when the DLL fails
/// to load.  The panic message is:
///   "An error occurred while attempting to load the ONNX Runtime binary
///    at `{path}`: LoadLibraryExW failed"
/// This happens on the first `Session::builder()` call, which triggers
/// the lazy `libloading::Library::new()`.
fn create_pool_caught<F>(create_fn: F, count: usize, label: &str) -> Result<OcrPool>
where
    F: Fn() -> Result<Box<dyn ImageToText<RgbImage> + Send>> + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        OcrPool::new(&create_fn, count)
    })) {
        Ok(result) => result.map_err(|e| diagnose_error(format!("{:#}", e))),
        Err(panic_payload) => {
            let panic_msg = match panic_payload.downcast_ref::<String>() {
                Some(s) => s.clone(),
                None => match panic_payload.downcast_ref::<&str>() {
                    Some(s) => s.to_string(),
                    None => "unknown panic".to_string(),
                },
            };
            Err(diagnose_error(format!("[{}] panic: {}", label, panic_msg)))
        },
    }
}

/// Log an OCR initialization failure and append actionable hints based on
/// the error message content.
///
/// Known failure modes (from ort 2.0.0-rc.10 with load-dynamic):
///
/// | Source | Pattern | Cause |
/// |--------|---------|-------|
/// | panic  | "LoadLibraryExW failed" | DLL or dependency missing (VC++ runtime) |
/// | panic  | "is not compatible with the ONNX Runtime binary" | DLL version mismatch |
/// | panic  | "OrtGetApiBase" | Not a valid ONNX Runtime DLL |
/// | error  | "protobuf parsing failed" / "could not parse model" | Corrupt ONNX model |
/// | error  | "bad_alloc" | Out of memory |
/// | error  | "not supported in this build" | ORT format version mismatch |
fn diagnose_error(msg: String) -> anyhow::Error {
    // Always log the raw error — essential for remote debugging
    log_error!("OCR模型加载失败: {}", "OCR model failed to load: {}", msg);

    let lower = msg.to_lowercase();
    const VCPP_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

    // DLL loading failure (panic from ort's load-dynamic).
    // By this point ensure_onnxruntime() confirmed onnxruntime.dll exists,
    // so "LoadLibraryExW failed" means a *dependency* is missing — almost
    // always the VC++ runtime.
    if lower.contains("loadlibraryexw failed") || lower.contains("loadlibrary") {
        log_error!(
            "onnxruntime.dll 加载失败，最常见原因：缺少 Visual C++ 运行库",
            "onnxruntime.dll failed to load. Most common cause: missing Visual C++ runtime"
        );
        log_error!(
            "请安装 VC++ 2015-2022 Redistributable (x64): {}",
            "Install VC++ 2015-2022 Redistributable (x64): {}",
            VCPP_URL
        );
        log_error!(
            "若已安装，请删除 onnxruntime.dll 后重启程序以重新下载",
            "If already installed, delete onnxruntime.dll and restart to re-download"
        );
        return anyhow::anyhow!("{}", msg);
    }

    // DLL version mismatch
    if lower.contains("is not compatible with the onnx runtime binary")
        || lower.contains("not supported in this build")
    {
        log_error!(
            "onnxruntime.dll 版本不兼容，请删除后重启程序以重新下载正确版本",
            "onnxruntime.dll version is incompatible. Delete it and restart to re-download the correct version"
        );
        return anyhow::anyhow!("{}", msg);
    }

    // Corrupt or invalid DLL
    if lower.contains("ortgetapibase") {
        log_error!(
            "onnxruntime.dll 文件损坏或无效，请删除后重启程序以重新下载",
            "onnxruntime.dll is corrupt or invalid. Delete it and restart to re-download"
        );
        return anyhow::anyhow!("{}", msg);
    }

    // Corrupt ONNX model (embedded at compile time — should never happen,
    // but could indicate a bad build)
    if lower.contains("protobuf parsing failed")
        || lower.contains("could not parse model")
        || lower.contains("model verification failed")
    {
        log_error!(
            "OCR模型文件损坏，请重新下载本程序",
            "OCR model data is corrupt. Please re-download this program"
        );
        return anyhow::anyhow!("{}", msg);
    }

    // Out of memory
    if lower.contains("bad_alloc") || lower.contains("out of memory") {
        log_error!(
            "内存不足，无法加载OCR模型。请关闭其他程序后重试",
            "Out of memory loading OCR model. Close other programs and try again"
        );
        return anyhow::anyhow!("{}", msg);
    }

    // Unknown error — raw message already logged above
    anyhow::anyhow!("{}", msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(v5: &str, v4: &str) -> OcrSlotBackends {
        OcrSlotBackends {
            v5: v5.to_string(),
            v4: v4.to_string(),
        }
    }

    #[test]
    fn defaults_keep_characters_off_v6_tiny() {
        let backends = OcrBackends::default();

        assert_eq!(backends.character, slots("ppocrv5", "ppocrv4"));
        assert_eq!(backends.weapon, slots("ppocrv6tiny", "ppocrv6tiny"));
        assert_eq!(backends.artifact, slots("ppocrv6tiny", "ppocrv6tiny"));
    }

    #[test]
    fn secondary_override_applies_to_every_category() {
        let backends = OcrBackends::resolve(Some("ppocrv5"), "ppocrv4", "ppocrv4", "ppocrv4");

        assert_eq!(backends.character, slots("ppocrv5", "ppocrv4"));
        assert_eq!(backends.weapon, slots("ppocrv5", "ppocrv4"));
        assert_eq!(backends.artifact, slots("ppocrv5", "ppocrv4"));
    }
}
