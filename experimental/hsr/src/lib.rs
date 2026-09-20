//! Honkai: Star Rail capture, screenshot scanner, export, and attended
//! reversible status manager used by the shared GOODScanner applications.

pub mod annotator;
pub mod capture;
pub mod device;
mod embedded_reference;
pub mod error;
pub mod layout;
pub mod localization;
pub mod manager;
pub mod model;
#[cfg(feature = "capture")]
mod network;
pub mod observation;
pub mod ocr;
#[cfg(feature = "capture")]
pub mod packet_capture;
pub mod pipeline;
mod privacy;
pub mod reference;
pub mod scanner;
pub mod scanner_export;
pub mod vision;

pub use embedded_reference::{
    generate_embedded_gilore_reference, load_embedded_gilore_reference, EMBEDDED_GILORE_COMMIT,
    EMBEDDED_GILORE_MANIFEST_SHA256, EMBEDDED_GILORE_SOURCE_REVISION,
    EMBEDDED_REFERENCE_ACHIEVEMENT_COUNT, EMBEDDED_REFERENCE_BYTE_COUNT,
    EMBEDDED_REFERENCE_FORMAT_VERSION, EMBEDDED_REFERENCE_PROVIDER,
    EMBEDDED_REFERENCE_SENTINEL_ACHIEVEMENT_ID, EMBEDDED_REFERENCE_SHA256,
};
pub use error::{HsrError, HsrResult};
pub use localization::{Language, LocalizedText};
pub use model::*;
pub use observation::{
    parse_sanitized_fixture, FixtureObservationSource, ObservationSource,
    ValidatedObservationSnapshot,
};
#[cfg(feature = "capture")]
pub use packet_capture::{
    DecodedAchievementSnapshot, HsrCaptureCommand, HsrCaptureMonitor, HsrCaptureState,
    HsrPacketDecoder, HSR_CAPTURE_REVISION,
};
pub use pipeline::{
    build_achievement_only_export, build_achievement_snapshot, build_export,
    build_export_with_achievements, write_export_create_new, write_json_create_new,
};
pub use reference::{
    load_gilore_reference_bundle, GiloreBundleReferenceProvider, JsonFileReferenceProvider,
    ReferenceCache, ReferenceProvider,
};
pub use scanner_export::{
    build_scanner_export, export_observations, CaptureExportDetails, CharacterDetails,
};

pub mod data_cache;
pub mod packet_reference;
