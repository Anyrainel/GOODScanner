use etherparse::{SlicedPacket, TransportSlice, UdpHeader};
use tracing::{debug, info, instrument, trace, warn};

use crate::{ConnectionPacket, PacketDirection};

#[instrument(skip_all)]
pub fn parse_connection_packet(port_filter: &[u16], bytes: Vec<u8>) -> Option<ConnectionPacket> {
    let (udp, payload) = parse_udp(bytes)?;
    let direction = validate_ports(port_filter, udp)?;

    if payload.len() <= 20 {
        let code = u32::from_be_bytes(payload[..4].try_into().unwrap());
        match code {
            0xFF => {
                info!("handshake requested");
                Some(ConnectionPacket::HandshakeRequested)
            },
            404 => {
                warn!("disconnected packet");
                Some(ConnectionPacket::Disconnected)
            },
            _ => {
                trace!("handshake established");
                Some(ConnectionPacket::HandshakeEstablished)
            },
        }
    } else {
        Some(ConnectionPacket::SegmentData(direction, payload))
    }
}

#[instrument(skip_all, fields(len = data.len()))]
pub fn parse_udp(data: Vec<u8>) -> Option<(UdpHeader, Vec<u8>)> {
    let packet = match SlicedPacket::from_ethernet(&data) {
        Ok(p) => p,
        Err(e) => {
            debug!("failed: {e}");
            return None;
        },
    };

    // sanity checking the pcap filters
    let Some(transport) = packet.transport else {
        debug!("transport was not present");
        return None;
    };

    let TransportSlice::Udp(udp) = transport else {
        debug!("packet was not udp");
        return None;
    };

    trace!("complete");

    Some((udp.to_header(), udp.payload().to_vec()))
}

fn validate_ports(port_filter: &[u16], udp: UdpHeader) -> Option<PacketDirection> {
    let (src, dest) = (udp.source_port, udp.destination_port);
    if port_filter.contains(&src) {
        Some(PacketDirection::Received)
    } else if port_filter.contains(&dest) {
        Some(PacketDirection::Sent)
    } else {
        trace!(src, dest, "incorrect ports");
        return None;
    }
}
