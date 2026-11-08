mod mtu;
mod tcp;
mod udp;
pub use mtu::*;
pub use tcp::*;
pub use udp::*;
use proto::*;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::timeout;

#[derive(Debug, Default, Clone, serde::Serialize)]
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
    udp_ping_ctl(server, count, interval, &std::sync::atomic::AtomicBool::new(false)).await
}

/// Same as `udp_ping`, but ends early when `stop` is set (stats cover the packets sent so far).
pub async fn udp_ping_ctl(
    server: &str,
    count: u32,
    interval: Duration,
    stop: &std::sync::atomic::AtomicBool,
) -> std::io::Result<PingStats> {
    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    sock.connect((server, UDP_PORT)).await?;
    let start = Instant::now();
    let mut rtts = Vec::new();
    let mut buf = [0u8; 64];
    let mut sent = 0;
    for seq in 0..count {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        sent += 1;
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
    Ok(summarize(&rtts, sent))
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub enum Direction {
    Download,
    Upload,
}

#[derive(Debug, Default, Clone, serde::Serialize)]
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
    tcp_speed_live(server, dir, streams, secs, warmup, &std::sync::atomic::AtomicBool::new(false), |_, _| {}).await
}

/// Like `tcp_speed`, but calls `on_sample(seconds, mbps)` every 250 ms (warm-up included, for a
/// live graph) and ends early when `stop` is set.
pub async fn tcp_speed_live(
    server: &str,
    dir: Direction,
    streams: usize,
    secs: f64,
    warmup: f64,
    user_stop: &std::sync::atomic::AtomicBool,
    mut on_sample: impl FnMut(f64, f64),
) -> std::io::Result<SpeedResult> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let total = Arc::new(AtomicU64::new(0));
    let all = Arc::new(AtomicU64::new(0)); // every byte, for the live graph
    let counting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut tasks = Vec::new();
    for _ in 0..streams {
        let mut s = TcpStream::connect((server, TCP_DATA_PORT)).await?;
        s.set_nodelay(true)?;
        s.write_all(&[if dir == Direction::Download { MODE_SOURCE } else { MODE_SINK }]).await?;
        let (total, counting, stop, all) = (total.clone(), counting.clone(), stop.clone(), all.clone());
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
                all.fetch_add(n as u64, Ordering::Relaxed);
                if counting.load(Ordering::Relaxed) {
                    total.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
        }));
    }
    let begin = Instant::now();
    let (mut last_t, mut last_bytes) = (0.0f64, 0u64);
    let mut t0 = Instant::now();
    let mut warm = true;
    while begin.elapsed().as_secs_f64() < secs && !user_stop.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let t = begin.elapsed().as_secs_f64();
        let b = all.load(Ordering::Relaxed);
        on_sample(t, (b - last_bytes) as f64 * 8.0 / (t - last_t).max(1e-6) / 1e6);
        (last_t, last_bytes) = (t, b);
        if warm && t >= warmup {
            warm = false;
            counting.store(true, Ordering::Relaxed);
            t0 = Instant::now();
        }
    }
    let bytes = total.load(Ordering::Relaxed);
    let elapsed = t0.elapsed().as_secs_f64().max(1e-6);
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
