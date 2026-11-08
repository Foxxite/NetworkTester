//! Result model, saving to disk (JSON + CSV) and reading the previous run.
use engine::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Fallback when no per-user folder is known (not the normal case on Windows).
const FALLBACK_DIR: &str = "NetworkTester-results";

#[derive(Default, Serialize)]
pub struct Results {
    pub udp_ping: Option<PingStats>,
    pub tcp_ping: Option<PingStats>,
    pub jitter: Option<PingStats>,
    pub down_samples: Vec<[f64; 2]>,
    pub up_samples: Vec<[f64; 2]>,
    pub down: Option<SpeedResult>,
    pub up: Option<SpeedResult>,
    pub mtu: Option<Option<u32>>,
    pub udp_up: Vec<CellResult>,
    pub udp_down: Vec<CellResult>,
    pub tcp_up: Vec<TcpCell>,
    pub tcp_down: Vec<TcpCell>,
}

/// The few numbers shown when comparing with an earlier run.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Headline {
    pub time: u64,
    pub ping_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub down_mbps: Option<f64>,
    pub up_mbps: Option<f64>,
    pub mtu: Option<u32>,
}

impl Headline {
    pub fn from(r: &Results, time: u64) -> Self {
        Self {
            time,
            ping_ms: r.udp_ping.as_ref().map(|p| p.avg_ms),
            jitter_ms: r.jitter.as_ref().map(|p| p.jitter_ms),
            down_mbps: r.down.as_ref().map(|s| s.mbps),
            up_mbps: r.up.as_ref().map(|s| s.mbps),
            mtu: r.mtu.flatten(),
        }
    }

    pub fn describe(&self) -> String {
        let f = |v: Option<f64>, unit: &str| v.map(|x| format!("{x:.1} {unit}")).unwrap_or_else(|| "-".into());
        format!(
            "Ping {}   Jitter {}   Download {}   Upload {}   MTU {}",
            f(self.ping_ms, "ms"),
            f(self.jitter_ms, "ms"),
            f(self.down_mbps, "Mbps"),
            f(self.up_mbps, "Mbps"),
            self.mtu.map(|m| m.to_string()).unwrap_or_else(|| "-".into())
        )
    }
}

/// Where results are saved: `%LOCALAPPDATA%\NetworkTester\results`. This folder is always writable,
/// also when the app is installed in Program Files.
pub fn dir() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(base) => PathBuf::from(base).join("NetworkTester").join("results"),
        None => PathBuf::from(FALLBACK_DIR),
    }
}

fn run_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir())
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| {
        p.extension().is_some_and(|e| e == "json")
            && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("run-"))
    });
    v.sort();
    v
}

fn read_headline(p: &PathBuf) -> Option<Headline> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    serde_json::from_value(v.get("headline")?.clone()).ok()
}

/// Newest saved run.
pub fn latest_headline() -> Option<Headline> {
    run_files().last().and_then(read_headline)
}

/// The run before the newest one (used right after saving the current run).
pub fn previous_headline() -> Option<Headline> {
    let f = run_files();
    f.len().checked_sub(2).and_then(|i| read_headline(&f[i]))
}

/// Write `run-<unix>.json` and `run-<unix>.csv`. Returns the JSON path.
pub fn save(server: &str, r: &Results) -> std::io::Result<String> {
    std::fs::create_dir_all(dir())?;
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let json = serde_json::json!({ "time": time, "server": server, "headline": Headline::from(r, time), "results": r });
    let jp = dir().join(format!("run-{time}.json"));
    std::fs::write(&jp, serde_json::to_string_pretty(&json)?)?;

    let mut csv = String::from("test,size_bytes,target_mbps,goodput_mbps,loss_pct,jitter_ms,rtt_idle_ms,rtt_load_p95_ms,note\n");
    for (name, cells) in [("udp_up", &r.udp_up), ("udp_down", &r.udp_down)] {
        for c in cells {
            let base = cells.iter().filter(|x| x.size == c.size).map(|x| x.target_mbps).fold(f64::MAX, f64::min);
            let bj = cells.iter().find(|x| x.size == c.size && x.target_mbps == base).map(|x| x.jitter_ms).unwrap_or(0.0);
            csv += &format!(
                "{name},{},{},{:.3},{:.3},{:.4},,,{}\n",
                c.size, c.target_mbps, c.achieved_mbps, c.loss_pct, c.jitter_ms, bad_reason(c, bj).unwrap_or("clean")
            );
        }
    }
    for (name, cells) in [("tcp_up", &r.tcp_up), ("tcp_down", &r.tcp_down)] {
        for c in cells {
            csv += &format!(
                "{name},{},{},{:.3},,,{:.3},{:.3},{}\n",
                c.size, c.target_mbps, c.goodput_mbps, c.rtt_idle_ms, c.rtt_load_p95_ms, tcp_bad_reason(c).unwrap_or("clean")
            );
        }
    }
    std::fs::write(dir().join(format!("run-{time}.csv")), csv)?;
    Ok(jp.display().to_string())
}

pub fn open_folder() {
    let _ = std::fs::create_dir_all(dir());
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(dir()).spawn();
}
