//! Honkai: Star Rail capture, screenshot scanner, export, and attended
//! reversible status manager used by the shared GOODScanner applications.

#[cfg(feature = "capture")]
pub mod achievement_capture;
pub mod capture;
pub mod device;
pub mod error;
pub mod localization;
pub mod manager;
pub mod model;
pub mod observation;
pub mod ocr;
pub mod pipeline;
mod privacy;
pub mod reference;
pub mod scanner;
pub mod vision;

#[cfg(feature = "capture")]
pub use achievement_capture::{
    AchievementCaptureCommand, AchievementCaptureMonitor, AchievementCaptureState,
    AchievementPacketDecoder, DecodedAchievementSnapshot, ACHIEVEMENT_CAPTURE_REVISION,
};
pub use error::{HsrError, HsrResult};
pub use localization::{Language, LocalizedText};
pub use model::*;
pub use observation::{
    parse_sanitized_fixture, FixtureObservationSource, ObservationSource,
    ValidatedObservationSnapshot,
};
pub use pipeline::{
    build_achievement_only_export, build_achievement_snapshot, build_export,
    build_export_with_achievements, write_export_create_new,
};
pub use reference::{
    load_gilore_reference_bundle, GiloreBundleReferenceProvider, JsonFileReferenceProvider,
    ReferenceCache, ReferenceProvider,
};
