/// Capture monitor: orchestrates packet capture, decryption, and data accumulation.
///
/// Ported from irminsul's `monitor.rs`, simplified for yas integration.
/// The monitor runs on a tokio runtime and communicates via channels.
///
/// ## Version resilience
///
/// Both command IDs and protobuf field numbers change across game versions.
/// The monitor is resilient to both:
///
/// - **Command IDs**: every decrypted command is tested heuristically,
///   regardless of its `command_id`.
/// - **Outer field numbers and wrappers**: instead of relying on a fixed proto
///   schema for the outer container, we first try every repeated
///   length-delimited field. Item packets also get a wire-level fallback that
///   finds structurally valid `Item` records inside opaque/prefixed wrappers,
///   without assuming a wrapper field number, item tag, or byte offset.
///
/// Dispatch keys are loaded from an external `keys/gi.json` file first
/// (next to the exe), falling back to the compile-time embedded copy.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use auto_artifactarium::r#gen::protos::{AvatarInfo, Item, Unk};
use auto_artifactarium::{GamePacket, GameSniffer};
use base64::prelude::*;
use protobuf::Message;
use protobuf::UnknownValueRef;
use tokio::sync::mpsc;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;
use yas::{log_debug, log_error, log_info, log_warn};

use super::data_cache::load_data_cache;
use super::packet_capture::{CaptureError, PacketCapture};
use super::player_data::{CaptureExportSettings, PlayerData};

// --- Heuristic thresholds for field-number-agnostic packet matching ---
// These control how many valid sub-messages a repeated field must contain
// before we accept it as an item/avatar packet. Tuned to avoid false positives
// while accepting real game data (accounts typically have 100+ items, 20+ avatars).
const MIN_ITEM_ENTRIES: usize = 10;
const MIN_GEAR_COUNT: usize = 5; // items with actual weapon/reliquary equip data
const MIN_AVATAR_ENTRIES: usize = 4;
const MIN_AVATARS_WITH_PROPS: usize = 2; // avatars with non-empty prop_map
const MIN_ACHIEVEMENT_ENTRIES: usize = 5;
const MIN_ACHIEVEMENT_ID: u64 = 80_000;
const MAX_ACHIEVEMENT_ID_EXCLUSIVE: u64 = 100_000;
const MIN_FINISH_TIMESTAMP: u64 = 1_500_000_000;
const MAX_FINISH_TIMESTAMP: u64 = 4_102_444_800; // 2100-01-01 UTC
use crate::scanner::common::models::GoodExport;

/// Commands the UI can send to the monitor.
pub enum CaptureCommand {
    StartCapture {
        include_achievements: bool,
    },
    StopCapture,
    Export {
        settings: CaptureExportSettings,
        reply: tokio::sync::oneshot::Sender<Result<GoodExport>>,
    },
}

/// State shared between the monitor and UI.
#[derive(Clone, Debug)]
pub struct CaptureState {
    pub capturing: bool,
    /// All requested packet categories have been received; capture auto-stopped.
    pub complete: bool,
    pub has_characters: bool,
    pub has_items: bool,
    pub has_achievements: bool,
    pub character_count: usize,
    pub weapon_count: usize,
    pub artifact_count: usize,
    pub achievement_count: usize,
    pub error: Option<String>,
}

impl Default for CaptureState {
    fn default() -> Self {
        Self {
            capturing: false,
            complete: false,
            has_characters: false,
            has_items: false,
            has_achievements: false,
            character_count: 0,
            weapon_count: 0,
            artifact_count: 0,
            achievement_count: 0,
            error: None,
        }
    }
}

/// The capture monitor. Runs on a tokio runtime.
pub struct CaptureMonitor {
    player_data: PlayerData,
    sniffer: GameSniffer,
    state: Arc<Mutex<CaptureState>>,
    capture_cancel_token: Option<CancellationToken>,
    capture_task: Option<JoinHandle<Result<()>>>,
    packet_tx: mpsc::UnboundedSender<Vec<u8>>,
    packet_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    dump_packets: bool,
    dump_dir: std::path::PathBuf,
    dump_counter: u32,
    require_achievements: bool,
}

impl CaptureMonitor {
    /// Initialize the monitor: load data cache, set up sniffer.
    pub fn new(state: Arc<Mutex<CaptureState>>, dump_packets: bool) -> Result<Self> {
        #[cfg(target_os = "windows")]
        {
            yas::utils::ensure_admin()?;
        }

        let data_cache = load_data_cache()?;
        let player_data = PlayerData::new(data_cache);
        let keys = load_keys()?;
        let sniffer = GameSniffer::new().set_initial_keys(keys);
        let (packet_tx, packet_rx) = mpsc::unbounded_channel();

        let dump_dir = crate::cli::exe_dir().join("debug_capture");
        if dump_packets {
            std::fs::create_dir_all(&dump_dir).with_context(|| {
                format!(
                    "packet dump directory could not be created: {}",
                    dump_dir.display()
                )
            })?;
            log_info!(
                "数据包转储已开启 → {}",
                "Packet dump enabled → {}",
                dump_dir.display(),
            );
        }

        Ok(Self {
            player_data,
            sniffer,
            state,
            capture_cancel_token: None,
            capture_task: None,
            packet_tx,
            packet_rx,
            dump_packets,
            dump_dir,
            dump_counter: 0,
            require_achievements: false,
        })
    }

    /// Main event loop. Processes packets and UI commands.
    pub async fn run(mut self, mut cmd_rx: mpsc::UnboundedReceiver<CaptureCommand>) {
        loop {
            tokio::select! {
                Some(packet) = self.packet_rx.recv() => {
                    self.handle_packet(packet);
                }
                command = cmd_rx.recv() => {
                    match command {
                        Some(cmd) => {
                            if self.handle_command(cmd) {
                                break;
                            }
                        },
                        None => break,
                    }
                }
                result = wait_for_capture_task(&mut self.capture_task) => {
                    self.handle_capture_task_result(result);
                }
                else => break,
            }
        }
    }

    /// Returns true if the loop should exit.
    fn handle_command(&mut self, cmd: CaptureCommand) -> bool {
        match cmd {
            CaptureCommand::StartCapture {
                include_achievements,
            } => {
                if self.capture_task.is_some() {
                    return false;
                }
                self.player_data.begin_capture();
                self.require_achievements = include_achievements;
                let cancel_token = CancellationToken::new();
                if let Ok(mut state) = self.state.lock() {
                    *state = CaptureState {
                        capturing: true,
                        ..CaptureState::default()
                    };
                }
                self.capture_task = Some(tokio::spawn(capture_task(
                    cancel_token.clone(),
                    self.packet_tx.clone(),
                )));
                self.capture_cancel_token = Some(cancel_token);
            },
            CaptureCommand::StopCapture => {
                self.stop_capture();
            },
            CaptureCommand::Export { settings, reply } => {
                let result = self.player_data.export(&settings);
                let _ = reply.send(result);
            },
        }
        false
    }

    fn stop_capture(&mut self) {
        if let Some(token) = &self.capture_cancel_token {
            token.cancel();
        }
        if let Ok(mut state) = self.state.lock() {
            state.capturing = false;
        }
    }

    fn handle_capture_task_result(&mut self, result: std::result::Result<Result<()>, JoinError>) {
        let expected_stop = self
            .capture_cancel_token
            .as_ref()
            .map_or(false, CancellationToken::is_cancelled);
        self.capture_task = None;
        self.capture_cancel_token = None;
        update_state_from_capture_task(&self.state, result, expected_stop);
    }

    fn handle_packet(&mut self, packet: Vec<u8>) {
        let Some(GamePacket::Commands(commands)) = self.sniffer.receive_packet(packet) else {
            return;
        };

        // Heuristic matching: try parsing every command as item/avatar packets
        // regardless of command_id.  This survives command ID rotation across
        // game versions.
        for command in commands {
            // Dump raw decrypted commands when enabled
            if self.dump_packets {
                let path = self.dump_dir.join(format!(
                    "{:06}_cmd{}.bin",
                    self.dump_counter, command.command_id
                ));
                if let Err(e) = std::fs::write(&path, &command.proto_data) {
                    log_warn!(
                        "无法将抓包调试数据写入 {}。请检查磁盘空间和文件夹权限。完整错误详情: {:#}",
                        "Captured debug data could not be written to {}. Check disk space and folder permissions. Full error details: {:#}",
                        path.display(),
                        e,
                    );
                }
                self.dump_counter += 1;
            }

            if let Some(achievement_ids) = try_match_achievements(&command.proto_data) {
                log_info!(
                    "捕获到成就数据包 (cmd={})，共 {} 个已完成成就",
                    "Captured achievement packet (cmd={}), {} completed achievements",
                    command.command_id,
                    achievement_ids.len(),
                );
                self.player_data.process_achievements(&achievement_ids);
                if let Ok(mut state) = self.state.lock() {
                    state.has_achievements = true;
                    state.achievement_count = self.player_data.achievement_count();
                }
            } else if let Some(items) = try_match_items(&command.proto_data) {
                log_info!(
                    "捕获到物品数据包 (cmd={})，共 {} 个物品",
                    "Captured item packet (cmd={}), {} items",
                    command.command_id,
                    items.len(),
                );
                self.player_data.process_items(&items);
                if let Ok(mut state) = self.state.lock() {
                    state.weapon_count = self.player_data.weapon_count();
                    state.artifact_count = self.player_data.artifact_count();
                    // Recent game versions may split store categories across
                    // multiple notifications. Do not stop after a weapon-only
                    // batch before the artifact batch has arrived.
                    state.has_items = state.weapon_count > 0 && state.artifact_count > 0;
                }
            } else if let Some(avatars) = try_match_avatars(&command.proto_data) {
                log_info!(
                    "捕获到角色数据包 (cmd={})，共 {} 个角色",
                    "Captured avatar packet (cmd={}), {} avatars",
                    command.command_id,
                    avatars.len(),
                );
                self.player_data.process_characters(&avatars);
                if let Ok(mut state) = self.state.lock() {
                    state.has_characters = true;
                    state.character_count = self.player_data.character_count();
                }
            }
        }

        // Auto-stop once the normal GOOD data and every requested extension
        // have arrived. Achievement capture is optional for backwards-compatible
        // workflows, but defaults on in GOODCapture.
        let should_stop = self.state.lock().map_or(false, |s| {
            s.has_characters
                && s.has_items
                && (!self.require_achievements || s.has_achievements)
                && s.capturing
        });
        if should_stop {
            log_info!(
                "已收集到所有数据，自动停止抓包",
                "All data collected, stopping capture automatically"
            );
            self.stop_capture();
            if let Ok(mut state) = self.state.lock() {
                state.complete = true;
            }
        }
    }
}

async fn wait_for_capture_task(
    task: &mut Option<JoinHandle<Result<()>>>,
) -> std::result::Result<Result<()>, JoinError> {
    match task {
        Some(task) => task.await,
        None => std::future::pending().await,
    }
}

fn capture_task_failure(
    result: std::result::Result<Result<()>, JoinError>,
    expected_stop: bool,
) -> Option<String> {
    match result {
        Ok(Ok(())) if expected_stop => None,
        Ok(Ok(())) => Some("capture task exited unexpectedly without reporting an error".into()),
        Ok(Err(error)) => Some(format!("{error:#}")),
        Err(join_error) if join_error.is_panic() => {
            let fallback = join_error.to_string();
            let payload = join_error.into_panic();
            let message = payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or(fallback);
            Some(format!("capture task panicked: {message}"))
        },
        Err(join_error) => Some(format!(
            "capture task was cancelled unexpectedly: {join_error}"
        )),
    }
}

fn update_state_from_capture_task(
    state: &Arc<Mutex<CaptureState>>,
    result: std::result::Result<Result<()>, JoinError>,
    expected_stop: bool,
) {
    let error = capture_task_failure(result, expected_stop);
    if let Some(error) = &error {
        log_error!("抓包任务失败: {}", "Capture task failed: {}", error);
    }
    if let Ok(mut state) = state.lock() {
        state.capturing = false;
        if let Some(error) = error {
            state.error = Some(error);
        }
    }
}

/// Extract the repeated field with the most entries that parse as `T` and pass
/// the `filter`.  Returns `(best_field_number, parsed_entries)`.
///
/// This is the core of our field-number-agnostic packet matching: parse the
/// outer message as `Unk` (generic protobuf), group all length-delimited
/// values by field number, try parsing each group as `T`, and pick the field
/// with the most valid results.
fn find_best_field<T: Message>(
    proto_data: &[u8],
    min_entries: usize,
    filter: impl Fn(&T) -> bool,
) -> Option<(u32, Vec<T>)> {
    let unk = Unk::parse_from_bytes(proto_data).ok()?;

    let mut field_map: HashMap<u32, Vec<&[u8]>> = HashMap::new();
    for (field_num, value) in unk.unknown_fields().iter() {
        if let UnknownValueRef::LengthDelimited(bytes) = value {
            field_map.entry(field_num).or_default().push(bytes);
        }
    }

    let mut best: Option<(u32, Vec<T>)> = None;
    for (field_num, blobs) in &field_map {
        if blobs.len() < min_entries {
            continue;
        }
        let parsed: Vec<T> = blobs
            .iter()
            .filter_map(|b| T::parse_from_bytes(b).ok())
            .filter(|v| filter(v))
            .collect();
        if parsed.len() >= min_entries
            && best.as_ref().map_or(true, |(_, b)| parsed.len() > b.len())
        {
            best = Some((*field_num, parsed));
        }
    }
    best
}

/// Field-number-agnostic achievement packet detection.
///
/// Achievement entries are generic protobuf messages whose field numbers are
/// obfuscated per game version. We identify the repeated entry field by its
/// structure, then infer the ID and finish-time fields from value domains. This
/// deliberately avoids command IDs, protobuf tags, byte offsets, and a single
/// sentinel achievement ID. Only records with a finish timestamp are exported:
/// those are the achievements the account has actually completed.
fn try_match_achievements(proto_data: &[u8]) -> Option<Vec<u32>> {
    let outer = Unk::parse_from_bytes(proto_data).ok()?;
    let mut field_map: HashMap<u32, Vec<&[u8]>> = HashMap::new();
    for (field_num, value) in outer.unknown_fields().iter() {
        if let UnknownValueRef::LengthDelimited(bytes) = value {
            field_map.entry(field_num).or_default().push(bytes);
        }
    }

    let mut best: Option<(usize, Vec<u32>)> = None;
    for blobs in field_map.values() {
        if blobs.len() < MIN_ACHIEVEMENT_ENTRIES {
            continue;
        }

        let records: Vec<HashMap<u32, u64>> = blobs
            .iter()
            .filter_map(|bytes| parse_varint_record(bytes))
            .collect();
        if records.len() < MIN_ACHIEVEMENT_ENTRIES {
            continue;
        }

        let Some(id_tag) = infer_achievement_id_tag(&records) else {
            continue;
        };
        let Some(timestamp_tag) = infer_finish_timestamp_tag(&records) else {
            continue;
        };

        let mut all_ids = Vec::new();
        let mut completed_ids = Vec::new();
        for record in &records {
            let Some(&id) = record.get(&id_tag) else {
                continue;
            };
            all_ids.push(id as u32);
            if record.get(&timestamp_tag).is_some_and(|timestamp| {
                (MIN_FINISH_TIMESTAMP..=MAX_FINISH_TIMESTAMP).contains(timestamp)
            }) {
                completed_ids.push(id as u32);
            }
        }

        all_ids.sort_unstable();
        all_ids.dedup();
        // A real all-achievement notification has a unique ID on nearly every
        // repeated entry. This rejects unrelated repeated-varint messages.
        if all_ids.len() * 5 < records.len() * 4 {
            continue;
        }
        completed_ids.sort_unstable();
        completed_ids.dedup();

        if best
            .as_ref()
            .map_or(true, |(record_count, _)| records.len() > *record_count)
        {
            best = Some((records.len(), completed_ids));
        }
    }

    best.map(|(_, ids)| ids)
}

fn parse_varint_record(bytes: &[u8]) -> Option<HashMap<u32, u64>> {
    let record = Unk::parse_from_bytes(bytes).ok()?;
    let mut values = HashMap::new();
    for (field_num, value) in record.unknown_fields().iter() {
        match value {
            UnknownValueRef::Varint(value) => {
                values.insert(field_num, value);
            },
            // AchievementInfo consists solely of scalar varints. Rejecting
            // other wire types is a useful false-positive guard.
            _ => return None,
        }
    }
    (!values.is_empty()).then_some(values)
}

fn infer_achievement_id_tag(records: &[HashMap<u32, u64>]) -> Option<u32> {
    let mut counts = HashMap::<u32, usize>::new();
    for record in records {
        for (&tag, &value) in record {
            if (MIN_ACHIEVEMENT_ID..MAX_ACHIEVEMENT_ID_EXCLUSIVE).contains(&value) {
                *counts.entry(tag).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count >= MIN_ACHIEVEMENT_ENTRIES)
        .max_by_key(|(_, count)| *count)
        .map(|(tag, _)| tag)
}

fn infer_finish_timestamp_tag(records: &[HashMap<u32, u64>]) -> Option<u32> {
    let mut counts = HashMap::<u32, usize>::new();
    for record in records {
        for (&tag, &value) in record {
            if (MIN_FINISH_TIMESTAMP..=MAX_FINISH_TIMESTAMP).contains(&value) {
                *counts.entry(tag).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(tag, _)| tag)
}

/// Field-number-agnostic item packet detection.
///
/// Survives both command ID rotation AND outer field number changes.
fn try_match_items(proto_data: &[u8]) -> Option<Vec<Item>> {
    let direct = find_best_field::<Item>(proto_data, MIN_ITEM_ENTRIES, |item| {
        item.item_id != 0 && item.guid != 0
    });

    let (source, items) = match direct {
        Some((field, items)) => (format!("field={field}"), items),
        None => {
            // Avatar notifications can contain their equipped Item records.
            // They are real gear, but only a partial inventory snapshot, so
            // prefer the stronger avatar packet classification when it fits.
            if try_match_avatars(proto_data).is_some() {
                return None;
            }
            (
                "nested wire scan".to_string(),
                find_nested_items(proto_data),
            )
        },
    };

    let gear_count = items
        .iter()
        .filter(|i| i.has_equip() && (i.equip().has_weapon() || i.equip().has_reliquary()))
        .count();
    if gear_count < MIN_GEAR_COUNT {
        log_debug!(
            "物品数据包候选被拒（来源={}，{} 个物品，{} 个武器/圣遗物）",
            "Item packet candidate rejected ({}, {} items, {} weapons/artifacts)",
            source,
            items.len(),
            gear_count,
        );
        return None;
    }

    log_debug!(
        "物品数据包匹配成功（来源={}，{} 个物品）",
        "Item packet matched ({}, {} items)",
        source,
        items.len(),
    );
    Some(items)
}

/// Find equipment `Item` records anywhere in a command payload.
///
/// Some versions wrap repeated item records in an opaque blob with a non-proto
/// prefix. Walking possible length-delimited wire records byte-by-byte lets us
/// enter such blobs naturally and does not bake in the wrapper's field number,
/// the repeated-item tag, or the prefix length. Structural validation plus GUID
/// deduplication keeps unrelated protobuf messages from becoming item packets.
fn find_nested_items(data: &[u8]) -> Vec<Item> {
    let mut items_by_guid = HashMap::new();
    let mut offset = 0usize;

    while offset < data.len() {
        let Some((key, key_len)) = read_varint(&data[offset..]) else {
            offset += 1;
            continue;
        };
        if key >> 3 == 0 || key & 0x07 != 2 {
            offset += 1;
            continue;
        }

        let len_offset = offset + key_len;
        let Some((value_len, value_len_size)) = data.get(len_offset..).and_then(read_varint) else {
            offset += 1;
            continue;
        };
        let Ok(value_len) = usize::try_from(value_len) else {
            offset += 1;
            continue;
        };
        let value_start = len_offset + value_len_size;
        let Some(value_end) = value_start.checked_add(value_len) else {
            offset += 1;
            continue;
        };
        if value_end > data.len() {
            offset += 1;
            continue;
        }

        if let Ok(item) = Item::parse_from_bytes(&data[value_start..value_end]) {
            if item.item_id != 0
                && item.guid != 0
                && item.has_equip()
                && (item.equip().has_weapon() || item.equip().has_reliquary())
            {
                items_by_guid.insert(item.guid, item);
                offset = value_end;
                continue;
            }
        }

        // The value may itself contain a prefixed protobuf stream, so advance
        // by one byte rather than skipping the whole length-delimited value.
        offset += 1;
    }

    items_by_guid.into_values().collect()
}

fn read_varint(data: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (index, byte) in data.iter().copied().take(10).enumerate() {
        let shift = u32::try_from(index).ok()?.checked_mul(7)?;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
    }
    None
}

/// Field-number-agnostic avatar packet detection.
///
/// Requires ≥4 avatars and ≥2 with non-empty `prop_map`
/// (every real avatar has property entries like level/ascension).
fn try_match_avatars(proto_data: &[u8]) -> Option<Vec<AvatarInfo>> {
    let (field, avatars) = find_best_field::<AvatarInfo>(proto_data, MIN_AVATAR_ENTRIES, |a| {
        a.avatar_id != 0 && a.guid != 0
    })?;

    let has_props = avatars.iter().filter(|a| !a.prop_map.is_empty()).count();
    if has_props < MIN_AVATARS_WITH_PROPS {
        log_debug!(
            "角色数据包候选被拒（field={}, {} 个角色，仅 {} 个有属性）",
            "Avatar packet candidate rejected (field={}, {} avatars, only {} with props)",
            field,
            avatars.len(),
            has_props,
        );
        return None;
    }

    log_debug!(
        "角色数据包匹配成功（field={}, {} 个角色）",
        "Avatar packet matched (field={}, {} avatars)",
        field,
        avatars.len(),
    );
    Some(avatars)
}

async fn capture_task(
    cancel_token: CancellationToken,
    packet_tx: mpsc::UnboundedSender<Vec<u8>>,
) -> Result<()> {
    let mut capture = PacketCapture::new().map_err(|error| {
        capture_error_with_context(error, "创建抓包失败 / Error creating packet capture")
    })?;
    log_info!("开始抓包", "Starting packet capture");
    loop {
        let packet = tokio::select!(
            packet = capture.next_packet() => packet,
            _ = cancel_token.cancelled() => break,
        );
        let packet = match packet {
            Ok(packet) => packet,
            Err(error) => {
                handle_capture_read_error(error)?;
                continue;
            },
        };
        if let Err(e) = packet_tx.send(packet) {
            log_error!("发送数据包出错: {}", "Error sending captured packet: {}", e);
        }
    }
    log_info!("抓包已停止", "Packet capture stopped");
    Ok(())
}

/// Add user-facing operation context without flattening the nested pktmon error.
///
/// `CaptureError` predates `std::error::Error` support, but its variants retain
/// the original `anyhow::Error`. Destructuring it here keeps that source chain
/// available to the GUI's alternate (`{:#}`) formatter.
fn capture_error_with_context(error: CaptureError, operation: &'static str) -> anyhow::Error {
    let error = match error {
        CaptureError::Filter(source) => source.context("pktmon packet-filter setup failed"),
        CaptureError::Capture {
            has_captured,
            error: source,
        } => source.context(format!(
            "pktmon capture failed (has_captured = {has_captured})"
        )),
        CaptureError::CaptureClosed => anyhow!("packet capture stream closed"),
    };
    error.context(operation)
}

/// Return permanent stream closure to the retained task so the monitor can
/// publish it to `CaptureState::error`. Other receive failures remain retryable.
fn handle_capture_read_error(error: CaptureError) -> Result<()> {
    match error {
        error @ CaptureError::CaptureClosed => Err(capture_error_with_context(
            error,
            "抓包数据流意外关闭 / Packet capture stream closed unexpectedly",
        )),
        error => {
            let error = capture_error_with_context(
                error,
                "接收抓包数据失败 / Failed to receive captured packet",
            );
            log_error!(
                "接收数据包时出现问题，将继续重试。完整错误详情: {:#}",
                "A problem occurred while receiving a packet; capture will retry. Full error details: {:#}",
                error
            );
            Ok(())
        },
    }
}

/// Load dispatch keys from external file first, then merge with embedded keys.
///
/// External keys (in `keys/gi.json` next to the exe) override embedded ones for
/// the same version, allowing key updates without recompiling.
fn load_keys() -> Result<HashMap<u16, Vec<u8>>> {
    let mut all_keys = HashMap::new();

    // 1. Embedded keys (compile-time fallback)
    let embedded: HashMap<u16, String> =
        serde_json::from_slice(include_bytes!("../../keys/gi.json"))?;
    for (version, b64) in &embedded {
        all_keys.insert(*version, BASE64_STANDARD.decode(b64)?);
    }

    // 2. External key file next to the exe (overrides embedded for same version)
    let external_path = crate::cli::exe_dir().join("keys").join("gi.json");
    match std::fs::read_to_string(&external_path) {
        Ok(content) => match serde_json::from_str::<HashMap<u16, String>>(&content) {
            Ok(external) => {
                let mut added = 0usize;
                for (version, b64) in &external {
                    match BASE64_STANDARD.decode(b64) {
                        Ok(decoded) => {
                            if !all_keys.contains_key(version) {
                                added += 1;
                            }
                            all_keys.insert(*version, decoded);
                        },
                        Err(error) => log_warn!(
                            "外部密钥文件中的版本 {} 无法解码；该版本将使用内置密钥（如有）。完整错误详情: {:#}",
                            "Version {} in the external key file could not be decoded; the embedded key will be used for that version when available. Full error details: {:#}",
                            version,
                            error,
                        ),
                    }
                }
                log_info!(
                    "已加载外部密钥文件（{} 个密钥，{} 个新增）",
                    "Loaded external key file ({} keys, {} new)",
                    external.len(),
                    added,
                );
            },
            Err(error) => log_warn!(
                "外部密钥文件格式错误；将改用内置密钥。完整错误详情: {:#}",
                "The external key file has an invalid format; embedded keys will be used instead. Full error details: {:#}",
                error,
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => log_warn!(
            "无法读取外部密钥文件；将改用内置密钥。完整错误详情: {:#}",
            "The external key file could not be read; embedded keys will be used instead. Full error details: {:#}",
            error,
        ),
    }

    Ok(all_keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEMS_BIN: &[u8] = include_bytes!("testdata/items.bin");
    const AVATARS_BIN: &[u8] = include_bytes!("testdata/avatars.bin");
    const NOISE_BIN: &[u8] = include_bytes!("testdata/noise.bin");

    #[test]
    fn capture_creation_error_preserves_nested_source_chain() {
        let source = anyhow!("windows error 5").context("pktmon session start failed");
        let error = capture_error_with_context(
            CaptureError::Capture {
                has_captured: false,
                error: source,
            },
            "创建抓包失败 / Error creating packet capture",
        );

        assert_eq!(
            format!("{error:#}"),
            "创建抓包失败 / Error creating packet capture: \
             pktmon capture failed (has_captured = false): \
             pktmon session start failed: windows error 5"
        );
    }

    #[test]
    fn closed_capture_stream_is_terminal_and_searchable() {
        let error = handle_capture_read_error(CaptureError::CaptureClosed)
            .expect_err("a closed capture stream must terminate the capture task");

        assert_eq!(
            format!("{error:#}"),
            "抓包数据流意外关闭 / Packet capture stream closed unexpectedly: \
             packet capture stream closed"
        );
    }

    #[tokio::test]
    async fn capture_task_error_updates_state_with_full_chain() {
        let state = Arc::new(Mutex::new(CaptureState {
            capturing: true,
            ..CaptureState::default()
        }));
        let task = tokio::spawn(async {
            Err::<(), _>(anyhow!("pktmon diagnostic").context("packet capture creation failed"))
        });

        update_state_from_capture_task(&state, task.await, false);

        let state = state.lock().unwrap();
        assert!(!state.capturing);
        assert_eq!(
            state.error.as_deref(),
            Some("packet capture creation failed: pktmon diagnostic")
        );
    }

    #[tokio::test]
    async fn capture_task_panic_updates_state_with_payload() {
        let state = Arc::new(Mutex::new(CaptureState {
            capturing: true,
            ..CaptureState::default()
        }));
        let task = tokio::spawn(async {
            panic!("pktmon access violation 0xc0000005");
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        });

        update_state_from_capture_task(&state, task.await, false);

        let state = state.lock().unwrap();
        assert!(!state.capturing);
        assert_eq!(
            state.error.as_deref(),
            Some("capture task panicked: pktmon access violation 0xc0000005")
        );
    }

    #[tokio::test]
    async fn expected_capture_task_stop_does_not_set_an_error() {
        let state = Arc::new(Mutex::new(CaptureState {
            capturing: true,
            ..CaptureState::default()
        }));
        let task = tokio::spawn(async { Ok::<(), anyhow::Error>(()) });

        update_state_from_capture_task(&state, task.await, true);

        let state = state.lock().unwrap();
        assert!(!state.capturing);
        assert!(state.error.is_none());
    }

    // --- try_match_items ---

    #[test]
    fn match_items_on_real_packet() {
        let items = try_match_items(ITEMS_BIN).expect("should match item packet");
        // Real account has thousands of items, hundreds of weapons/artifacts
        assert!(
            items.len() > 1000,
            "expected >1000 items, got {}",
            items.len()
        );
        let weapons = items
            .iter()
            .filter(|i| i.has_equip() && i.equip().has_weapon())
            .count();
        let artifacts = items
            .iter()
            .filter(|i| i.has_equip() && i.equip().has_reliquary())
            .count();
        assert!(weapons > 50, "expected >50 weapons, got {}", weapons);
        assert!(artifacts > 50, "expected >50 artifacts, got {}", artifacts);
    }

    #[test]
    fn match_items_inside_prefixed_unknown_wrapper() {
        let (_, items) = find_best_field::<Item>(ITEMS_BIN, 10, |i| i.item_id != 0 && i.guid != 0)
            .expect("fixture should contain items");

        let mut opaque = vec![0xff, 0x00, 0x7f, 0x80, 0x00];
        for item in items.into_iter().filter(|i| i.has_equip()).take(12) {
            let bytes = item.write_to_bytes().expect("serialize item");
            // Deliberately use an arbitrary two-byte field key. The matcher
            // must not depend on the historical field-2 tag (0x12).
            push_varint((37u64 << 3) | 2, &mut opaque);
            push_varint(bytes.len() as u64, &mut opaque);
            opaque.extend_from_slice(&bytes);
        }

        let mut wrapped = vec![0x91, 0x92, 0x93];
        push_varint((91u64 << 3) | 2, &mut wrapped);
        push_varint(opaque.len() as u64, &mut wrapped);
        wrapped.extend_from_slice(&opaque);

        let matched = try_match_items(&wrapped).expect("should scan nested item records");
        assert!(matched.len() >= MIN_GEAR_COUNT);
        assert!(matched.iter().all(|item| item.has_equip()));
    }

    fn push_varint(mut value: u64, output: &mut Vec<u8>) {
        while value >= 0x80 {
            output.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        output.push(value as u8);
    }

    fn push_varint_field(field: u32, value: u64, output: &mut Vec<u8>) {
        push_varint(u64::from(field) << 3, output);
        push_varint(value, output);
    }

    fn push_message_field(field: u32, message: &[u8], output: &mut Vec<u8>) {
        push_varint((u64::from(field) << 3) | 2, output);
        push_varint(message.len() as u64, output);
        output.extend_from_slice(message);
    }

    fn achievement_packet(
        outer_tag: u32,
        id_tag: u32,
        timestamp_tag: u32,
        records: &[(u32, Option<u64>)],
    ) -> Vec<u8> {
        let mut packet = Vec::new();
        for (index, &(id, timestamp)) in records.iter().enumerate() {
            let mut record = Vec::new();
            // These arbitrary status/progress tags make the fixture resemble
            // AchievementInfo without teaching the matcher fixed positions.
            push_varint_field(2, if timestamp.is_some() { 2 } else { 1 }, &mut record);
            push_varint_field(id_tag, u64::from(id), &mut record);
            push_varint_field(19, index as u64, &mut record);
            if let Some(timestamp) = timestamp {
                push_varint_field(timestamp_tag, timestamp, &mut record);
            }
            push_message_field(outer_tag, &record, &mut packet);
        }
        packet
    }

    #[test]
    fn match_achievements_infers_obfuscated_fields() {
        let packet = achievement_packet(
            47,
            11,
            29,
            &[
                (87007, Some(1_725_000_000)),
                (82003, None),
                (80001, Some(1_600_000_000)),
                (84512, None),
                (86101, Some(1_700_000_000)),
                (83009, None),
            ],
        );

        assert_eq!(
            try_match_achievements(&packet),
            Some(vec![80001, 86101, 87007])
        );
    }

    #[test]
    fn match_achievements_survives_another_tag_layout_without_sentinel_id() {
        let packet = achievement_packet(
            5,
            73,
            4,
            &[
                (81001, Some(1_650_000_000)),
                (81002, Some(1_650_000_001)),
                (81003, None),
                (81004, None),
                (81005, Some(1_650_000_002)),
            ],
        );

        assert_eq!(
            try_match_achievements(&packet),
            Some(vec![81001, 81002, 81005])
        );
    }

    #[test]
    fn match_achievements_rejects_duplicate_id_records() {
        let packet = achievement_packet(
            5,
            7,
            9,
            &[
                (80001, Some(1_650_000_000)),
                (80001, Some(1_650_000_001)),
                (80001, Some(1_650_000_002)),
                (80001, Some(1_650_000_003)),
                (80001, Some(1_650_000_004)),
            ],
        );

        assert!(try_match_achievements(&packet).is_none());
        assert!(try_match_achievements(NOISE_BIN).is_none());
    }

    #[test]
    fn match_items_rejects_avatar_packet() {
        assert!(
            try_match_items(AVATARS_BIN).is_none(),
            "avatar packet should not match as items"
        );
    }

    #[test]
    fn match_items_rejects_noise() {
        assert!(try_match_items(NOISE_BIN).is_none());
    }

    #[test]
    fn match_items_rejects_empty() {
        assert!(try_match_items(&[]).is_none());
    }

    // --- try_match_avatars ---

    #[test]
    fn match_avatars_on_real_packet() {
        let avatars = try_match_avatars(AVATARS_BIN).expect("should match avatar packet");
        assert!(
            avatars.len() > 20,
            "expected >20 avatars, got {}",
            avatars.len()
        );
        // All real avatars should have prop_map with level
        let with_level = avatars
            .iter()
            .filter(|a| a.prop_map.contains_key(&4001))
            .count();
        assert!(
            with_level > 10,
            "expected >10 avatars with level prop, got {}",
            with_level
        );
    }

    #[test]
    fn match_avatars_rejects_item_packet() {
        assert!(
            try_match_avatars(ITEMS_BIN).is_none(),
            "item packet should not match as avatars"
        );
    }

    #[test]
    fn match_avatars_rejects_noise() {
        assert!(try_match_avatars(NOISE_BIN).is_none());
    }

    #[test]
    fn match_avatars_rejects_empty() {
        assert!(try_match_avatars(&[]).is_none());
    }

    // --- find_best_field ---

    #[test]
    fn find_best_field_returns_correct_field_number() {
        // Items are in field 3 for this game version
        let (field, items) =
            find_best_field::<Item>(ITEMS_BIN, 10, |i| i.item_id != 0 && i.guid != 0)
                .expect("should find items");
        assert_eq!(field, 3, "items should be in field 3");
        assert!(items.len() > 1000);

        // Avatars are in field 10 for this game version
        let (field, avatars) =
            find_best_field::<AvatarInfo>(AVATARS_BIN, 4, |a| a.avatar_id != 0 && a.guid != 0)
                .expect("should find avatars");
        assert_eq!(field, 10, "avatars should be in field 10");
        assert!(avatars.len() > 20);
    }

    #[test]
    fn find_best_field_respects_min_entries() {
        // With an impossibly high threshold, nothing should match
        assert!(find_best_field::<Item>(ITEMS_BIN, 100_000, |_| true).is_none());
    }

    // --- Cross-contamination: items shouldn't parse as avatars and vice versa ---

    #[test]
    fn item_data_does_not_produce_valid_avatars() {
        // Even if we lower thresholds, item blobs shouldn't parse as AvatarInfo
        // with meaningful data
        let result =
            find_best_field::<AvatarInfo>(ITEMS_BIN, 4, |a| a.avatar_id != 0 && a.guid != 0);
        // Either None, or if protobuf happens to parse garbage, the prop_map check
        // in try_match_avatars would reject it
        if let Some((_, avatars)) = result {
            let with_props = avatars.iter().filter(|a| !a.prop_map.is_empty()).count();
            assert!(
                with_props < MIN_AVATARS_WITH_PROPS,
                "item data shouldn't produce avatars with valid props"
            );
        }
    }
}
