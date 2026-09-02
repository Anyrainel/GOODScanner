//! Read-only packet-capture adapter for the isolated HSR experiment.
//!
//! GOODScanner does not parse game packets here. Instead, this module invokes
//! one user-supplied, checksum-pinned Reliquary Archiver executable, imports
//! its Fribbels-v4 JSON through a short-lived private directory. Cleanup is
//! attempted explicitly before every return and any failure is surfaced; Drop
//! remains a best-effort fallback. No packet-write or live import server exists
//! in this boundary.

use std::{
    cmp::Ordering,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    error::{hints, HsrError, HsrResult},
    model::{
        CoverageLevel, EvidenceKind, GearSlot, InventoryCoverage, ObservationEvidence,
        ObservationSnapshot, ObservedCharacter, ObservedGear, ObservedLightCone, ObservedSubstat,
        OBSERVATION_SCHEMA_VERSION,
    },
    observation::ValidatedObservationSnapshot,
    privacy::reject_sensitive_fields,
    reference::ReferenceCache,
};

/// Audited helper boundary. The release workflow built v0.18.0 from the source
/// represented by this post-release version commit and pins Reliquary v23.0.0
/// (`d5cf3b7e7e66470d2d8efff6676aa18762b21d3b`) for HSR 4.5.
pub const RELIQUARY_ARCHIVER_RELEASE: &str = "0.18.0";
pub const RELIQUARY_ARCHIVER_REVISION: &str = "cb109f17a4a15b7604cfe9d078a8735e7735cd25";
pub const RELIQUARY_ARCHIVE_SOURCE: &str = "reliquary_archiver";
pub const RELIQUARY_ARCHIVE_FORMAT_VERSION: u32 = 4;

const GILORE_PROVIDER: &str = "gilore.ggstarrail-reference";
const DEFAULT_CAPTURE_TIMEOUT_SECONDS: u64 = 120;
const MAX_CAPTURE_TIMEOUT_SECONDS: u64 = 600;
const PROCESS_EXIT_GRACE_SECONDS: u64 = 30;
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_HELPER_BYTES: u64 = 256 * 1024 * 1024;
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x0800_0000;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A successfully normalized archive plus its explicitly scoped coverage.
/// The contained observation has already passed the shared semantic and
/// privacy gates and cannot expose the helper's UID or item-instance IDs.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureImport {
    observations: ValidatedObservationSnapshot,
    coverage: InventoryCoverage,
}

impl CaptureImport {
    pub fn coverage(&self) -> InventoryCoverage {
        self.coverage
    }

    pub fn into_observations(self) -> ValidatedObservationSnapshot {
        self.observations
    }
}

/// User-selected helper pinned by a caller-provided SHA-256. This crate never
/// downloads, updates, replaces, or trusts an executable by filename alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedReliquaryArchiver {
    executable: PathBuf,
    expected_sha256: [u8; 32],
    timeout_seconds: u64,
}

impl PinnedReliquaryArchiver {
    pub fn new(
        executable: impl Into<PathBuf>,
        expected_sha256: impl AsRef<str>,
    ) -> HsrResult<Self> {
        Ok(Self {
            executable: executable.into(),
            expected_sha256: parse_sha256(expected_sha256.as_ref())?,
            timeout_seconds: DEFAULT_CAPTURE_TIMEOUT_SECONDS,
        })
    }

    pub fn with_timeout_seconds(mut self, timeout_seconds: u64) -> HsrResult<Self> {
        if !(1..=MAX_CAPTURE_TIMEOUT_SECONDS).contains(&timeout_seconds) {
            return Err(capture_error(
                "HSR-CAPTURE-CONFIG",
                format!("timeout must be between 1 and {MAX_CAPTURE_TIMEOUT_SECONDS} seconds"),
            ));
        }
        self.timeout_seconds = timeout_seconds;
        Ok(self)
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Run the pinned helper and require the complete initial archive promised
    /// by the audited Reliquary boundary. Partial offline files can still be
    /// imported with [`import_reliquary_archive_file`], but a live run never
    /// claims success unless all three upstream collections are present.
    pub fn capture(&self, references: &ReferenceCache) -> HsrResult<CaptureImport> {
        self.capture_with_runner(&SystemArchiverRunner, references)
    }

    pub fn capture_with_runner(
        &self,
        runner: &dyn ArchiverProcessRunner,
        references: &ReferenceCache,
    ) -> HsrResult<CaptureImport> {
        self.capture_with_runner_and_cleanup(runner, references, |path| fs::remove_dir_all(path))
    }

    fn capture_with_runner_and_cleanup<F>(
        &self,
        runner: &dyn ArchiverProcessRunner,
        references: &ReferenceCache,
        cleanup: F,
    ) -> HsrResult<CaptureImport>
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        validate_reference_boundary(references)?;
        references.validate_live_complete_profile()?;

        let mut output = PrivateCaptureOutput::create().map_err(|error| {
            capture_error(
                "HSR-CAPTURE-TEMP",
                format!("could not create private capture output; cause={error}"),
            )
        })?;
        let result = (|| {
            let verified_executable = stage_verified_executable(
                &self.executable,
                &self.expected_sha256,
                output.directory(),
            )?;
            let invocation = ArchiverInvocation {
                executable: verified_executable,
                arguments: vec![
                    OsString::from("--no-update"),
                    OsString::from("--exit-after-capture"),
                    OsString::from("--headless"),
                    OsString::from("--timeout"),
                    OsString::from(self.timeout_seconds.to_string()),
                    output.path().as_os_str().to_owned(),
                ],
                working_directory: output.directory().to_path_buf(),
                output_path: output.path().to_path_buf(),
                maximum_runtime: Duration::from_secs(
                    self.timeout_seconds
                        .saturating_add(PROCESS_EXIT_GRACE_SECONDS),
                ),
            };

            let process = runner.run(&invocation).map_err(|error| {
                capture_error(
                    "HSR-CAPTURE-PROCESS",
                    format!("pinned helper could not complete; cause={error}"),
                )
            })?;
            if !process.success {
                return Err(capture_error(
                    "HSR-CAPTURE-EXIT",
                    format!(
                        "pinned helper exited unsuccessfully; exitCode={}",
                        process
                            .exit_code
                            .map_or_else(|| "unavailable".to_string(), |code| code.to_string())
                    ),
                ));
            }

            let imported = import_reliquary_archive_file(output.path(), references)?;
            let complete = InventoryCoverage {
                characters: CoverageLevel::Complete,
                light_cones: CoverageLevel::Complete,
                relics: CoverageLevel::Complete,
            };
            if imported.coverage != complete {
                return Err(archive_error(
                    "HSR-CAPTURE-INCOMPLETE",
                    "helper output did not prove non-empty characters, lightCones, and relics collections",
                ));
            }
            Ok(imported)
        })();
        output.finish(result, cleanup)
    }

    /// Test-only cleanup fault injection. Even a remover that reports success
    /// cannot bypass the postcondition that the private directory is absent.
    #[cfg(feature = "test-as-invoker")]
    pub fn capture_with_runner_and_cleanup_for_test<F>(
        &self,
        runner: &dyn ArchiverProcessRunner,
        references: &ReferenceCache,
        cleanup: F,
    ) -> HsrResult<CaptureImport>
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        self.capture_with_runner_and_cleanup(runner, references, cleanup)
    }
}

/// Exact process description handed to a runner. Tests can inspect it without
/// launching a real helper, while production uses [`SystemArchiverRunner`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiverInvocation {
    executable: PathBuf,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    output_path: PathBuf,
    maximum_runtime: Duration,
}

impl ArchiverInvocation {
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    pub fn maximum_runtime(&self) -> Duration {
        self.maximum_runtime
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiverProcessResult {
    success: bool,
    exit_code: Option<i32>,
}

impl ArchiverProcessResult {
    pub const fn success() -> Self {
        Self {
            success: true,
            exit_code: Some(0),
        }
    }

    pub const fn failure(exit_code: Option<i32>) -> Self {
        Self {
            success: false,
            exit_code,
        }
    }
}

pub trait ArchiverProcessRunner: Send + Sync {
    fn run(&self, invocation: &ArchiverInvocation) -> io::Result<ArchiverProcessResult>;
}

/// Production runner. Helper stdout and stderr are sent directly to the null
/// device and are never captured, logged, or included in user-facing errors.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemArchiverRunner;

impl ArchiverProcessRunner for SystemArchiverRunner {
    fn run(&self, invocation: &ArchiverInvocation) -> io::Result<ArchiverProcessResult> {
        let mut command = Command::new(invocation.executable());
        command
            .args(invocation.arguments())
            .current_dir(invocation.working_directory())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_remove("RUST_LOG");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
        }

        let mut child = command.spawn()?;
        let deadline = Instant::now() + invocation.maximum_runtime();
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(ArchiverProcessResult {
                    success: status.success(),
                    exit_code: status.code(),
                });
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "pinned helper exceeded the bounded capture time",
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Import a previously created archive without invoking or controlling any
/// process. This is the testable/offline boundary; it performs the same source,
/// version, redaction, reference, and semantic checks as live helper output.
pub fn import_reliquary_archive_file(
    path: impl AsRef<Path>,
    references: &ReferenceCache,
) -> HsrResult<CaptureImport> {
    validate_reference_boundary(references)?;
    let bytes = read_bounded_regular_file(path.as_ref())?;
    parse_reliquary_archive(&bytes, references)
}

pub fn parse_reliquary_archive(
    bytes: &[u8],
    references: &ReferenceCache,
) -> HsrResult<CaptureImport> {
    validate_reference_boundary(references)?;
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(archive_error(
            "HSR-CAPTURE-SIZE",
            "archive exceeds the 64 MiB import limit",
        ));
    }

    let mut value: Value = serde_json::from_slice(bytes).map_err(|error| {
        HsrError::new(
            "HSR-CAPTURE-JSON",
            hints::JSON_INVALID,
            format!(
                "archive JSON failed at line {}, column {}",
                error.line(),
                error.column()
            ),
        )
    })?;
    reject_embedded_packet_data(&value, "$")?;
    discard_private_archive_fields(&mut value, true);
    reject_sensitive_fields(&value).map_err(|error| {
        HsrError::new(
            error.code(),
            hints::SENSITIVE_DATA,
            "archive retained a prohibited field after identifier redaction",
        )
    })?;

    let archive: ReliquaryArchive = serde_json::from_value(value).map_err(|_| {
        archive_error(
            "HSR-CAPTURE-SHAPE",
            "archive does not match the audited Fribbels v4 structure",
        )
    })?;
    validate_archive_header(&archive)?;
    normalize_archive(archive, references)
}

#[derive(Debug, Deserialize)]
struct ReliquaryArchive {
    source: String,
    build: String,
    version: u32,
    #[serde(default)]
    characters: Option<Vec<ArchiveCharacter>>,
    #[serde(default)]
    light_cones: Option<Vec<ArchiveLightCone>>,
    #[serde(default)]
    relics: Option<Vec<ArchiveRelic>>,
}

#[derive(Debug, Deserialize)]
struct ArchiveCharacter {
    id: PublicId,
    level: u32,
    ascension: u32,
    eidolon: u32,
}

#[derive(Debug, Deserialize)]
struct ArchiveLightCone {
    id: PublicId,
    level: u32,
    ascension: u32,
    superimposition: u32,
    #[serde(default)]
    location: String,
    #[serde(default)]
    lock: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ArchiveRelic {
    set_id: PublicId,
    slot: String,
    rarity: u32,
    level: u32,
    mainstat: String,
    #[serde(default)]
    substats: Vec<ArchiveSubstat>,
    #[serde(default)]
    location: String,
    #[serde(default)]
    lock: Option<bool>,
    #[serde(default)]
    discard: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ArchiveSubstat {
    key: String,
    value: f64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PublicId {
    String(String),
    Number(u64),
}

impl PublicId {
    fn into_u32(self, context: &str) -> HsrResult<u32> {
        let parsed = match self {
            Self::String(value) => value.parse::<u32>().ok(),
            Self::Number(value) => u32::try_from(value).ok(),
        };
        parsed.filter(|value| *value != 0).ok_or_else(|| {
            archive_error(
                "HSR-CAPTURE-ID",
                format!("{context} contains an invalid public ID; value redacted"),
            )
        })
    }
}

fn normalize_archive(
    archive: ReliquaryArchive,
    references: &ReferenceCache,
) -> HsrResult<CaptureImport> {
    let coverage = InventoryCoverage {
        characters: collection_coverage(&archive.characters),
        light_cones: collection_coverage(&archive.light_cones),
        relics: collection_coverage(&archive.relics),
    };

    let mut characters = archive
        .characters
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, entry)| normalize_character(entry, references, index))
        .collect::<HsrResult<Vec<_>>>()?;
    characters.sort_by_key(|entry| entry.character_id);
    if characters
        .windows(2)
        .any(|pair| pair[0].character_id == pair[1].character_id)
    {
        return Err(archive_error(
            "HSR-CAPTURE-DUPLICATE",
            "archive contains a duplicate public character ID",
        ));
    }

    let mut light_cones = archive
        .light_cones
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, entry)| normalize_light_cone(entry, references, index))
        .collect::<HsrResult<Vec<_>>>()?;
    light_cones.sort_by(compare_light_cones);

    let mut gear = archive
        .relics
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, entry)| normalize_relic(entry, references, index))
        .collect::<HsrResult<Vec<_>>>()?;
    gear.sort_by(compare_gear);

    let snapshot = ObservationSnapshot {
        schema_version: OBSERVATION_SCHEMA_VERSION,
        evidence: ObservationEvidence {
            kind: EvidenceKind::PacketCapture,
            revision: format!(
                "reliquary-archiver-{}-{}",
                RELIQUARY_ARCHIVER_RELEASE,
                &RELIQUARY_ARCHIVER_REVISION[..8]
            ),
            coverage,
        },
        characters,
        light_cones,
        gear,
    };
    let observations = ValidatedObservationSnapshot::from_packet_capture(snapshot)?;
    Ok(CaptureImport {
        observations,
        coverage,
    })
}

fn normalize_character(
    entry: ArchiveCharacter,
    references: &ReferenceCache,
    index: usize,
) -> HsrResult<ObservedCharacter> {
    let id = entry.id.into_u32(&format!("characters[{index}]"))?;
    require_reference(
        references.character(id).is_some(),
        "character",
        &format!("characters[{index}]"),
    )?;
    let level = bounded_u8(entry.level, 1, 80, &format!("characters[{index}].level"))?;
    let ascension = bounded_u8(
        entry.ascension,
        0,
        6,
        &format!("characters[{index}].ascension"),
    )?;
    let eidolon = bounded_u8(entry.eidolon, 0, 6, &format!("characters[{index}].eidolon"))?;
    Ok(ObservedCharacter {
        character_id: id,
        level,
        ascension,
        eidolon,
    })
}

fn normalize_light_cone(
    entry: ArchiveLightCone,
    references: &ReferenceCache,
    index: usize,
) -> HsrResult<ObservedLightCone> {
    let context = format!("lightCones[{index}]");
    let id = entry.id.into_u32(&context)?;
    require_reference(references.light_cone(id).is_some(), "lightCone", &context)?;
    Ok(ObservedLightCone {
        light_cone_id: id,
        level: bounded_u8(entry.level, 1, 80, &format!("{context}.level"))?,
        ascension: bounded_u8(entry.ascension, 0, 6, &format!("{context}.ascension"))?,
        superimposition: bounded_u8(
            entry.superimposition,
            1,
            5,
            &format!("{context}.superimposition"),
        )?,
        equipped_character_id: parse_location(&entry.location, references, &context)?,
        lock: entry.lock,
    })
}

fn normalize_relic(
    entry: ArchiveRelic,
    references: &ReferenceCache,
    index: usize,
) -> HsrResult<ObservedGear> {
    let context = format!("relics[{index}]");
    let set_id = entry.set_id.into_u32(&context)?;
    let set_key = set_id.to_string();
    let slot = parse_slot(&entry.slot, &context)?;
    let rarity = bounded_u8(entry.rarity, 2, 5, &format!("{context}.rarity"))?;
    let level = bounded_u8(
        entry.level,
        0,
        u32::from(rarity) * 3,
        &format!("{context}.level"),
    )?;
    let main_stat_key = main_stat_key(&entry.mainstat, slot).ok_or_else(|| {
        archive_error(
            "HSR-CAPTURE-STAT",
            format!("{context} has an unsupported main-stat label"),
        )
    })?;
    require_reference(
        references.stat(main_stat_key).is_some(),
        "mainStat",
        &context,
    )?;
    let gear_reference = references
        .resolve_gear_by_set_slot_rarity_main_stat(&set_key, slot, rarity, main_stat_key, level)
        .ok_or_else(|| missing_reference("gearMainStatIdentity", &context))?;
    let main_stat_value = references
        .relic_main_stat_value_for_piece(gear_reference, main_stat_key, level)
        .ok_or_else(|| missing_reference("mainStatValue", &context))?;

    if entry.substats.len() > 4 {
        return Err(archive_error(
            "HSR-CAPTURE-STAT",
            format!("{context} contains more than four substats"),
        ));
    }
    let mut substats = entry
        .substats
        .into_iter()
        .enumerate()
        .map(|(substat_index, substat)| {
            let substat_context = format!("{context}.substats[{substat_index}]");
            let stat_key = substat_key(&substat.key).ok_or_else(|| {
                archive_error(
                    "HSR-CAPTURE-STAT",
                    format!("{substat_context} has an unsupported stat label"),
                )
            })?;
            require_reference(
                references.stat(stat_key).is_some(),
                "substat",
                &substat_context,
            )?;
            if !substat.value.is_finite() || substat.value < 0.0 {
                return Err(archive_error(
                    "HSR-CAPTURE-STAT",
                    format!("{substat_context} has an invalid display value"),
                ));
            }
            Ok(ObservedSubstat {
                stat_key: stat_key.to_string(),
                value: normalize_display_number(substat.value),
            })
        })
        .collect::<HsrResult<Vec<_>>>()?;
    substats.sort_by(|left, right| left.stat_key.cmp(&right.stat_key));

    Ok(ObservedGear {
        piece_id: gear_reference.game_id,
        level,
        main_stat_key: main_stat_key.to_string(),
        main_stat_value: normalize_display_number(main_stat_value),
        substats,
        equipped_character_id: parse_location(&entry.location, references, &context)?,
        lock: entry.lock,
        discard: entry.discard,
    })
}

fn collection_coverage<T>(collection: &Option<Vec<T>>) -> CoverageLevel {
    if collection.as_ref().is_some_and(|values| !values.is_empty()) {
        CoverageLevel::Complete
    } else {
        CoverageLevel::Unknown
    }
}

fn parse_location(
    location: &str,
    references: &ReferenceCache,
    context: &str,
) -> HsrResult<Option<u32>> {
    let location = location.trim();
    if location.is_empty() {
        return Ok(None);
    }
    let id = location
        .parse::<u32>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| {
            archive_error(
                "HSR-CAPTURE-LOCATION",
                format!("{context} contains an invalid public equipment location; value redacted"),
            )
        })?;
    require_reference(
        references.character(id).is_some(),
        "equippedCharacter",
        context,
    )?;
    Ok(Some(id))
}

fn parse_slot(value: &str, context: &str) -> HsrResult<GearSlot> {
    match value.trim() {
        "Head" => Ok(GearSlot::Head),
        "Hands" | "Hand" => Ok(GearSlot::Hands),
        "Body" => Ok(GearSlot::Body),
        "Feet" | "Foot" => Ok(GearSlot::Feet),
        "Planar Sphere" | "PlanarSphere" => Ok(GearSlot::PlanarSphere),
        "Link Rope" | "LinkRope" => Ok(GearSlot::LinkRope),
        _ => Err(archive_error(
            "HSR-CAPTURE-SLOT",
            format!("{context} contains an unsupported relic slot"),
        )),
    }
}

fn main_stat_key(value: &str, slot: GearSlot) -> Option<&'static str> {
    match (value.trim(), slot) {
        ("HP", GearSlot::Head) => Some("HPDelta"),
        ("ATK", GearSlot::Hands) => Some("AttackDelta"),
        ("HP", _) => Some("HPAddedRatio"),
        ("ATK", _) => Some("AttackAddedRatio"),
        ("DEF", _) => Some("DefenceAddedRatio"),
        ("CRIT Rate", _) => Some("CriticalChanceBase"),
        ("CRIT DMG", _) => Some("CriticalDamageBase"),
        ("Outgoing Healing Boost", _) => Some("HealRatioBase"),
        ("SPD", _) => Some("SpeedDelta"),
        ("Effect Hit Rate", _) => Some("StatusProbabilityBase"),
        ("Physical DMG Boost", _) => Some("PhysicalAddedRatio"),
        ("Fire DMG Boost", _) => Some("FireAddedRatio"),
        ("Ice DMG Boost", _) => Some("IceAddedRatio"),
        ("Lightning DMG Boost", _) => Some("ThunderAddedRatio"),
        ("Wind DMG Boost", _) => Some("WindAddedRatio"),
        ("Quantum DMG Boost", _) => Some("QuantumAddedRatio"),
        ("Imaginary DMG Boost", _) => Some("ImaginaryAddedRatio"),
        ("Break Effect", _) => Some("BreakDamageAddedRatioBase"),
        ("Energy Regeneration Rate", _) => Some("SPRatioBase"),
        // Canonical GIlore keys are accepted for sanitized compatibility
        // fixtures, but still must resolve in the pinned reference cache.
        ("HPDelta", _) => Some("HPDelta"),
        ("AttackDelta", _) => Some("AttackDelta"),
        ("HPAddedRatio", _) => Some("HPAddedRatio"),
        ("AttackAddedRatio", _) => Some("AttackAddedRatio"),
        ("DefenceAddedRatio", _) => Some("DefenceAddedRatio"),
        ("CriticalChanceBase", _) => Some("CriticalChanceBase"),
        ("CriticalDamageBase", _) => Some("CriticalDamageBase"),
        ("HealRatioBase", _) => Some("HealRatioBase"),
        ("SpeedDelta", _) => Some("SpeedDelta"),
        ("StatusProbabilityBase", _) => Some("StatusProbabilityBase"),
        ("PhysicalAddedRatio", _) => Some("PhysicalAddedRatio"),
        ("FireAddedRatio", _) => Some("FireAddedRatio"),
        ("IceAddedRatio", _) => Some("IceAddedRatio"),
        ("ThunderAddedRatio", _) => Some("ThunderAddedRatio"),
        ("WindAddedRatio", _) => Some("WindAddedRatio"),
        ("QuantumAddedRatio", _) => Some("QuantumAddedRatio"),
        ("ImaginaryAddedRatio", _) => Some("ImaginaryAddedRatio"),
        ("BreakDamageAddedRatioBase", _) => Some("BreakDamageAddedRatioBase"),
        ("SPRatioBase", _) => Some("SPRatioBase"),
        _ => None,
    }
}

fn substat_key(value: &str) -> Option<&'static str> {
    match value.trim() {
        "HP" | "HPDelta" => Some("HPDelta"),
        "ATK" | "AttackDelta" => Some("AttackDelta"),
        "DEF" | "DefenceDelta" => Some("DefenceDelta"),
        "HP_" | "HPAddedRatio" => Some("HPAddedRatio"),
        "ATK_" | "AttackAddedRatio" => Some("AttackAddedRatio"),
        "DEF_" | "DefenceAddedRatio" => Some("DefenceAddedRatio"),
        "CRIT Rate_" | "CriticalChanceBase" => Some("CriticalChanceBase"),
        "CRIT DMG_" | "CriticalDamageBase" => Some("CriticalDamageBase"),
        "SPD" | "SpeedDelta" => Some("SpeedDelta"),
        "Effect Hit Rate_" | "StatusProbabilityBase" => Some("StatusProbabilityBase"),
        "Effect RES_" | "StatusResistanceBase" => Some("StatusResistanceBase"),
        "Break Effect_" | "BreakDamageAddedRatioBase" => Some("BreakDamageAddedRatioBase"),
        _ => None,
    }
}

fn bounded_u8(value: u32, minimum: u32, maximum: u32, context: &str) -> HsrResult<u8> {
    if !(minimum..=maximum).contains(&value) {
        return Err(archive_error(
            "HSR-CAPTURE-RANGE",
            format!("{context} is outside the supported range {minimum}..={maximum}"),
        ));
    }
    u8::try_from(value).map_err(|_| {
        archive_error(
            "HSR-CAPTURE-RANGE",
            format!("{context} cannot be represented safely"),
        )
    })
}

fn normalize_display_number(value: f64) -> f64 {
    if value == -0.0 {
        0.0
    } else {
        value
    }
}

fn compare_light_cones(left: &ObservedLightCone, right: &ObservedLightCone) -> Ordering {
    left.light_cone_id
        .cmp(&right.light_cone_id)
        .then_with(|| left.level.cmp(&right.level))
        .then_with(|| left.ascension.cmp(&right.ascension))
        .then_with(|| left.superimposition.cmp(&right.superimposition))
        .then_with(|| left.equipped_character_id.cmp(&right.equipped_character_id))
        .then_with(|| left.lock.cmp(&right.lock))
}

fn compare_gear(left: &ObservedGear, right: &ObservedGear) -> Ordering {
    left.piece_id
        .cmp(&right.piece_id)
        .then_with(|| left.level.cmp(&right.level))
        .then_with(|| left.main_stat_key.cmp(&right.main_stat_key))
        .then_with(|| left.main_stat_value.total_cmp(&right.main_stat_value))
        .then_with(|| compare_substats(&left.substats, &right.substats))
        .then_with(|| left.equipped_character_id.cmp(&right.equipped_character_id))
        .then_with(|| left.lock.cmp(&right.lock))
        .then_with(|| left.discard.cmp(&right.discard))
}

fn compare_substats(left: &[ObservedSubstat], right: &[ObservedSubstat]) -> Ordering {
    for (left, right) in left.iter().zip(right) {
        let ordering = left
            .stat_key
            .cmp(&right.stat_key)
            .then_with(|| left.value.total_cmp(&right.value));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

fn validate_archive_header(archive: &ReliquaryArchive) -> HsrResult<()> {
    if archive.source != RELIQUARY_ARCHIVE_SOURCE {
        return Err(archive_error(
            "HSR-CAPTURE-SOURCE",
            "archive source is not the audited reliquary_archiver boundary",
        ));
    }
    if archive.build != RELIQUARY_ARCHIVER_RELEASE {
        return Err(archive_error(
            "HSR-CAPTURE-BUILD",
            format!("archive build is unsupported; expected={RELIQUARY_ARCHIVER_RELEASE}"),
        ));
    }
    if archive.version != RELIQUARY_ARCHIVE_FORMAT_VERSION {
        return Err(archive_error(
            "HSR-CAPTURE-VERSION",
            format!("archive format is unsupported; expected={RELIQUARY_ARCHIVE_FORMAT_VERSION}"),
        ));
    }
    Ok(())
}

fn validate_reference_boundary(references: &ReferenceCache) -> HsrResult<()> {
    if references.provider() != GILORE_PROVIDER || references.schema_version() != 1 {
        return Err(HsrError::new(
            "HSR-CAPTURE-REFERENCE",
            hints::REFERENCE_INVALID,
            "packet capture requires gilore.ggstarrail-reference schemaVersion=1",
        ));
    }
    Ok(())
}

fn require_reference(found: bool, kind: &str, context: &str) -> HsrResult<()> {
    if found {
        Ok(())
    } else {
        Err(missing_reference(kind, context))
    }
}

fn missing_reference(kind: &str, context: &str) -> HsrError {
    HsrError::new(
        "HSR-REF-MISSING",
        hints::REFERENCE_MISSING,
        format!("kind={kind}; record={context}; observed identifier redacted"),
    )
}

fn parse_sha256(value: &str) -> HsrResult<[u8; 32]> {
    let value = value.trim();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(capture_error(
            "HSR-CAPTURE-CHECKSUM",
            "expected helper SHA-256 must contain exactly 64 hexadecimal characters",
        ));
    }
    let mut result = [0_u8; 32];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| {
            capture_error("HSR-CAPTURE-CHECKSUM", "expected helper SHA-256 is invalid")
        })?;
    }
    Ok(result)
}

fn stage_verified_executable(
    source_path: &Path,
    expected_sha256: &[u8; 32],
    private_directory: &Path,
) -> HsrResult<PathBuf> {
    let metadata = fs::symlink_metadata(source_path).map_err(|error| {
        capture_error(
            "HSR-CAPTURE-HELPER",
            format!("could not inspect the pinned helper; cause={error}"),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(capture_error(
            "HSR-CAPTURE-HELPER",
            "pinned helper path must be a regular non-symlink file",
        ));
    }
    if metadata.len() > MAX_HELPER_BYTES {
        return Err(capture_error(
            "HSR-CAPTURE-HELPER",
            "pinned helper exceeds the 256 MiB staging limit",
        ));
    }
    let mut source = File::open(source_path).map_err(|error| {
        capture_error(
            "HSR-CAPTURE-HELPER",
            format!("could not open the pinned helper; cause={error}"),
        )
    })?;
    let staged_path = private_directory.join("verified-reliquary-archiver.exe");
    let mut staged = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged_path)
        .map_err(|error| {
            capture_error(
                "HSR-CAPTURE-HELPER",
                format!("could not reserve the private helper copy; cause={error}"),
            )
        })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        let read = source.read(&mut buffer).map_err(|error| {
            capture_error(
                "HSR-CAPTURE-HELPER",
                format!("could not hash the pinned helper; cause={error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .filter(|size| *size <= MAX_HELPER_BYTES)
            .ok_or_else(|| {
                capture_error(
                    "HSR-CAPTURE-HELPER",
                    "pinned helper grew beyond the 256 MiB staging limit",
                )
            })?;
        hasher.update(&buffer[..read]);
        staged.write_all(&buffer[..read]).map_err(|error| {
            capture_error(
                "HSR-CAPTURE-HELPER",
                format!("could not write the private helper copy; cause={error}"),
            )
        })?;
    }
    let actual: [u8; 32] = hasher.finalize().into();
    if &actual != expected_sha256 {
        return Err(capture_error(
            "HSR-CAPTURE-CHECKSUM",
            "helper SHA-256 does not match the user-approved pin",
        ));
    }
    staged
        .flush()
        .and_then(|()| staged.sync_all())
        .map_err(|error| {
            capture_error(
                "HSR-CAPTURE-HELPER",
                format!("could not finalize the private helper copy; cause={error}"),
            )
        })?;
    drop(staged);
    fs::set_permissions(&staged_path, metadata.permissions()).map_err(|error| {
        capture_error(
            "HSR-CAPTURE-HELPER",
            format!("could not preserve helper execution permissions; cause={error}"),
        )
    })?;

    let staged_metadata = fs::symlink_metadata(&staged_path).map_err(|error| {
        capture_error(
            "HSR-CAPTURE-HELPER",
            format!("could not re-inspect the private helper copy; cause={error}"),
        )
    })?;
    if staged_metadata.file_type().is_symlink()
        || !staged_metadata.is_file()
        || staged_metadata.len() != copied
    {
        return Err(capture_error(
            "HSR-CAPTURE-HELPER",
            "private helper copy changed before execution",
        ));
    }
    let staged_hash = sha256_regular_file(&staged_path)?;
    if &staged_hash != expected_sha256 {
        return Err(capture_error(
            "HSR-CAPTURE-CHECKSUM",
            "private helper copy changed before execution",
        ));
    }
    Ok(staged_path)
}

fn sha256_regular_file(path: &Path) -> HsrResult<[u8; 32]> {
    let mut file = File::open(path).map_err(|error| {
        capture_error(
            "HSR-CAPTURE-HELPER",
            format!("could not reopen the private helper copy; cause={error}"),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            capture_error(
                "HSR-CAPTURE-HELPER",
                format!("could not re-hash the private helper copy; cause={error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn read_bounded_regular_file(path: &Path) -> HsrResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        HsrError::new(
            "HSR-CAPTURE-READ",
            hints::READ_FAILED,
            format!("could not inspect archive output; cause={error}"),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(HsrError::new(
            "HSR-CAPTURE-READ",
            hints::READ_FAILED,
            "archive output must be a regular non-symlink file",
        ));
    }
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(archive_error(
            "HSR-CAPTURE-SIZE",
            "archive exceeds the 64 MiB import limit",
        ));
    }
    let file = File::open(path).map_err(|error| {
        HsrError::new(
            "HSR-CAPTURE-READ",
            hints::READ_FAILED,
            format!("could not open archive output; cause={error}"),
        )
    })?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_ARCHIVE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            HsrError::new(
                "HSR-CAPTURE-READ",
                hints::READ_FAILED,
                format!("could not read archive output; cause={error}"),
            )
        })?;
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(archive_error(
            "HSR-CAPTURE-SIZE",
            "archive grew beyond the 64 MiB import limit while reading",
        ));
    }
    Ok(bytes)
}

fn reject_embedded_packet_data(value: &Value, path: &str) -> HsrResult<()> {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let normalized = normalize_field_name(key);
                if is_raw_packet_field(&normalized) {
                    return Err(HsrError::new(
                        "HSR-CAPTURE-RAW-PACKET",
                        hints::SENSITIVE_DATA,
                        format!("prohibited raw packet field at {path}.{key}"),
                    ));
                }
                reject_embedded_packet_data(child, &format!("{path}.{key}"))?;
            }
        },
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                reject_embedded_packet_data(child, &format!("{path}[{index}]"))?;
            }
        },
        _ => {},
    }
    Ok(())
}

fn discard_private_archive_fields(value: &mut Value, root: bool) {
    match value {
        Value::Object(fields) => {
            let keys: Vec<String> = fields.keys().cloned().collect();
            for key in keys {
                let normalized = normalize_field_name(&key);
                let private_root_section =
                    root && matches!(normalized.as_str(), "metadata" | "gacha" | "materials");
                if private_root_section || is_identifier_field(&normalized) {
                    fields.remove(&key);
                } else if let Some(child) = fields.get_mut(&key) {
                    discard_private_archive_fields(child, false);
                }
            }
        },
        Value::Array(values) => {
            for child in values {
                discard_private_archive_fields(child, false);
            }
        },
        _ => {},
    }
}

fn normalize_field_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_identifier_field(normalized: &str) -> bool {
    matches!(
        normalized,
        "uid"
            | "guid"
            | "uniqueid"
            | "server"
            | "serverid"
            | "serveritemid"
            | "accountid"
            | "accountuid"
            | "userid"
            | "useruid"
            | "playerid"
            | "playeruid"
            | "playername"
            | "nickname"
            | "localid"
            | "deviceid"
    ) || normalized.ends_with("uid")
        || normalized.ends_with("guid")
        || normalized.ends_with("serveritemid")
}

fn is_raw_packet_field(normalized: &str) -> bool {
    matches!(
        normalized,
        "packet"
            | "packets"
            | "packetdata"
            | "packetbytes"
            | "rawpacket"
            | "rawpackets"
            | "rawpacketdata"
            | "rawpacketbytes"
            | "pcap"
            | "pcapng"
            | "etl"
    ) || normalized.contains("rawpacket")
}

fn capture_error(code: &'static str, detail: impl Into<String>) -> HsrError {
    HsrError::new(code, hints::DEVICE_UNAVAILABLE, detail)
}

fn archive_error(code: &'static str, detail: impl Into<String>) -> HsrError {
    HsrError::new(code, hints::OBSERVATION_INVALID, detail)
}

struct PrivateCaptureOutput {
    temporary_root: PathBuf,
    directory: PathBuf,
    path: PathBuf,
    cleaned: bool,
}

impl PrivateCaptureOutput {
    fn create() -> io::Result<Self> {
        let base = fs::canonicalize(std::env::temp_dir())?;
        for _ in 0..64 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let candidate = base.join(format!(
                "goodscanner-hsr-capture-{}-{timestamp:x}-{sequence:x}",
                std::process::id()
            ));
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&candidate) {
                Ok(()) => {
                    let directory = match fs::canonicalize(&candidate) {
                        Ok(directory) => directory,
                        Err(error) => {
                            let _ = fs::remove_dir(&candidate);
                            return Err(error);
                        },
                    };
                    if !directory.starts_with(&base) {
                        let _ = fs::remove_dir(&candidate);
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "private output escaped the OS temporary directory",
                        ));
                    }
                    return Ok(Self {
                        temporary_root: base,
                        path: directory.join("archive.json"),
                        directory,
                        cleaned: false,
                    });
                },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a unique private capture directory",
        ))
    }

    fn directory(&self) -> &Path {
        &self.directory
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn finish<T, F>(&mut self, result: HsrResult<T>, cleanup: F) -> HsrResult<T>
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        let prior_code = result.as_ref().err().map_or("none", |error| error.code());
        match self.cleanup_with(cleanup) {
            Ok(()) => result,
            Err(error) => Err(HsrError::new(
                "HSR-CAPTURE-CLEANUP",
                hints::WRITE_FAILED,
                format!(
                    "private temporary directory cleanup failed; path={}; priorErrorCode={prior_code}; cause={error}",
                    self.directory.display()
                ),
            )),
        }
    }

    fn cleanup_with<F>(&mut self, cleanup: F) -> io::Result<()>
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        if self.cleaned {
            return Ok(());
        }
        let resolved = match fs::canonicalize(&self.directory) {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleaned = true;
                return Ok(());
            },
            Err(error) => return Err(error),
        };
        if resolved == self.temporary_root
            || resolved != self.directory
            || !resolved.starts_with(&self.temporary_root)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private capture directory identity changed before cleanup",
            ));
        }
        cleanup(&resolved)?;
        match fs::symlink_metadata(&resolved) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleaned = true;
                Ok(())
            },
            Ok(_) => Err(io::Error::other(
                "private capture directory still exists after cleanup",
            )),
            Err(error) => Err(error),
        }
    }
}

impl Drop for PrivateCaptureOutput {
    fn drop(&mut self) {
        // Explicit finish handles every normal return. This remains only a
        // best-effort unwind/process-error fallback and keeps the same resolved
        // path containment checks.
        let _ = self.cleanup_with(|path| fs::remove_dir_all(path));
    }
}
