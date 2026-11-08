//! Runs the tests on a background thread and sends results to the UI.
use engine::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct Plan {
    pub ping: bool,
    pub jitter: bool,
    pub speed: bool,
    pub mtu: bool,
    /// UDP up, UDP down, TCP up, TCP down.
    pub sweeps: [bool; 4],
    pub quick: bool,
}

pub enum Msg {
    Status(String),
    UdpPing(PingStats),
    TcpPing(PingStats),
    Jitter(PingStats),
    Sample { dir: Direction, t: f64, mbps: f64 },
    Speed(Direction, SpeedResult),
    Mtu(Option<u32>),
    Udp(CellResult),
    Tcp(TcpCell),
    Error(String),
    Done,
}

pub fn spawn(server: String, plan: Plan, stop: Arc<AtomicBool>, tx: Sender<Msg>, ctx: eframe::egui::Context) {
    std::thread::spawn(move || {
        let send = |m: Msg| {
            let _ = tx.send(m);
            ctx.request_repaint();
        };
        if let Err(e) = run(&server, plan, &stop, &send) {
            send(Msg::Error(format!("{e}")));
        }
        send(Msg::Done);
    });
}

fn run(server: &str, plan: Plan, stop: &AtomicBool, send: &dyn Fn(Msg)) -> std::io::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    let stopped = || stop.load(Ordering::Relaxed);

    if plan.ping && !stopped() {
        send(Msg::Status("UDP ping".into()));
        send(Msg::UdpPing(rt.block_on(udp_ping_ctl(server, 100, Duration::from_millis(20), stop))?));
        send(Msg::Status("TCP ping".into()));
        send(Msg::TcpPing(tcp_ping_blocking(server, 100, stop)));
    }
    if plan.jitter && !stopped() {
        send(Msg::Status("Jitter (10 s)".into()));
        send(Msg::Jitter(rt.block_on(udp_ping_ctl(server, 500, Duration::from_millis(20), stop))?));
    }
    if plan.speed {
        for (name, dir) in [("Download", Direction::Download), ("Upload", Direction::Upload)] {
            if stopped() {
                break;
            }
            send(Msg::Status(format!("{name} speed (10 s)")));
            let r = rt.block_on(tcp_speed_live(server, dir, 4, 10.0, 2.0, stop, |t, mbps| {
                send(Msg::Sample { dir, t, mbps })
            }))?;
            send(Msg::Speed(dir, r));
        }
    }
    if plan.mtu && !stopped() {
        send(Msg::Status("Path MTU".into()));
        send(Msg::Mtu(probe_path_mtu_blocking(server)?));
    }

    let (rates, udp_sizes, tcp_sizes): (Vec<f64>, Vec<u16>, Vec<u16>) = if plan.quick {
        (vec![1.0, 10.0, 50.0, 100.0, 500.0], vec![64, 512, 1472, 4000], vec![64, 1024, 16384, 65000])
    } else {
        (SweepConfig::default().rates_mbps, SweepConfig::default().sizes, TcpSweepConfig::default().sizes)
    };
    let names = ["UDP upload", "UDP download", "TCP upload", "TCP download"];
    for i in 0..4 {
        if !plan.sweeps[i] || stopped() {
            continue;
        }
        send(Msg::Status(format!("Breakdown sweep: {}", names[i])));
        if i < 2 {
            let cfg = SweepConfig {
                dir: if i == 0 { UdpDir::Upload } else { UdpDir::Download },
                sizes: udp_sizes.clone(),
                rates_mbps: rates.clone(),
                secs: 2.0,
            };
            udp_sweep_blocking(server, &cfg, stop, |c| send(Msg::Udp(c.clone())))?;
        } else {
            let cfg = TcpSweepConfig {
                dir: if i == 2 { Direction::Upload } else { Direction::Download },
                sizes: tcp_sizes.clone(),
                rates_mbps: rates.clone(),
                secs: 2.0,
            };
            tcp_sweep_blocking(server, &cfg, stop, |c| send(Msg::Tcp(c.clone())))?;
        }
    }
    Ok(())
}
