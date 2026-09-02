//! Isolated experimental Honkai: Star Rail screenshot scanner and attended
//! reversible status manager. This crate is excluded from the official Cargo
//! workspace and is never linked into GOODScanner or GOODCapture.

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

pub use error::{HsrError, HsrResult};
pub use localization::{Language, LocalizedText};
pub use model::*;
pub use observation::{
    parse_sanitized_fixture, FixtureObservationSource, ObservationSource,
    ValidatedObservationSnapshot,
};
pub use pipeline::build_export;
pub use reference::{
    GiloreBundleReferenceProvider, JsonFileReferenceProvider, ReferenceCache, ReferenceProvider,
};
