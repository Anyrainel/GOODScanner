//! In-process, read-only capture of completed Honkai: Star Rail achievements.
//!
//! The native boundary captures only HSR UDP traffic on the two established
//! game ports. [`AchievementPacketDecoder`] can replay the same packet bytes
//! without a live device, while [`protocol`] is a pure decoder for sanitized
//! decrypted-command fixtures.

use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use auto_reliquary::{GamePacket, GameSniffer};
use base64::prelude::*;
use futures::{stream::FusedStream, StreamExt};
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

pub mod protocol;

/// Stable privacy-safe source revision written into HSR achievement exports.
pub const ACHIEVEMENT_CAPTURE_REVISION: &str = "auto-reliquary-1.2.0";

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
    "HSR 成就抓包任务意外停止。请重新抓包；若问题再次出现，请复制完整错误。",
    "The HSR achievement capture task stopped unexpectedly. Capture again; if it recurs, copy the full error.",
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

/// Stateful Ethernet/IP/UDP/KCP/session-key decoder used by both native
/// capture and deterministic packet replay.
pub struct AchievementPacketDecoder {
    known_ids: BTreeSet<u32>,
    dispatch_keys: HashMap<u32, Vec<u8>>,
    sniffer: GameSniffer,
}

impl AchievementPacketDecoder {
    pub fn new<I>(known_achievement_ids: I) -> HsrResult<Self>
    where
        I: IntoIterator<Item = u32>,
    {
        let known_ids = protocol::collect_known_ids(known_achievement_ids)?;
        let dispatch_keys = load_embedded_dispatch_keys()?;
        let sniffer = GameSniffer::new().set_initial_keys(dispatch_keys.clone());
        Ok(Self {
            known_ids,
            dispatch_keys,
            sniffer,
        })
    }

    /// Feed one full packet frame. Packet and command IDs are never retained.
    pub fn receive_packet(
        &mut self,
        packet: Vec<u8>,
    ) -> HsrResult<Option<DecodedAchievementSnapshot>> {
        let Some(GamePacket::Commands(commands)) = self.sniffer.receive_packet(packet) else {
            return Ok(None);
        };

        for command in commands {
            if let Some(snapshot) =
                protocol::decode_achievement_command(&command.proto_data, &self.known_ids)?
            {
                return Ok(Some(snapshot));
            }
        }
        Ok(None)
    }

    fn reset(&mut self) {
        self.sniffer = GameSniffer::new().set_initial_keys(self.dispatch_keys.clone());
    }
}

/// Commands sent by the application to the background capture monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AchievementCaptureCommand {
    StartCapture,
    /// Terminal for this monitor instance. A later queued StartCapture is
    /// ignored so an early UI stop cannot race native-device initialization.
    StopCapture,
}

/// Privacy-safe state shared with the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AchievementCaptureState {
    pub capturing: bool,
    /// True only after a structurally valid complete achievement response.
    /// This remains true when `achievement_count == 0`.
    pub complete: bool,
    pub achievement_count: usize,
    pub completed_ids: Vec<u32>,
    pub error: Option<HsrError>,
}

/// Cancelable capture monitor. Constructing it does not access a native
/// capture device; the device opens only after [`StartCapture`](AchievementCaptureCommand::StartCapture).
pub struct AchievementCaptureMonitor {
    decoder: AchievementPacketDecoder,
    state: Arc<Mutex<AchievementCaptureState>>,
    capture_cancel_token: Option<CancellationToken>,
    capture_task: Option<JoinHandle<HsrResult<()>>>,
    terminal_stop_requested: bool,
    packet_tx: mpsc::UnboundedSender<Vec<u8>>,
    packet_rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl AchievementCaptureMonitor {
    pub fn new<I>(
        state: Arc<Mutex<AchievementCaptureState>>,
        known_achievement_ids: I,
    ) -> HsrResult<Self>
    where
        I: IntoIterator<Item = u32>,
    {
        let decoder = AchievementPacketDecoder::new(known_achievement_ids)?;
        let (packet_tx, packet_rx) = mpsc::unbounded_channel();
        Ok(Self {
            decoder,
            state,
            capture_cancel_token: None,
            capture_task: None,
            terminal_stop_requested: false,
            packet_tx,
            packet_rx,
        })
    }

    pub async fn run(mut self, mut command_rx: mpsc::UnboundedReceiver<AchievementCaptureCommand>) {
        loop {
            tokio::select! {
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

    fn handle_command(&mut self, command: AchievementCaptureCommand) {
        match command {
            AchievementCaptureCommand::StartCapture if !self.terminal_stop_requested => {
                self.start_capture();
            },
            AchievementCaptureCommand::StartCapture => {},
            AchievementCaptureCommand::StopCapture => {
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
        if let Ok(mut state) = self.state.lock() {
            *state = AchievementCaptureState {
                capturing: true,
                ..AchievementCaptureState::default()
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
        if let Some(token) = &self.capture_cancel_token {
            token.cancel();
        }
        if let Ok(mut state) = self.state.lock() {
            state.capturing = false;
        }
    }

    fn handle_packet(&mut self, packet: Vec<u8>) {
        match self.decoder.receive_packet(packet) {
            Ok(Some(snapshot)) => {
                self.stop_capture();
                if let Ok(mut state) = self.state.lock() {
                    state.achievement_count = snapshot.completed_ids().len();
                    state.completed_ids = snapshot.into_completed_ids();
                    state.complete = true;
                    state.error = None;
                }
            },
            Ok(None) => {},
            Err(error) => self.fail(error),
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
                "HSR-ACHIEVEMENT-CAPTURE-CLOSED",
                WORKER_HINT,
                "pktmon capture stream ended before an achievement response was decoded",
            )),
            Ok(Err(_)) if expected_stop => {},
            Ok(Err(error)) => self.fail(error),
            Err(error) if expected_stop && error.is_cancelled() => {},
            Err(error) => self.fail(HsrError::new(
                "HSR-ACHIEVEMENT-CAPTURE-WORKER",
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

impl Drop for AchievementCaptureMonitor {
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
                "HSR-ACHIEVEMENT-CAPTURE-ADMIN",
                CAPTURE_HINT,
                format!("administrator check failed; cause={error:#}"),
            )
        })?;

        let mut capture = Capture::new().map_err(|error| {
            HsrError::new(
                "HSR-ACHIEVEMENT-CAPTURE-OPEN",
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
                        "HSR-ACHIEVEMENT-CAPTURE-FILTER",
                        CAPTURE_HINT,
                        format!("pktmon UDP port {port} filter failed; cause={error}"),
                    )
                })?;
        }

        let stream = capture.stream().map_err(|error| {
            HsrError::new(
                "HSR-ACHIEVEMENT-CAPTURE-STREAM",
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
                    "HSR-ACHIEVEMENT-CAPTURE-CLOSED",
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
                "HSR-ACHIEVEMENT-KEYS-JSON",
                KEYS_HINT,
                format!("embedded keys/hsr.json is invalid; cause={error}"),
            )
        })?;
    if encoded.is_empty() {
        return Err(HsrError::new(
            "HSR-ACHIEVEMENT-KEYS-EMPTY",
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
                        "HSR-ACHIEVEMENT-KEYS-BASE64",
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
        let state = Arc::new(Mutex::new(AchievementCaptureState {
            capturing: true,
            ..AchievementCaptureState::default()
        }));
        let mut monitor = AchievementCaptureMonitor::new(
            state.clone(),
            [4_010_101, 4_010_102, 4_010_103, 4_010_104, 4_010_105],
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
        let state = Arc::new(Mutex::new(AchievementCaptureState::default()));
        let mut monitor = AchievementCaptureMonitor::new(
            state.clone(),
            [4_010_101, 4_010_102, 4_010_103, 4_010_104, 4_010_105],
        )
        .unwrap();

        monitor.handle_command(AchievementCaptureCommand::StopCapture);
        monitor.handle_command(AchievementCaptureCommand::StartCapture);

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
}
