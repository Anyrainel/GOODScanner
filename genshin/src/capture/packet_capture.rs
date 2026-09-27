/// UDP packet capture for Genshin Impact traffic.
///
/// Ported from irminsul's `capture.rs`. Filters UDP on ports 22101–22102.
///
/// The backend is Windows-only (pktmon). Other platforms compile the same
/// public API but `PacketCapture::new` fails with a clear error so callers
/// and the monitor logic stay portable.
use std::fmt::{Debug, Display};

use anyhow::Error;
#[cfg(target_os = "windows")]
use futures::stream::FusedStream;
#[cfg(target_os = "windows")]
use futures::StreamExt;
#[cfg(target_os = "windows")]
use pktmon::filter::{PktMonFilter, TransportProtocol};
#[cfg(target_os = "windows")]
use pktmon::{Capture, Packet};

pub const PORT_RANGE: (u16, u16) = (22101, 22102);

#[derive(Debug)]
#[allow(dead_code)]
pub enum CaptureError {
    Filter(Error),
    Capture { has_captured: bool, error: Error },
    CaptureClosed,
}

impl Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Filter(e) => write!(f, "Filter error: {}", e),
            CaptureError::Capture {
                has_captured,
                error,
            } => write!(
                f,
                "Capture error (has_captured = {}): {}",
                has_captured, error
            ),
            CaptureError::CaptureClosed => write!(f, "Capture closed"),
        }
    }
}

pub type Result<T> = std::result::Result<T, CaptureError>;

#[cfg(target_os = "windows")]
pub struct PacketCapture {
    stream: Box<dyn FusedStream<Item = Packet> + Unpin + Send>,
}

#[cfg(target_os = "windows")]
impl PacketCapture {
    pub fn new() -> Result<Self> {
        let mut capture = Capture::new().map_err(|e| CaptureError::Capture {
            has_captured: false,
            error: e.into(),
        })?;

        let filter1 = PktMonFilter {
            name: "UDP Filter".to_string(),
            transport_protocol: Some(TransportProtocol::UDP),
            port: PORT_RANGE.0.into(),
            ..PktMonFilter::default()
        };
        capture
            .add_filter(filter1)
            .map_err(|e| CaptureError::Filter(e.into()))?;

        let filter2 = PktMonFilter {
            name: "UDP Filter".to_string(),
            transport_protocol: Some(TransportProtocol::UDP),
            port: PORT_RANGE.1.into(),
            ..PktMonFilter::default()
        };
        capture
            .add_filter(filter2)
            .map_err(|e| CaptureError::Filter(e.into()))?;

        let stream = capture.stream().map_err(|error| CaptureError::Capture {
            has_captured: false,
            error: Error::from(error).context("pktmon packet stream could not be started"),
        })?;

        Ok(Self {
            stream: Box::new(stream.boxed().fuse()),
        })
    }

    pub async fn next_packet(&mut self) -> Result<Vec<u8>> {
        futures::select! {
            packet = self.stream.select_next_some() => {
                Ok(packet.payload.to_vec().clone())
            },
            complete => Err(CaptureError::CaptureClosed),
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub struct PacketCapture {
    _platform: (),
}

#[cfg(not(target_os = "windows"))]
impl PacketCapture {
    pub fn new() -> Result<Self> {
        Err(CaptureError::Capture {
            has_captured: false,
            error: Error::msg(
                "抓包功能目前仅支持 Windows（pktmon），Linux 抓包后端尚未实现。\n\
                 Packet capture is currently Windows-only (pktmon); a Linux backend is not implemented yet.",
            ),
        })
    }

    pub async fn next_packet(&mut self) -> Result<Vec<u8>> {
        Err(CaptureError::CaptureClosed)
    }
}
