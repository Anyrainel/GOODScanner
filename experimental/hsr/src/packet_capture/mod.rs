//! In-process, read-only capture of Honkai: Star Rail inventory and achievements.
//!
//! The native boundary captures only HSR UDP traffic on the two established
//! game ports. [`HsrPacketDecoder`] can replay the same packet bytes
//! without a live device, while [`protocol`] is a pure decoder for sanitized
//! decrypted-command fixtures.

use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use crate::network::{GamePacket, GameSniffer};
use crate::{model::ObservationSnapshot, reference::ReferenceCache};
use base64::prelude::*;
use futures::{stream::FusedStream, StreamExt};
use inventory::InventoryDecoder;
use pktmon::{
    filter::{PktMonFilter, TransportProtocol},
    Capture, Packet,
};
use tokio::{
    sync::mpsc,
    task::{JoinError, JoinHandle},
};
use tokio_util::sync::CancellationToken;

use crate::{localization::LocalizedText, HsrError, HsrResult};

mod inventory;
pub mod proto;
pub mod protocol;

/// Decoder revision written into HSR exports.
pub const HSR_CAPTURE_REVISION: &str = "reliquary-shape-4.5-v1";

/// HSR's two UDP gateway ports. Kept public for build-time regression tests.
pub const HSR_PACKET_PORTS: [u16; 2] = [23301, 23302];

const CAPTURE_HINT: LocalizedText = LocalizedText::new(
    "无法读取 HSR 网络数据。请确认程序以管理员身份运行，然后重新抓包。",
    "HSR network data could not be read. Make sure the application is running as administrator, then capture again.",
);
const KEYS_HINT: LocalizedText = LocalizedText::new(
    "内置的 HSR 协议数据无法加载。请重新下载当前版本的 GOODCapture。",
    "The bundled HSR protocol data could not be loaded. Download the current GOODCapture build again.",
);
const WORKER_HINT: LocalizedText = LocalizedText::new(
    "HSR 抓包任务意外停止。请重新抓包；若问题再次出现，请复制完整错误。",
    "The HSR capture task stopped unexpectedly. Capture again; if it recurs, copy the full error.",
);

/// A successfully recognized complete achievement response.
///
/// The vector contains only public definition IDs whose raw status was 2 or
/// 3. It is sorted and deduplicated. An empty vector is still a present,
/// complete response and must not be confused with `None` from the decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedAchievementSnapshot {
    completed_ids: Vec<u32>,
}

impl DecodedAchievementSnapshot {
    pub(crate) fn new(completed_ids: Vec<u32>) -> Self {
        Self { completed_ids }
    }

    pub fn completed_ids(&self) -> &[u32] {
        &self.completed_ids
    }

    pub fn into_completed_ids(self) -> Vec<u32> {
        self.completed_ids
    }
}

/// Categories selected before starting a capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureTargets {
    pub characters: bool,
    pub light_cones: bool,
    pub relics: bool,
    pub achievements: bool,
}

impl Default for CaptureTargets {
    fn default() -> Self {
        Self {
            characters: true,
            light_cones: true,
            relics: true,
            achievements: true,
        }
    }
}

impl CaptureTargets {
    pub fn any(self) -> bool {
        self.characters || self.light_cones || self.relics || self.achievements
    }
}

/// Stateful Ethernet/IP/UDP/KCP/session-key decoder used by both native
/// capture and deterministic packet replay.
pub struct HsrPacketDecoder {
    known_ids: BTreeSet<u32>,
    dispatch_keys: HashMap<u32, Vec<u8>>,
    sniffer: GameSniffer,
    inventory: InventoryDecoder,
    targets: CaptureTargets,
    state: HsrCaptureState,
    conversation: Option<u32>,
}

impl HsrPacketDecoder {
    pub fn new(references: ReferenceCache, targets: CaptureTargets) -> HsrResult<Self> {
        if !targets.any() {
            return Err(HsrError::new(
                "HSR-CAPTURE-TARGETS",
                LocalizedText::new(
                    "请至少选择一项导出内容。",
                    "Select at least one category to export.",
                ),
                "capture requires at least one selected category",
            ));
        }
        let known_ids = protocol::collect_known_ids(references.achievement_ids())?;
        let dispatch_keys = load_embedded_dispatch_keys()?;
        let sniffer = GameSniffer::new().set_initial_keys(dispatch_keys.clone());
        Ok(Self {
            known_ids,
            dispatch_keys,
            sniffer,
            inventory: InventoryDecoder::new(references, targets)?,
            targets,
            state: HsrCaptureState::default(),
            conversation: None,
        })
    }

    pub fn state(&self) -> &HsrCaptureState {
        &self.state
    }

    /// Full packet replay and live capture use exactly the same transport.
    pub fn receive_packet(&mut self, packet: Vec<u8>) -> HsrResult<()> {
        self.state.packet_count += 1;
        let packets = match self.sniffer.receive_packet(packet) {
            Ok(packets) => packets,
            Err(error) => {
                self.state.last_transport_error = Some(error.to_string());
                return Ok(());
            },
        };
        for packet in packets {
            match packet {
                GamePacket::Commands {
                    conv_id,
                    result: Ok(command),
                } => {
                    if self
                        .conversation
                        .is_some_and(|selected| selected != conv_id)
                    {
                        continue;
                    }
                    self.state.command_count += 1;
                    self.receive_command(&command.proto_data)?;
                    if self.state.has_characters
                        || self.state.has_light_cones
                        || self.state.has_relics
                        || self.state.has_achievements
                    {
                        self.conversation = Some(conv_id);
                    }
                },
                GamePacket::Commands {
                    result: Err(error), ..
                } => {
                    self.state.last_transport_error = Some(error.to_string());
                },
                GamePacket::Connection(connection) => {
                    if matches!(connection, crate::network::ConnectionPacket::Disconnected) {
                        self.state.last_transport_error = Some("game connection closed".to_owned());
                    }
                },
            }
        }
        Ok(())
    }

    /// Decrypted-command replay; wrappers and command IDs are not assumed.
    pub fn receive_command(&mut self, bytes: &[u8]) -> HsrResult<()> {
        self.inventory.receive(bytes)?;
        if self.targets.achievements && !self.state.has_achievements {
            for container in protocol::containers(bytes) {
                if let Some(snapshot) =
                    protocol::decode_achievement_command(container, &self.known_ids)?
                {
                    self.state.completed_ids = snapshot.into_completed_ids();
                    self.state.achievement_count = self.state.completed_ids.len();
                    self.state.has_achievements = true;
                    break;
                }
            }
        }
        self.state.character_count = self.inventory.characters.as_ref().map_or(0, Vec::len);
        self.state.light_cone_count = self.inventory.light_cones.as_ref().map_or(0, Vec::len);
        self.state.relic_count = self.inventory.relics.as_ref().map_or(0, Vec::len);
        self.state.has_characters = self.inventory.characters.is_some();
        self.state.has_light_cones = self.inventory.light_cones.is_some();
        self.state.has_relics = self.inventory.relics.is_some();
        self.state.has_items =
            self.inventory.light_cones.is_some() && self.inventory.relics.is_some();
        self.state.complete = self.inventory.complete()
            && (!self.targets.achievements || self.state.has_achievements);
        if self.state.complete {
            self.state.inventory = self.inventory.snapshot();
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.sniffer = GameSniffer::new().set_initial_keys(self.dispatch_keys.clone());
        self.inventory.reset();
        self.conversation = None;
        self.state = HsrCaptureState::default();
    }
}

/// Commands sent by the application to the background capture monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HsrCaptureCommand {
    StartCapture,
    /// Terminal for this monitor instance. A later queued StartCapture is
    /// ignored so an early UI stop cannot race native-device initialization.
    StopCapture,
}

/// Privacy-safe state shared with the UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HsrCaptureState {
    pub capturing: bool,
    /// True only after all selected categories arrive.
    pub complete: bool,
    pub packet_count: usize,
    pub command_count: usize,
    pub last_transport_error: Option<String>,
    pub has_characters: bool,
    pub has_items: bool,
    pub has_light_cones: bool,
    pub has_relics: bool,
    pub has_achievements: bool,
    pub character_count: usize,
    pub light_cone_count: usize,
    pub relic_count: usize,
    pub inventory: Option<ObservationSnapshot>,
    pub achievement_count: usize,
    pub completed_ids: Vec<u32>,
    pub error: Option<HsrError>,
}

/// Cancelable capture monitor. Constructing it does not access a native
/// capture device; the device opens only after [`StartCapture`](HsrCaptureCommand::StartCapture).
pub struct HsrCaptureMonitor {
    decoder: HsrPacketDecoder,
    state: Arc<Mutex<HsrCaptureState>>,
    capture_cancel_token: Option<CancellationToken>,
    capture_task: Option<JoinHandle<HsrResult<()>>>,
    terminal_stop_requested: bool,
    started: Option<tokio::time::Instant>,
    packet_tx: mpsc::UnboundedSender<Vec<u8>>,
    packet_rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl HsrCaptureMonitor {
    pub fn new(
        state: Arc<Mutex<HsrCaptureState>>,
        references: ReferenceCache,
        targets: CaptureTargets,
    ) -> HsrResult<Self> {
        let decoder = HsrPacketDecoder::new(references, targets)?;
        let (packet_tx, packet_rx) = mpsc::unbounded_channel();
        Ok(Self {
            decoder,
            state,
            capture_cancel_token: None,
            capture_task: None,
            terminal_stop_requested: false,
            started: None,
            packet_tx,
            packet_rx,
        })
    }

    pub async fn run(mut self, mut command_rx: mpsc::UnboundedReceiver<HsrCaptureCommand>) {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = timer.tick() => self.check_timeout(),
                Some(packet) = self.packet_rx.recv() => self.handle_packet(packet),
                command = command_rx.recv() => match command {
                    Some(command) => self.handle_command(command),
                    None => break,
                },
                result = wait_for_capture_task(&mut self.capture_task) => {
                    self.handle_capture_task_result(result);
                },
                else => break,
            }
        }

        self.stop_capture();
        if let Some(task) = self.capture_task.take() {
            let _ = task.await;
        }
    }

    fn handle_command(&mut self, command: HsrCaptureCommand) {
        match command {
            HsrCaptureCommand::StartCapture if !self.terminal_stop_requested => {
                self.start_capture();
            },
            HsrCaptureCommand::StartCapture => {},
            HsrCaptureCommand::StopCapture => {
                self.terminal_stop_requested = true;
                self.stop_capture();
            },
        }
    }

    fn start_capture(&mut self) {
        if self.capture_task.is_some() {
            return;
        }

        self.decoder.reset();
        self.started = Some(tokio::time::Instant::now());
        if let Ok(mut state) = self.state.lock() {
            *state = HsrCaptureState {
                capturing: true,
                ..HsrCaptureState::default()
            };
        }

        let cancel_token = CancellationToken::new();
        self.capture_task = Some(tokio::spawn(capture_task(
            cancel_token.clone(),
            self.packet_tx.clone(),
        )));
        self.capture_cancel_token = Some(cancel_token);
    }

    fn stop_capture(&mut self) {
        self.started = None;
        if let Some(token) = &self.capture_cancel_token {
            token.cancel();
        }
        if let Ok(mut state) = self.state.lock() {
            state.capturing = false;
        }
    }

    fn handle_packet(&mut self, packet: Vec<u8>) {
        if self.started.is_none() {
            return;
        }
        match self.decoder.receive_packet(packet) {
            Ok(()) => {
                let mut snapshot = self.decoder.state().clone();
                snapshot.capturing = !snapshot.complete;
                if snapshot.complete {
                    self.stop_capture();
                }
                if let Ok(mut state) = self.state.lock() {
                    *state = snapshot;
                }
            },
            Err(error) => self.fail(error),
        }
    }

    fn check_timeout(&mut self) {
        if self
            .started
            .is_some_and(|start| start.elapsed().as_secs() >= 120)
        {
            let state = self.decoder.state();
            let detail = format!("capture timed out after 120 seconds; packets={}; decryptedCommands={}; characters={}; lightCones={}; relics={}; achievements={}; lastTransportError={}",
                state.packet_count, state.command_count, state.character_count, state.light_cone_count,
                state.relic_count, state.achievement_count, state.last_transport_error.as_deref().unwrap_or("none"));
            let hint = if state.packet_count == 0 {
                LocalizedText::new("未收到星穹铁道流量。请确认游戏正在本机运行，重新抓包后再登录。",
                    "No Star Rail traffic was received. Check that the game is running on this PC, then start capture before logging in.")
            } else if state.command_count == 0 {
                LocalizedText::new("已收到网络流量，但未能解密登录数据。请更新 GOODCapture，并在登录前开始抓包。",
                    "Traffic was received, but login data could not be decrypted. Update GOODCapture and start capture before logging in.")
            } else {
                LocalizedText::new("已读取游戏流量，但数据尚不完整。请重新登录抓包；若仍失败，请复制完整错误。",
                    "Game traffic was decoded, but some data is missing. Capture a new login; if it fails again, copy the full error.")
            };
            self.fail(HsrError::new("HSR-CAPTURE-TIMEOUT", hint, detail));
        }
    }

    fn handle_capture_task_result(&mut self, result: Result<HsrResult<()>, JoinError>) {
        let expected_stop = self
            .capture_cancel_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled);
        self.capture_task = None;
        self.capture_cancel_token = None;

        match result {
            Ok(Ok(())) if expected_stop => {},
            Ok(Ok(())) => self.fail(HsrError::new(
                "HSR-CAPTURE-CLOSED",
                WORKER_HINT,
                "pktmon capture stream ended before all requested data was decoded",
            )),
            Ok(Err(_)) if expected_stop => {},
            Ok(Err(error)) => self.fail(error),
            Err(error) if expected_stop && error.is_cancelled() => {},
            Err(error) => self.fail(HsrError::new(
                "HSR-CAPTURE-WORKER",
                WORKER_HINT,
                format!("capture task join failed; cause={error}"),
            )),
        }
    }

    fn fail(&mut self, error: HsrError) {
        self.stop_capture();
        if let Ok(mut state) = self.state.lock() {
            state.complete = false;
            state.error = Some(error);
        }
    }
}

impl Drop for HsrCaptureMonitor {
    fn drop(&mut self) {
        if let Some(token) = &self.capture_cancel_token {
            token.cancel();
        }
    }
}

async fn wait_for_capture_task(
    task: &mut Option<JoinHandle<HsrResult<()>>>,
) -> Result<HsrResult<()>, JoinError> {
    match task {
        Some(task) => task.await,
        None => std::future::pending().await,
    }
}

async fn capture_task(
    cancel_token: CancellationToken,
    packet_tx: mpsc::UnboundedSender<Vec<u8>>,
) -> HsrResult<()> {
    let mut source = NativePacketSource::new()?;
    pump_packets(&mut source, &cancel_token, &packet_tx).await
}

trait PacketSource: Send {
    fn next_packet(&mut self) -> Pin<Box<dyn Future<Output = HsrResult<Vec<u8>>> + Send + '_>>;
}

async fn pump_packets<S: PacketSource>(
    source: &mut S,
    cancel_token: &CancellationToken,
    packet_tx: &mpsc::UnboundedSender<Vec<u8>>,
) -> HsrResult<()> {
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => return Ok(()),
            packet = source.next_packet() => {
                let packet = packet?;
                if packet_tx.send(packet).is_err() {
                    return Ok(());
                }
            },
        }
    }
}

struct NativePacketSource {
    stream: Box<dyn FusedStream<Item = Packet> + Unpin + Send>,
}

impl NativePacketSource {
    fn new() -> HsrResult<Self> {
        #[cfg(target_os = "windows")]
        yas::utils::ensure_admin().map_err(|error| {
            HsrError::new(
                "HSR-CAPTURE-ADMIN",
                CAPTURE_HINT,
                format!("administrator check failed; cause={error:#}"),
            )
        })?;

        let mut capture = Capture::new().map_err(|error| {
            HsrError::new(
                "HSR-CAPTURE-OPEN",
                CAPTURE_HINT,
                format!("pktmon capture could not be created; cause={error}"),
            )
        })?;
        for (index, port) in HSR_PACKET_PORTS.into_iter().enumerate() {
            capture
                .add_filter(PktMonFilter {
                    name: format!("HSR UDP {index}"),
                    transport_protocol: Some(TransportProtocol::UDP),
                    port: port.into(),
                    ..PktMonFilter::default()
                })
                .map_err(|error| {
                    HsrError::new(
                        "HSR-CAPTURE-FILTER",
                        CAPTURE_HINT,
                        format!("pktmon UDP port {port} filter failed; cause={error}"),
                    )
                })?;
        }

        let stream = capture.stream().map_err(|error| {
            HsrError::new(
                "HSR-CAPTURE-STREAM",
                CAPTURE_HINT,
                format!("pktmon packet stream could not be started; cause={error}"),
            )
        })?;
        Ok(Self {
            stream: Box::new(stream.boxed().fuse()),
        })
    }
}

impl PacketSource for NativePacketSource {
    fn next_packet(&mut self) -> Pin<Box<dyn Future<Output = HsrResult<Vec<u8>>> + Send + '_>> {
        Box::pin(async move {
            match self.stream.next().await {
                Some(packet) => Ok(packet.payload.to_vec().clone()),
                None => Err(HsrError::new(
                    "HSR-CAPTURE-CLOSED",
                    WORKER_HINT,
                    "pktmon packet stream closed",
                )),
            }
        })
    }
}

fn load_embedded_dispatch_keys() -> HsrResult<HashMap<u32, Vec<u8>>> {
    let encoded: HashMap<u32, String> =
        serde_json::from_slice(include_bytes!("../../keys/hsr.json")).map_err(|error| {
            HsrError::new(
                "HSR-KEYS-JSON",
                KEYS_HINT,
                format!("embedded keys/hsr.json is invalid; cause={error}"),
            )
        })?;
    if encoded.is_empty() {
        return Err(HsrError::new(
            "HSR-KEYS-EMPTY",
            KEYS_HINT,
            "embedded keys/hsr.json contains no dispatch keys",
        ));
    }

    encoded
        .into_iter()
        .map(|(version, encoded)| {
            BASE64_STANDARD
                .decode(encoded)
                .map(|key| (version, key))
                .map_err(|error| {
                    HsrError::new(
                        "HSR-KEYS-BASE64",
                        KEYS_HINT,
                        format!("dispatch key {version} is invalid base64; cause={error}"),
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PendingPacketSource;

    impl PacketSource for PendingPacketSource {
        fn next_packet(&mut self) -> Pin<Box<dyn Future<Output = HsrResult<Vec<u8>>> + Send + '_>> {
            Box::pin(std::future::pending())
        }
    }

    #[tokio::test]
    async fn packet_pump_honors_cancellation_while_source_is_pending() {
        let mut source = PendingPacketSource;
        let token = CancellationToken::new();
        token.cancel();
        let (packet_tx, mut packet_rx) = mpsc::unbounded_channel();

        pump_packets(&mut source, &token, &packet_tx)
            .await
            .expect("cancellation should be a clean stop");
        assert!(packet_rx.try_recv().is_err());
    }

    #[test]
    fn monitor_stop_cancels_the_owned_token_and_is_not_complete() {
        let state = Arc::new(Mutex::new(HsrCaptureState {
            capturing: true,
            ..HsrCaptureState::default()
        }));
        let mut monitor = HsrCaptureMonitor::new(
            state.clone(),
            crate::load_embedded_gilore_reference().unwrap(),
            CaptureTargets::default(),
        )
        .unwrap();
        let token = CancellationToken::new();
        monitor.capture_cancel_token = Some(token.clone());

        monitor.stop_capture();

        assert!(token.is_cancelled());
        let state = state.lock().unwrap();
        assert!(!state.capturing);
        assert!(!state.complete);
    }

    #[test]
    fn stop_before_start_is_terminal_and_never_spawns_native_capture() {
        let state = Arc::new(Mutex::new(HsrCaptureState::default()));
        let mut monitor = HsrCaptureMonitor::new(
            state.clone(),
            crate::load_embedded_gilore_reference().unwrap(),
            CaptureTargets::default(),
        )
        .unwrap();

        monitor.handle_command(HsrCaptureCommand::StopCapture);
        monitor.handle_command(HsrCaptureCommand::StartCapture);

        assert!(monitor.capture_task.is_none());
        assert!(monitor.capture_cancel_token.is_none());
        let state = state.lock().unwrap();
        assert!(!state.capturing);
        assert!(!state.complete);
    }

    #[test]
    fn embedded_dispatch_keys_are_pinned_and_decodable() {
        let keys = load_embedded_dispatch_keys().unwrap();
        assert_eq!(keys.len(), 30);
        assert!(keys.values().all(|key| key.len() >= 4_096));
        assert_eq!(HSR_PACKET_PORTS, [23301, 23302]);
    }

    #[test]
    fn timeout_reports_transport_progress_and_stops_without_claiming_success() {
        let state = Arc::new(Mutex::new(HsrCaptureState::default()));
        let mut monitor = HsrCaptureMonitor::new(
            state.clone(),
            crate::load_embedded_gilore_reference().unwrap(),
            CaptureTargets::default(),
        )
        .unwrap();
        monitor.decoder.state.packet_count = 12;
        monitor.decoder.state.last_transport_error =
            Some("decryption key is missing for command".to_owned());
        monitor.started = Some(tokio::time::Instant::now() - std::time::Duration::from_secs(121));
        monitor.check_timeout();
        let state = state.lock().unwrap();
        assert!(!state.capturing);
        assert!(!state.complete);
        let error = state.error.as_ref().unwrap();
        assert_eq!(error.code(), "HSR-CAPTURE-TIMEOUT");
        let message = error.localized_message(crate::Language::En);
        assert!(message.contains("packets=12"));
        assert!(message.contains("decryptedCommands=0"));
    }
}
