//! Read-only, fixture-proven Honkai: Star Rail inventory export prototype.
//!
//! This crate is deliberately outside the official GOODScanner workspace and
//! contains no input-control, live-capture, packet-decryption, or mutation code.

pub mod error;
pub mod localization;
pub mod model;
pub mod observation;
pub mod pipeline;
mod privacy;
pub mod reference;

pub use error::{HsrError, HsrResult};
pub use localization::{Language, LocalizedText};
pub use model::*;
pub use observation::{
    parse_sanitized_fixture, FixtureObservationSource, ObservationSource,
    ValidatedObservationSnapshot,
};
pub use pipeline::build_export;
pub use reference::{JsonFileReferenceProvider, ReferenceCache, ReferenceProvider};
