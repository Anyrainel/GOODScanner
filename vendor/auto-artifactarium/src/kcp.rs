use std::time::SystemTime;

use kcp::{KCP_OVERHEAD, Kcp, get_conv};
use tracing::{Level, error, info, instrument, span, trace, warn};

use crate::bytes_as_hex;

pub(crate) struct KcpSniffer {
    conv_id: u32,
    kcp: Kcp<Vec<u8>>,
    time_start: SystemTime,
}

impl KcpSniffer {
    #[instrument(skip(segment))]
    pub fn try_new(segment: &[u8]) -> Option<Self> {
        validate_kcp_segment(segment).map(Self::new).or_else(|| {
            error!("could not create new kcp instance");
            None
        })
    }

    #[instrument]
    fn new(conv_id: u32) -> Self {
        info!("new connection, created new kcp instance");

        KcpSniffer {
            conv_id,
            kcp: new_kcp(conv_id),
            time_start: SystemTime::now(),
        }
    }

    #[instrument(skip_all, fields(conv_id = self.conv_id, len = segments.len()))]
    pub fn receive_segments(&mut self, segments: &[u8]) -> Vec<Vec<u8>> {
        let Some(conv_id) = validate_kcp_segment(segments) else {
            return Vec::new();
        };

        trace!("message data: {}", bytes_as_hex(segments));

        if conv_id != self.conv_id {
            warn!(
                expected = self.conv_id,
                "packet did not belong to conversation"
            );
            return Vec::new();
        }

        // game uses special format which adds 4 bytes at index 4,
        // reprocess to discard bytes 4..8 of every segment
        let segments = reformat_kcp_segments(segments);

        match self.kcp.input(&segments) {
            Ok(size) => trace!(size, "input successful"),
            Err(e) => warn!("could not input to kcp: {e}"),
        }

        let mut recv = Vec::new();
        while let Ok(size) = self.kcp.peeksize() {
            let span = span!(Level::TRACE, "receiving", size);
            let _enter = span.enter();

            let mut bytes = vec![0; size];

            match self.kcp.recv(&mut bytes) {
                Ok(_size) => {
                    recv.push(bytes);
                },
                Err(e) => {
                    warn!(%e, "could not receive kcp bytes");
                },
            }
        }

        if let Err(e) = self.kcp.update(self.clock()) {
            warn!(%e, "could not update kcp state");
        }

        recv
    }

    #[inline]
    fn clock(&self) -> u32 {
        SystemTime::now()
            .duration_since(self.time_start)
            .expect("time went backwards")
            .as_millis() as u32
    }
}

#[inline]
fn new_kcp(conv_id: u32) -> Kcp<Vec<u8>> {
    let mut kcp = Kcp::new(conv_id, Vec::new());
    kcp.set_wndsize(1024, 1024);
    kcp
}

fn validate_kcp_segment(payload: &[u8]) -> Option<u32> {
    if payload.len() <= KCP_OVERHEAD {
        warn!(
            len = payload.len(),
            data = bytes_as_hex(payload),
            "kcp header was too short"
        );
        return None;
    }
    Some(get_conv(payload))
}

// reformat to skip bytes 4..8
fn reformat_kcp_segments(data: &[u8]) -> Vec<u8> {
    let span = span!(Level::TRACE, "split");
    let _enter = span.enter();

    let mut reformatted_bytes = Vec::new();

    trace!("before split: {}", bytes_as_hex(data));

    let mut i = 0;
    while i < data.len() {
        let conv_id = &data[i..i + 4];

        let _ = &data[i + 4..i + 8]; // skip

        let remaining_header = &data[i + 8..i + 28];
        let content_len = u32::from_le_bytes(data[i + 24..i + 28].try_into().unwrap()) as usize;
        let content = &data[i + 28 + 4..i + 28 + 4 + content_len];

        for b in conv_id.iter().chain(remaining_header).chain(content) {
            reformatted_bytes.push(*b);
        }

        i += 28 + 4 + content_len;
    }

    trace!(" after split: {}", bytes_as_hex(&reformatted_bytes));

    reformatted_bytes
}
