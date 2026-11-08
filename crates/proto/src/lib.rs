//! Shared constants and the UDP probe header.
use serde::{Deserialize, Serialize};

pub const CONTROL_PORT: u16 = 7000;
pub const UDP_PORT: u16 = 7001;
pub const TCP_DATA_PORT: u16 = 7002;

/// Header size in bytes: session(4) seq(4) send_ts_ns(8) size(2) flags(2).
pub const HEADER_LEN: usize = 20;
pub const FLAG_ECHO: u16 = 1; // server echoes the packet back
pub const FLAG_DATA: u16 = 2; // client -> server load packet, server counts it
pub const FLAG_REPORT_REQ: u16 = 4; // client asks for the counters of a session
pub const FLAG_REPORT: u16 = 8; // server reply, Report follows the header
pub const FLAG_SRC_REQ: u16 = 16; // client asks the server to send a paced stream

/// Safety limits for a server-sent stream.
pub const MAX_SRC_MBPS: f64 = 2000.0;
pub const MAX_SRC_SECS: f64 = 30.0;
pub const MAX_UDP_PAYLOAD: usize = 65507;

/// Counters a receiver keeps for one session.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct Report {
    pub received: u32,
    pub reordered: u32,
    pub bytes: u64,
    pub jitter_ms: f64,
    pub duration_ms: f64,
}

impl Report {
    pub const LEN: usize = 32;
    pub fn write(&self, buf: &mut [u8]) {
        buf[0..4].copy_from_slice(&self.received.to_be_bytes());
        buf[4..8].copy_from_slice(&self.reordered.to_be_bytes());
        buf[8..16].copy_from_slice(&self.bytes.to_be_bytes());
        buf[16..24].copy_from_slice(&self.jitter_ms.to_bits().to_be_bytes());
        buf[24..32].copy_from_slice(&self.duration_ms.to_bits().to_be_bytes());
    }
    pub fn read(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::LEN {
            return None;
        }
        Some(Self {
            received: u32::from_be_bytes(buf[0..4].try_into().ok()?),
            reordered: u32::from_be_bytes(buf[4..8].try_into().ok()?),
            bytes: u64::from_be_bytes(buf[8..16].try_into().ok()?),
            jitter_ms: f64::from_bits(u64::from_be_bytes(buf[16..24].try_into().ok()?)),
            duration_ms: f64::from_bits(u64::from_be_bytes(buf[24..32].try_into().ok()?)),
        })
    }
}

/// Counts packets, reordering and RFC 3550 jitter for one stream.
/// Jitter uses only time differences, so the two clocks need no sync.
#[derive(Default, Debug)]
pub struct RecvTracker {
    received: u32,
    reordered: u32,
    bytes: u64,
    max_seq: Option<u32>,
    jitter_ns: f64,
    prev: Option<(u64, u64)>, // (send_ts, arrival)
    first_arrival: Option<u64>,
    last_arrival: u64,
}

impl RecvTracker {
    pub fn on_packet(&mut self, seq: u32, send_ts_ns: u64, arrival_ns: u64, len: usize) {
        self.received += 1;
        self.bytes += len as u64;
        match self.max_seq {
            Some(m) if seq < m => self.reordered += 1,
            _ => self.max_seq = Some(seq),
        }
        if let Some((ps, pa)) = self.prev {
            let d = (arrival_ns as i128 - pa as i128) - (send_ts_ns as i128 - ps as i128);
            self.jitter_ns += ((d.abs() as f64) - self.jitter_ns) / 16.0;
        }
        self.prev = Some((send_ts_ns, arrival_ns));
        self.first_arrival.get_or_insert(arrival_ns);
        self.last_arrival = arrival_ns;
    }
    pub fn report(&self) -> Report {
        Report {
            received: self.received,
            reordered: self.reordered,
            bytes: self.bytes,
            jitter_ms: self.jitter_ns / 1e6,
            duration_ms: self.last_arrival.saturating_sub(self.first_arrival.unwrap_or(0)) as f64 / 1e6,
        }
    }
}

/// First byte a client sends on the TCP data port.
pub const MODE_ECHO: u8 = b'E';
pub const MODE_SINK: u8 = b'U'; // client uploads, server discards
pub const MODE_SOURCE: u8 = b'D'; // server sends, client downloads

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Header {
    pub session: u32,
    pub seq: u32,
    pub send_ts_ns: u64,
    pub size: u16,
    pub flags: u16,
}

impl Header {
    pub fn write(&self, buf: &mut [u8]) {
        buf[0..4].copy_from_slice(&self.session.to_be_bytes());
        buf[4..8].copy_from_slice(&self.seq.to_be_bytes());
        buf[8..16].copy_from_slice(&self.send_ts_ns.to_be_bytes());
        buf[16..18].copy_from_slice(&self.size.to_be_bytes());
        buf[18..20].copy_from_slice(&self.flags.to_be_bytes());
    }
    pub fn read(buf: &[u8]) -> Option<Self> {
        if buf.len() < HEADER_LEN {
            return None;
        }
        Some(Self {
            session: u32::from_be_bytes(buf[0..4].try_into().ok()?),
            seq: u32::from_be_bytes(buf[4..8].try_into().ok()?),
            send_ts_ns: u64::from_be_bytes(buf[8..16].try_into().ok()?),
            size: u16::from_be_bytes(buf[16..18].try_into().ok()?),
            flags: u16::from_be_bytes(buf[18..20].try_into().ok()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_roundtrip() {
        let r = Report { received: 5, reordered: 1, bytes: 900, jitter_ms: 0.25, duration_ms: 12.5 };
        let mut b = [0u8; Report::LEN];
        r.write(&mut b);
        assert_eq!(Report::read(&b), Some(r));
    }
    #[test]
    fn tracker_counts_reorder_and_steady_jitter() {
        let mut t = RecvTracker::default();
        for i in 0..10u32 {
            t.on_packet(i, i as u64 * 1_000_000, 5_000_000 + i as u64 * 1_000_000, 100);
        }
        t.on_packet(3, 0, 20_000_000, 100); // late old packet
        let r = t.report();
        assert_eq!(r.received, 11);
        assert_eq!(r.reordered, 1);
        assert!(r.jitter_ms > 0.0);
        let mut s = RecvTracker::default();
        for i in 0..50u32 {
            s.on_packet(i, i as u64 * 1_000_000, 9 + i as u64 * 1_000_000, 100);
        }
        assert_eq!(s.report().jitter_ms, 0.0);
    }
    #[test]
    fn header_roundtrip() {
        let h = Header { session: 1, seq: 2, send_ts_ns: 3, size: 64, flags: FLAG_ECHO };
        let mut b = [0u8; HEADER_LEN];
        h.write(&mut b);
        assert_eq!(Header::read(&b), Some(h));
    }
}
