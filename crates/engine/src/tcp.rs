//! TCP load cells and the write size x rate sweep.
use crate::{breaks_by, Break, Direction};
use proto::*;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct TcpCell {
    pub dir: Direction,
    pub size: u16,
    pub target_mbps: f64,
    pub goodput_mbps: f64,
    pub rtt_idle_ms: f64,
    pub rtt_load_p95_ms: f64,
    /// Longest time one write was blocked (upload) or the longest gap between reads (download).
    pub max_stall_ms: f64,
    pub pps: f64,
}

fn connect(server: &str, mode: u8) -> io::Result<TcpStream> {
    let addr = (server, TCP_DATA_PORT)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot resolve server"))?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    s.set_nodelay(true)?;
    s.write_all(&[mode])?;
    Ok(s)
}

/// Ping over a TCP connection. Returns RTTs in ms. Stops at `stop` or after `max` samples.
fn tcp_rtts(server: &str, stop: &AtomicBool, max: usize) -> Vec<f64> {
    let mut v = Vec::new();
    let Ok(mut s) = connect(server, MODE_ECHO) else { return v };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let mut b = [0u8; 8];
    while !stop.load(Ordering::Relaxed) && v.len() < max {
        let t = Instant::now();
        if s.write_all(&b).is_err() || s.read_exact(&mut b).is_err() {
            break;
        }
        v.push(t.elapsed().as_secs_f64() * 1000.0);
        std::thread::sleep(Duration::from_millis(20));
    }
    v
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 * p) as usize).min(v.len() - 1)]
}

pub fn tcp_cell_blocking(server: &str, dir: Direction, size: u16, mbps: f64, secs: f64) -> io::Result<TcpCell> {
    if size == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad write size"));
    }
    let pps = (mbps * 1e6 / 8.0 / size as f64).max(1.0);

    // Idle RTT first, then keep measuring RTT on a second connection while the load runs.
    let never = AtomicBool::new(false);
    let mut idle = tcp_rtts(server, &never, 10);
    let rtt_idle = percentile(&mut idle, 0.5);
    let stop = Arc::new(AtomicBool::new(false));
    let probe = {
        let (srv, stop) = (server.to_string(), stop.clone());
        std::thread::spawn(move || tcp_rtts(&srv, &stop, usize::MAX))
    };

    let result = match dir {
        Direction::Upload => upload_load(server, size, pps, secs),
        Direction::Download => download_load(server, size, pps, secs),
    };
    stop.store(true, Ordering::Relaxed);
    let mut loaded = probe.join().unwrap_or_default();
    let (bytes, elapsed, stall) = result?;

    let goodput = bytes as f64 * 8.0 / elapsed / 1e6;
    Ok(TcpCell {
        dir,
        size,
        target_mbps: mbps,
        goodput_mbps: goodput,
        rtt_idle_ms: rtt_idle,
        rtt_load_p95_ms: percentile(&mut loaded, 0.95),
        max_stall_ms: stall * 1000.0,
        pps,
    })
}

/// Returns (bytes, elapsed secs, longest stall secs).
fn upload_load(server: &str, size: u16, pps: f64, secs: f64) -> io::Result<(u64, f64, f64)> {
    let mut s = connect(server, MODE_SINK)?;
    let buf = vec![0u8; size as usize];
    let start = Instant::now();
    let (mut writes, mut longest) = (0u64, 0.0f64);
    while start.elapsed().as_secs_f64() < secs {
        let due = ((start.elapsed().as_secs_f64() * pps) as u64 + 1).min(writes + 64);
        while writes < due {
            let t = Instant::now();
            s.write_all(&buf)?;
            longest = longest.max(t.elapsed().as_secs_f64());
            writes += 1;
        }
        let wait = writes as f64 / pps - start.elapsed().as_secs_f64();
        if wait > 0.002 {
            std::thread::sleep(Duration::from_secs_f64(wait - 0.001));
        } else if wait > 0.0 {
            std::hint::spin_loop();
        }
    }
    Ok((writes * size as u64, start.elapsed().as_secs_f64(), longest))
}

fn download_load(server: &str, size: u16, pps: f64, secs: f64) -> io::Result<(u64, f64, f64)> {
    let mut s = connect(server, MODE_PACED)?;
    let mut req = [0u8; 6];
    req[0..2].copy_from_slice(&size.to_be_bytes());
    req[2..6].copy_from_slice(&(pps as u32).to_be_bytes());
    s.write_all(&req)?;
    s.set_read_timeout(Some(Duration::from_millis(1500)))?;
    let mut buf = vec![0u8; 65536];
    let start = Instant::now();
    let (mut bytes, mut longest) = (0u64, 0.0f64);
    let mut last = Instant::now();
    while start.elapsed().as_secs_f64() < secs {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes += n as u64;
                longest = longest.max(last.elapsed().as_secs_f64());
                last = Instant::now();
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                longest = longest.max(last.elapsed().as_secs_f64());
            }
            Err(e) => return Err(e),
        }
    }
    Ok((bytes, start.elapsed().as_secs_f64(), longest))
}

pub fn tcp_bad_reason(c: &TcpCell) -> Option<&'static str> {
    // Note: for small writes at high rates, "goodput below target" can also mean this PC cannot
    // issue that many writes. Larger sizes passing at the same rate point to that cause.
    if c.goodput_mbps < c.target_mbps * 0.9 {
        Some("goodput below 90% of target")
    } else if c.max_stall_ms > 500.0 + 2000.0 / c.pps {
        Some("stall over 500 ms")
    } else if c.rtt_load_p95_ms > (c.rtt_idle_ms * 5.0).max(c.rtt_idle_ms + 30.0) {
        Some("RTT inflated (queueing)")
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct TcpSweepConfig {
    pub dir: Direction,
    pub sizes: Vec<u16>,
    pub rates_mbps: Vec<f64>,
    pub secs: f64,
}

impl Default for TcpSweepConfig {
    fn default() -> Self {
        Self {
            dir: Direction::Upload,
            sizes: vec![64, 256, 1024, 4096, 16384, 65000],
            rates_mbps: vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
            secs: 2.0,
        }
    }
}

/// Same rule as the UDP sweep: a size stops after 2 bad cells in a row.
pub fn tcp_sweep_blocking(
    server: &str,
    cfg: &TcpSweepConfig,
    stop: &AtomicBool,
    mut progress: impl FnMut(&TcpCell),
) -> io::Result<Vec<TcpCell>> {
    let mut all = Vec::new();
    for &size in &cfg.sizes {
        let mut bad_run = 0;
        for &rate in &cfg.rates_mbps {
            if stop.load(Ordering::Relaxed) {
                return Ok(all);
            }
            let cell = tcp_cell_blocking(server, cfg.dir, size, rate, cfg.secs)?;
            let bad = tcp_bad_reason(&cell).is_some();
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

pub fn tcp_find_breaks(cells: &[TcpCell]) -> Vec<Break> {
    breaks_by(cells, |c| c.size, |c| c.target_mbps, |c, _| tcp_bad_reason(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(rate: f64, goodput: f64, idle: f64, p95: f64, stall: f64) -> TcpCell {
        TcpCell {
            dir: Direction::Upload, size: 1024, target_mbps: rate, goodput_mbps: goodput, rtt_idle_ms: idle,
            rtt_load_p95_ms: p95, max_stall_ms: stall, pps: 1000.0,
        }
    }

    #[test]
    fn clean_cell_is_ok() {
        assert_eq!(tcp_bad_reason(&cell(10.0, 10.0, 1.0, 2.0, 5.0)), None);
    }

    #[test]
    fn detects_each_failure() {
        assert_eq!(tcp_bad_reason(&cell(100.0, 40.0, 1.0, 2.0, 5.0)), Some("goodput below 90% of target"));
        assert_eq!(tcp_bad_reason(&cell(10.0, 10.0, 1.0, 2.0, 900.0)), Some("stall over 500 ms"));
        assert_eq!(tcp_bad_reason(&cell(10.0, 10.0, 1.0, 80.0, 5.0)), Some("RTT inflated (queueing)"));
    }

    #[test]
    fn breaks_found_per_size() {
        let cells = vec![cell(10.0, 10.0, 1.0, 2.0, 5.0), cell(50.0, 49.0, 1.0, 2.0, 5.0), cell(100.0, 48.0, 1.0, 2.0, 5.0)];
        let b = tcp_find_breaks(&cells);
        assert_eq!(b[0].last_good_mbps, Some(50.0));
        assert_eq!(b[0].first_bad_mbps, Some(100.0));
    }
}
