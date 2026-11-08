//! UDP load cells and the size x rate sweep.
use proto::*;
use std::io;
use std::net::{ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static NEXT_SESSION: AtomicU32 = AtomicU32::new(1000);

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub enum UdpDir {
    Upload,
    Download,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CellResult {
    pub dir: UdpDir,
    pub size: u16,
    pub target_mbps: f64,
    pub sent: u32,
    pub received: u32,
    pub loss_pct: f64,
    pub reordered: u32,
    pub jitter_ms: f64,
    /// Goodput the receiver saw (UDP payload bits per second).
    pub achieved_mbps: f64,
    /// Rate the sender really reached. Below target means this PC (or server) is the limit.
    pub send_mbps: f64,
    pub client_limited: bool,
}

fn resolve(server: &str) -> io::Result<std::net::SocketAddr> {
    (server, UDP_PORT)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot resolve server"))
}

fn pps_for(size: u16, mbps: f64) -> f64 {
    (mbps * 1e6 / 8.0 / size as f64).max(1.0)
}

fn finish(dir: UdpDir, size: u16, mbps: f64, sent: u32, send_secs: f64, r: Report) -> CellResult {
    let loss = if sent == 0 { 100.0 } else { 100.0 * (1.0 - (r.received.min(sent) as f64 / sent as f64)) };
    let send_mbps = sent as f64 * size as f64 * 8.0 / send_secs.max(1e-9) / 1e6;
    let achieved = if r.duration_ms > 0.0 { r.bytes as f64 * 8.0 / (r.duration_ms / 1e3) / 1e6 } else { 0.0 };
    CellResult {
        dir,
        size,
        target_mbps: mbps,
        sent,
        received: r.received,
        loss_pct: loss,
        reordered: r.reordered,
        jitter_ms: r.jitter_ms,
        achieved_mbps: achieved,
        send_mbps,
        client_limited: dir == UdpDir::Upload && send_mbps < mbps * 0.9,
    }
}

/// Send packets of `size` bytes at `mbps` for `secs` and ask the server what arrived.
pub fn udp_upload_cell_blocking(server: &str, size: u16, mbps: f64, secs: f64) -> io::Result<CellResult> {
    if (size as usize) < HEADER_LEN || size as usize > MAX_UDP_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad packet size"));
    }
    let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect(resolve(server)?)?;
    let pps = pps_for(size, mbps);
    let mut buf = vec![0u8; size as usize];
    let start = Instant::now();
    let mut sent: u32 = 0;
    while start.elapsed().as_secs_f64() < secs {
        let el = start.elapsed().as_secs_f64();
        let due = ((el * pps) as u64 + 1).min(sent as u64 + 64) as u32;
        while sent < due {
            Header { session, seq: sent, send_ts_ns: start.elapsed().as_nanos() as u64, size, flags: FLAG_DATA }
                .write(&mut buf);
            let _ = sock.send(&buf); // errors count as sent: the packet is lost anyway
            sent += 1;
        }
        let wait = sent as f64 / pps - start.elapsed().as_secs_f64();
        if wait > 0.002 {
            std::thread::sleep(Duration::from_secs_f64(wait - 0.001));
        } else if wait > 0.0 {
            std::hint::spin_loop();
        }
    }
    let send_secs = start.elapsed().as_secs_f64();
    std::thread::sleep(Duration::from_millis(150)); // let packets in flight arrive

    sock.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut req = [0u8; HEADER_LEN];
    Header { session, seq: 0, send_ts_ns: 0, size: HEADER_LEN as u16, flags: FLAG_REPORT_REQ }.write(&mut req);
    let mut rbuf = [0u8; 256];
    for _ in 0..6 {
        let _ = sock.send(&req);
        while let Ok(n) = sock.recv(&mut rbuf) {
            let ok = Header::read(&rbuf[..n]).filter(|h| h.flags & FLAG_REPORT != 0 && h.session == session);
            if ok.is_some() {
                if let Some(r) = Report::read(&rbuf[HEADER_LEN..n]) {
                    return Ok(finish(UdpDir::Upload, size, mbps, sent, send_secs, r));
                }
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "no report from server"))
}

/// Ask the server to send a paced stream and measure what arrives.
pub fn udp_download_cell_blocking(server: &str, size: u16, mbps: f64, secs: f64) -> io::Result<CellResult> {
    let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect(resolve(server)?)?;
    let pps = pps_for(size, mbps);
    let count = (pps * secs).ceil() as u32;
    let mut req = [0u8; HEADER_LEN + 10];
    Header { session, seq: 0, send_ts_ns: 0, size: req.len() as u16, flags: FLAG_SRC_REQ }.write(&mut req);
    req[HEADER_LEN..HEADER_LEN + 2].copy_from_slice(&size.to_be_bytes());
    req[HEADER_LEN + 2..HEADER_LEN + 6].copy_from_slice(&(pps.round().max(1.0) as u32).to_be_bytes());
    req[HEADER_LEN + 6..HEADER_LEN + 10].copy_from_slice(&count.to_be_bytes());
    let start = Instant::now();
    sock.send(&req)?;
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut tracker = RecvTracker::default();
    let mut buf = vec![0u8; 65536];
    let deadline = Duration::from_secs_f64(secs + 1.5);
    while start.elapsed() < deadline && tracker.report().received < count {
        match sock.recv(&mut buf) {
            Ok(n) => {
                if let Some(h) = Header::read(&buf[..n]).filter(|h| h.session == session) {
                    tracker.on_packet(h.seq, h.send_ts_ns, start.elapsed().as_nanos() as u64, n);
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                if tracker.report().received > 0 {
                    break; // stream went quiet: the rest is lost
                }
            }
            Err(_) => {} // e.g. ICMP reset on Windows
        }
    }
    Ok(finish(UdpDir::Download, size, mbps, count, secs, tracker.report()))
}

#[derive(Debug, Clone)]
pub struct SweepConfig {
    pub dir: UdpDir,
    pub sizes: Vec<u16>,
    pub rates_mbps: Vec<f64>,
    pub secs: f64,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            dir: UdpDir::Upload,
            sizes: vec![64, 128, 256, 512, 1024, 1472, 2000, 4000, 8000],
            rates_mbps: vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
            secs: 2.0,
        }
    }
}

/// Why a cell is bad. `baseline_jitter` is the jitter of the lowest-rate cell of the same size.
pub fn bad_reason(c: &CellResult, baseline_jitter: f64) -> Option<&'static str> {
    if c.client_limited {
        Some("sender too slow")
    } else if c.loss_pct > 1.0 {
        Some("loss above 1%")
    } else if c.jitter_ms > (3.0 * baseline_jitter).max(1.0) {
        Some("jitter above 3x baseline")
    } else if c.achieved_mbps < c.target_mbps * 0.9 && c.sent > 0 && c.loss_pct > 0.0 {
        Some("goodput below 90% of target")
    } else {
        None
    }
}

/// Run the grid. Rates go from low to high for each size. A size stops after 2 bad cells in a row.
/// `progress` is called after each cell. Runs on a blocking thread: call from `spawn_blocking`.
pub fn udp_sweep_blocking(
    server: &str,
    cfg: &SweepConfig,
    stop: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(&CellResult),
) -> io::Result<Vec<CellResult>> {
    let mut all = Vec::new();
    for &size in &cfg.sizes {
        let mut baseline = None;
        let mut bad_run = 0;
        for &rate in &cfg.rates_mbps {
            if stop.load(Ordering::Relaxed) {
                return Ok(all);
            }
            let cell = match cfg.dir {
                UdpDir::Upload => udp_upload_cell_blocking(server, size, rate, cfg.secs)?,
                UdpDir::Download => udp_download_cell_blocking(server, size, rate, cfg.secs)?,
            };
            let base = *baseline.get_or_insert(cell.jitter_ms);
            let bad = bad_reason(&cell, base).is_some();
            progress(&cell);
            all.push(cell);
            bad_run = if bad { bad_run + 1 } else { 0 };
            if bad_run >= 2 {
                break;
            }
        }
    }
    Ok(all)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Break {
    pub size: u16,
    pub last_good_mbps: Option<f64>,
    pub first_bad_mbps: Option<f64>,
    pub reason: Option<&'static str>,
}

/// For each size, find the highest clean rate and the first bad one.
pub fn find_breaks(cells: &[CellResult]) -> Vec<Break> {
    breaks_by(cells, |c| c.size, |c| c.target_mbps, |c, base| bad_reason(c, base.jitter_ms))
}

/// Generic break finder. `reason(cell, baseline_cell)` says why a cell is bad, or None.
/// The baseline is the lowest-rate cell of the same size.
pub fn breaks_by<T>(
    cells: &[T],
    size_of: impl Fn(&T) -> u16,
    rate_of: impl Fn(&T) -> f64,
    reason: impl Fn(&T, &T) -> Option<&'static str>,
) -> Vec<Break> {
    let mut sizes: Vec<u16> = cells.iter().map(&size_of).collect();
    sizes.dedup();
    sizes
        .into_iter()
        .map(|size| {
            let mut row: Vec<&T> = cells.iter().filter(|c| size_of(c) == size).collect();
            row.sort_by(|a, b| rate_of(a).partial_cmp(&rate_of(b)).unwrap());
            let mut b = Break { size, last_good_mbps: None, first_bad_mbps: None, reason: None };
            let Some(&base) = row.first() else { return b };
            for c in row {
                match reason(c, base) {
                    None if b.first_bad_mbps.is_none() => b.last_good_mbps = Some(rate_of(c)),
                    Some(r) if b.first_bad_mbps.is_none() => {
                        b.first_bad_mbps = Some(rate_of(c));
                        b.reason = Some(r);
                    }
                    _ => {}
                }
            }
            b
        })
        .collect()
}

/// One plain line per size.
pub fn summary(breaks: &[Break]) -> Vec<String> {
    breaks
        .iter()
        .map(|b| match (b.last_good_mbps, b.first_bad_mbps) {
            (_, Some(bad)) => format!("{} B: breaks at {} Mbps ({})", b.size, bad, b.reason.unwrap_or("?")),
            (Some(good), None) => format!("{} B: clean up to {} Mbps (no break found)", b.size, good),
            (None, None) => format!("{} B: no data", b.size),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(size: u16, rate: f64, loss: f64, jitter: f64) -> CellResult {
        CellResult {
            dir: UdpDir::Upload, size, target_mbps: rate, sent: 1000, received: (1000.0 * (1.0 - loss / 100.0)) as u32,
            loss_pct: loss, reordered: 0, jitter_ms: jitter, achieved_mbps: rate, send_mbps: rate, client_limited: false,
        }
    }

    #[test]
    fn finds_break_on_loss() {
        let cells = vec![cell(1472, 10.0, 0.0, 0.1), cell(1472, 50.0, 0.2, 0.1), cell(1472, 100.0, 5.0, 0.2)];
        let b = find_breaks(&cells);
        assert_eq!(b[0].last_good_mbps, Some(50.0));
        assert_eq!(b[0].first_bad_mbps, Some(100.0));
        assert_eq!(b[0].reason, Some("loss above 1%"));
    }

    #[test]
    fn finds_break_on_jitter_and_clean_row() {
        let cells = vec![cell(64, 1.0, 0.0, 0.1), cell(64, 5.0, 0.0, 4.0), cell(512, 1.0, 0.0, 0.1), cell(512, 5.0, 0.0, 0.2)];
        let b = find_breaks(&cells);
        assert_eq!(b[0].first_bad_mbps, Some(5.0));
        assert_eq!(b[1].first_bad_mbps, None);
        assert_eq!(b[1].last_good_mbps, Some(5.0));
        assert!(summary(&b)[1].contains("clean"));
    }

    #[test]
    fn small_jitter_is_not_a_break() {
        // 0.05 ms -> 0.4 ms is 8x, but still under the 1 ms floor.
        let cells = vec![cell(256, 1.0, 0.0, 0.05), cell(256, 5.0, 0.0, 0.4)];
        assert_eq!(find_breaks(&cells)[0].first_bad_mbps, None);
    }
}
