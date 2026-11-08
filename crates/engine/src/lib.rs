mod udp;
pub use udp::*;
use proto::*;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::timeout;

#[derive(Debug, Default)]
pub struct PingStats {
    pub sent: u32,
    pub lost: u32,
    pub min_ms: f64,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub p95_ms: f64,
    /// RFC 3550 style jitter: mean absolute difference of consecutive RTTs.
    pub jitter_ms: f64,
    /// Standard deviation of RTT.
    pub stddev_ms: f64,
}

pub fn summarize(rtts: &[f64], sent: u32) -> PingStats {
    let mut s = PingStats { sent, lost: sent - rtts.len() as u32, ..Default::default() };
    if rtts.is_empty() {
        return s;
    }
    let mut sorted = rtts.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s.min_ms = sorted[0];
    s.max_ms = *sorted.last().unwrap();
    s.avg_ms = rtts.iter().sum::<f64>() / rtts.len() as f64;
    s.p95_ms = sorted[((sorted.len() as f64 * 0.95) as usize).min(sorted.len() - 1)];
    let mut j = 0.0;
    for w in rtts.windows(2) {
        j += ((w[1] - w[0]).abs() - j) / 16.0;
    }
    s.jitter_ms = j;
    s.stddev_ms =
        (rtts.iter().map(|r| (r - s.avg_ms).powi(2)).sum::<f64>() / rtts.len() as f64).sqrt();
    s
}

pub async fn udp_ping(server: &str, count: u32, interval: Duration) -> std::io::Result<PingStats> {
    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    sock.connect((server, UDP_PORT)).await?;
    let start = Instant::now();
    let mut rtts = Vec::new();
    let mut buf = [0u8; 64];
    for seq in 0..count {
        let h = Header { session: 1, seq, send_ts_ns: start.elapsed().as_nanos() as u64, size: 64, flags: FLAG_ECHO };
        h.write(&mut buf);
        let t0 = Instant::now();
        sock.send(&buf).await?;
        if let Ok(Ok(n)) = timeout(Duration::from_secs(1), sock.recv(&mut buf)).await {
            if Header::read(&buf[..n]).map(|r| r.seq) == Some(seq) {
                rtts.push(t0.elapsed().as_secs_f64() * 1000.0);
            }
        }
        tokio::time::sleep(interval).await;
    }
    Ok(summarize(&rtts, count))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    Download,
    Upload,
}

#[derive(Debug, Default)]
pub struct SpeedResult {
    pub mbps: f64,
    pub bytes: u64,
    pub secs: f64,
}

/// TCP speed test: `streams` parallel connections for `secs` seconds.
/// The first `warmup` seconds are not counted.
pub async fn tcp_speed(
    server: &str,
    dir: Direction,
    streams: usize,
    secs: f64,
    warmup: f64,
) -> std::io::Result<SpeedResult> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let total = Arc::new(AtomicU64::new(0));
    let counting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut tasks = Vec::new();
    for _ in 0..streams {
        let mut s = TcpStream::connect((server, TCP_DATA_PORT)).await?;
        s.set_nodelay(true)?;
        s.write_all(&[if dir == Direction::Download { MODE_SOURCE } else { MODE_SINK }]).await?;
        let (total, counting, stop) = (total.clone(), counting.clone(), stop.clone());
        tasks.push(tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            while !stop.load(Ordering::Relaxed) {
                let n = match dir {
                    Direction::Download => match s.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    },
                    Direction::Upload => match s.write(&buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    },
                };
                if counting.load(Ordering::Relaxed) {
                    total.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
        }));
    }
    tokio::time::sleep(Duration::from_secs_f64(warmup)).await;
    counting.store(true, Ordering::Relaxed);
    let t0 = Instant::now();
    tokio::time::sleep(Duration::from_secs_f64(secs - warmup)).await;
    let bytes = total.load(Ordering::Relaxed);
    let elapsed = t0.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    for t in tasks {
        t.abort();
    }
    Ok(SpeedResult { mbps: bytes as f64 * 8.0 / elapsed / 1e6, bytes, secs: elapsed })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jitter_constant_is_zero() {
        assert_eq!(summarize(&[5.0; 20], 20).jitter_ms, 0.0);
    }
    #[test]
    fn loss_counted() {
        assert_eq!(summarize(&[1.0, 2.0], 5).lost, 3);
    }
}
