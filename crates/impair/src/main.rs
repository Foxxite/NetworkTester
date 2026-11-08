//! Test tool: a UDP proxy that makes a link worse (loss, rate limit, delay, jitter).
//! usage: impair <listen_ip> <upstream_ip> [loss_pct] [rate_mbps] [delay_ms] [jitter_ms] [queue_ms]
//! Both ports are 7001. Point the client at <listen_ip>.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

#[derive(Clone, Copy)]
struct Cfg {
    loss: f64,    // 0..1
    rate_bps: f64, // 0 = unlimited
    delay: f64,   // seconds
    jitter: f64,  // seconds, +- uniform
    queue: f64,   // seconds of buffer at the bottleneck
}

struct Link {
    next_free: Instant,
    rng: u64,
}

impl Link {
    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
    /// When to deliver this packet, or None if the link drops it.
    fn plan(&mut self, now: Instant, len: usize, c: &Cfg) -> Option<Instant> {
        if self.rand() < c.loss {
            return None;
        }
        let mut t = now;
        if c.rate_bps > 0.0 {
            let start = self.next_free.max(now);
            if (start - now).as_secs_f64() > c.queue {
                return None; // buffer full: tail drop
            }
            let tx = Duration::from_secs_f64((len + 28) as f64 * 8.0 / c.rate_bps);
            self.next_free = start + tx;
            t = self.next_free;
        }
        let j = (self.rand() * 2.0 - 1.0) * c.jitter;
        Some(t + Duration::from_secs_f64((c.delay + j).max(0.0)))
    }
}

async fn deliver(sock: Arc<UdpSocket>, to: Option<SocketAddr>, data: Vec<u8>, at: Instant) {
    tokio::time::sleep_until(at.into()).await;
    let _ = match to {
        Some(a) => sock.send_to(&data, a).await,
        None => sock.send(&data).await,
    };
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let arg = |i: usize, d: f64| a.get(i).and_then(|x| x.parse().ok()).unwrap_or(d);
    let cfg = Cfg {
        loss: arg(3, 0.0) / 100.0,
        rate_bps: arg(4, 0.0) * 1e6,
        delay: arg(5, 0.0) / 1e3,
        jitter: arg(6, 0.0) / 1e3,
        queue: arg(7, 20.0) / 1e3,
    };
    let listen = format!("{}:7001", a.get(1).expect("listen_ip"));
    let upstream: SocketAddr = format!("{}:7001", a.get(2).expect("upstream_ip")).parse().unwrap();
    let front = Arc::new(UdpSocket::bind(&listen).await?);
    println!("impair on {listen} -> {upstream}");

    let up_link = Arc::new(Mutex::new(Link { next_free: Instant::now(), rng: 0x9E3779B97F4A7C15 }));
    let down_link = Arc::new(Mutex::new(Link { next_free: Instant::now(), rng: 0xD1B54A32D192ED03 }));
    let mut clients: HashMap<SocketAddr, Arc<UdpSocket>> = HashMap::new();
    let mut buf = vec![0u8; 65536];
    loop {
        let (n, from) = front.recv_from(&mut buf).await?;
        let back = match clients.get(&from) {
            Some(s) => s.clone(),
            None => {
                let s = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
                s.connect(upstream).await?;
                clients.insert(from, s.clone());
                // server -> client direction
                let (s2, front2, link) = (s.clone(), front.clone(), down_link.clone());
                tokio::spawn(async move {
                    let mut b = vec![0u8; 65536];
                    while let Ok(n) = s2.recv(&mut b).await {
                        let at = link.lock().unwrap().plan(Instant::now(), n, &cfg);
                        if let Some(at) = at {
                            tokio::spawn(deliver(front2.clone(), Some(from), b[..n].to_vec(), at));
                        }
                    }
                });
                s
            }
        };
        let at = up_link.lock().unwrap().plan(Instant::now(), n, &cfg);
        if let Some(at) = at {
            tokio::spawn(deliver(back, None, buf[..n].to_vec(), at));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_limit_drops_burst() {
        let mut l = Link { next_free: Instant::now(), rng: 1 };
        let c = Cfg { loss: 0.0, rate_bps: 10e6, delay: 0.0, jitter: 0.0, queue: 0.02 };
        let now = Instant::now();
        // 1000-byte packets at 10 Mbps take ~0.8 ms each: 20 ms queue holds ~25.
        let kept = (0..100).filter(|_| l.plan(now, 1000, &c).is_some()).count();
        assert!((20..=30).contains(&kept), "kept {kept}");
    }
    #[test]
    fn loss_rate_is_close() {
        let mut l = Link { next_free: Instant::now(), rng: 7 };
        let c = Cfg { loss: 0.05, rate_bps: 0.0, delay: 0.0, jitter: 0.0, queue: 0.02 };
        let now = Instant::now();
        let dropped = (0..20000).filter(|_| l.plan(now, 100, &c).is_none()).count();
        assert!((800..1200).contains(&dropped), "dropped {dropped}");
    }
}
