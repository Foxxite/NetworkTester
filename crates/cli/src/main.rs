use engine::Direction;
use std::time::Duration;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let server = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1".into());
    let what = std::env::args().nth(2).unwrap_or_else(|| "all".into());

    if what == "ping" || what == "all" {
        let s = engine::udp_ping(&server, 100, Duration::from_millis(20)).await?;
        println!("PING   avg {:.2} ms  min {:.2}  max {:.2}  p95 {:.2}  loss {}/{}",
            s.avg_ms, s.min_ms, s.max_ms, s.p95_ms, s.lost, s.sent);
    }
    if what == "jitter" || what == "all" {
        // 50 packets per second for 10 s.
        let s = engine::udp_ping(&server, 500, Duration::from_millis(20)).await?;
        println!("JITTER {:.3} ms  (RTT stddev {:.3} ms, loss {}/{})",
            s.jitter_ms, s.stddev_ms, s.lost, s.sent);
    }
    if what == "speed" || what == "all" {
        for (name, dir) in [("DOWN", Direction::Download), ("UP  ", Direction::Upload)] {
            let r = engine::tcp_speed(&server, dir, 4, 10.0, 2.0).await?;
            println!("{name}   {:.0} Mbps  ({} MB in {:.1} s)", r.mbps, r.bytes / 1_000_000, r.secs);
        }
    }
    if what == "udp-sweep" || what == "udp-sweep-down" {
        // Optional: nt-cli <server> udp-sweep <sizes,comma> <rates,comma> <secs>
        let mut cfg = engine::SweepConfig::default();
        if what == "udp-sweep-down" {
            cfg.dir = engine::UdpDir::Download;
        }
        if let Some(s) = std::env::args().nth(3) {
            cfg.sizes = s.split(',').filter_map(|x| x.parse().ok()).collect();
        }
        if let Some(s) = std::env::args().nth(4) {
            cfg.rates_mbps = s.split(',').filter_map(|x| x.parse().ok()).collect();
        }
        if let Some(s) = std::env::args().nth(5).and_then(|x| x.parse().ok()) {
            cfg.secs = s;
        }
        println!("{:>6} {:>8} {:>8} {:>8} {:>7} {:>9}", "size", "target", "sent", "goodput", "loss%", "jitter ms");
        let srv = server.clone();
        let c2 = cfg.clone();
        let cells = tokio::task::spawn_blocking(move || {
            let stop = std::sync::atomic::AtomicBool::new(false);
            engine::udp_sweep_blocking(&srv, &c2, &stop, |c| {
                println!("{:>6} {:>8.1} {:>8.1} {:>8.1} {:>7.2} {:>9.3}{}", c.size, c.target_mbps, c.send_mbps,
                    c.achieved_mbps, c.loss_pct, c.jitter_ms, if c.client_limited { "  (sender-limited)" } else { "" });
            })
        })
        .await
        .unwrap()?;
        for line in engine::summary(&engine::find_breaks(&cells)) {
            println!("{line}");
        }
    }
    Ok(())
}
