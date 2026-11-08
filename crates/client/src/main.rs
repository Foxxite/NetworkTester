#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;
mod worker;

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, Plot};
use engine::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use store::{Headline, Results};
use worker::{Msg, Plan};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Latency,
    Jitter,
    Speed,
    Breakdown,
}

struct App {
    server: String,
    plan: Plan,
    tab: Tab,
    running: bool,
    stop: Arc<AtomicBool>,
    rx: Option<Receiver<Msg>>,
    status: String,
    error: Option<String>,
    res: Results,
    previous: Option<Headline>,
    saved: Option<String>,
    /// Test helper: start the run on the first frame (`--autostart`).
    autostart: bool,
}

impl App {
    fn new() -> Self {
        // Optional flags, mostly for testing: --server <host> --autostart --tab <0-3>
        let args: Vec<String> = std::env::args().collect();
        let flag = |name: &str| args.iter().position(|a| a == name);
        let tab = match flag("--tab").and_then(|i| args.get(i + 1)).map(|s| s.as_str()) {
            Some("1") => Tab::Jitter,
            Some("2") => Tab::Speed,
            Some("3") => Tab::Breakdown,
            _ => Tab::Latency,
        };
        Self {
            server: flag("--server").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "127.0.0.1".into()),
            autostart: flag("--autostart").is_some(),
            plan: Plan { ping: true, jitter: true, speed: true, mtu: true, sweeps: [true; 4], quick: true },
            tab,
            running: false,
            stop: Arc::new(AtomicBool::new(false)),
            rx: None,
            status: "Ready".into(),
            error: None,
            res: Results::default(),
            previous: store::latest_headline(),
            saved: None,
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.res = Results::default();
        self.error = None;
        self.saved = None;
        self.stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.running = true;
        self.status = "Starting".into();
        worker::spawn(self.server.trim().to_string(), self.plan, self.stop.clone(), tx, ctx.clone());
    }

    fn drain(&mut self) {
        let Some(rx) = &self.rx else { return };
        let msgs: Vec<Msg> = rx.try_iter().collect();
        for m in msgs {
            match m {
                Msg::Status(s) => self.status = s,
                Msg::UdpPing(p) => self.res.udp_ping = Some(p),
                Msg::TcpPing(p) => self.res.tcp_ping = Some(p),
                Msg::Jitter(p) => self.res.jitter = Some(p),
                Msg::Sample { dir, t, mbps } => match dir {
                    Direction::Download => self.res.down_samples.push([t, mbps]),
                    Direction::Upload => self.res.up_samples.push([t, mbps]),
                },
                Msg::Speed(Direction::Download, r) => self.res.down = Some(r),
                Msg::Speed(Direction::Upload, r) => self.res.up = Some(r),
                Msg::Mtu(m) => self.res.mtu = Some(m),
                Msg::Udp(c) => match c.dir {
                    UdpDir::Upload => self.res.udp_up.push(c),
                    UdpDir::Download => self.res.udp_down.push(c),
                },
                Msg::Tcp(c) => match c.dir {
                    Direction::Upload => self.res.tcp_up.push(c),
                    Direction::Download => self.res.tcp_down.push(c),
                },
                Msg::Error(e) => self.error = Some(e),
                Msg::Done => {
                    self.running = false;
                    self.status = if self.stop.load(Ordering::Relaxed) { "Stopped".into() } else { "Finished".into() };
                    match store::save(&self.server, &self.res) {
                        Ok(p) => {
                            self.saved = Some(p);
                            self.previous = self.previous_for_display();
                        }
                        Err(e) => self.error = Some(format!("could not save results: {e}")),
                    }
                    self.rx = None;
                    return;
                }
            }
        }
    }

    /// After a save, the newest file is this run, so "previous" must be the one before it.
    fn previous_for_display(&self) -> Option<Headline> {
        store::previous_headline()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.autostart {
            self.autostart = false;
            self.start(&ctx);
        }
        self.drain();

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Server");
                ui.add_enabled(!self.running, egui::TextEdit::singleline(&mut self.server).desired_width(180.0));
                if self.running {
                    if ui.button("Stop").clicked() {
                        self.stop.store(true, Ordering::Relaxed);
                        self.status = "Stopping".into();
                    }
                    ui.spinner();
                } else if ui.button(RichText::new("Start").strong()).clicked() {
                    self.start(&ctx);
                }
                ui.label(&self.status);
            });
            ui.add_enabled_ui(!self.running, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Run:");
                    ui.checkbox(&mut self.plan.ping, "Ping");
                    ui.checkbox(&mut self.plan.jitter, "Jitter");
                    ui.checkbox(&mut self.plan.speed, "Speed");
                    ui.checkbox(&mut self.plan.mtu, "MTU");
                    ui.separator();
                    ui.label("Breakdown:");
                    ui.checkbox(&mut self.plan.sweeps[0], "UDP up");
                    ui.checkbox(&mut self.plan.sweeps[1], "UDP down");
                    ui.checkbox(&mut self.plan.sweeps[2], "TCP up");
                    ui.checkbox(&mut self.plan.sweeps[3], "TCP down");
                    ui.checkbox(&mut self.plan.quick, "Quick grid");
                });
            });
            if let Some(e) = &self.error {
                ui.colored_label(Color32::from_rgb(220, 80, 70), format!("Error: {e}"));
            }
            ui.horizontal(|ui| {
                for (t, name) in [(Tab::Latency, "Latency"), (Tab::Jitter, "Jitter"), (Tab::Speed, "Speed"), (Tab::Breakdown, "Breakdown")] {
                    ui.selectable_value(&mut self.tab, t, name);
                }
                if let Some(p) = &self.saved {
                    ui.separator();
                    ui.label(format!("Saved: {p}"));
                    if ui.small_button("Open folder").clicked() {
                        store::open_folder();
                    }
                }
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Latency => self.latency_ui(ui),
            Tab::Jitter => self.jitter_ui(ui),
            Tab::Speed => self.speed_ui(ui),
            Tab::Breakdown => self.breakdown_ui(ui),
        });
    }
}

fn stats_grid(ui: &mut egui::Ui, id: &str, title: &str, s: &Option<PingStats>) {
    ui.heading(title);
    match s {
        None => {
            ui.label("No result yet.");
        }
        Some(s) => {
            ui.label(RichText::new(format!("{:.2} ms", s.avg_ms)).size(32.0).strong());
            egui::Grid::new(id).num_columns(2).show(ui, |ui| {
                for (k, v) in [
                    ("Min", format!("{:.2} ms", s.min_ms)),
                    ("Max", format!("{:.2} ms", s.max_ms)),
                    ("95th percentile", format!("{:.2} ms", s.p95_ms)),
                    ("Packet loss", format!("{} of {}", s.lost, s.sent)),
                ] {
                    ui.label(k);
                    ui.label(v);
                    ui.end_row();
                }
            });
        }
    }
}

impl App {
    fn latency_ui(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            stats_grid(&mut cols[0], "udp_ping", "UDP ping", &self.res.udp_ping);
            stats_grid(&mut cols[1], "tcp_ping", "TCP ping", &self.res.tcp_ping);
        });
        if let Some(p) = &self.previous {
            ui.add_space(12.0);
            ui.separator();
            ui.label(RichText::new("Previous saved run").strong());
            ui.label(p.describe());
        }
    }

    fn jitter_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Jitter");
        match &self.res.jitter {
            None => {
                ui.label("No result yet. The test sends 50 packets per second for 10 seconds.");
            }
            Some(s) => {
                ui.label(RichText::new(format!("{:.2} ms", s.jitter_ms)).size(32.0).strong());
                ui.label(format!(
                    "RTT spread (standard deviation): {:.2} ms. Packet loss: {} of {}.",
                    s.stddev_ms, s.lost, s.sent
                ));
                let (verdict, color) = if s.jitter_ms < 5.0 {
                    ("Good for voice, video and games.", Color32::from_rgb(60, 170, 90))
                } else if s.jitter_ms < 30.0 {
                    ("Usable. You may hear glitches in calls.", Color32::from_rgb(230, 170, 40))
                } else {
                    ("Poor. Real-time apps will suffer.", Color32::from_rgb(210, 70, 60))
                };
                ui.colored_label(color, verdict);
            }
        }
    }

    fn speed_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (name, r) in [("Download", &self.res.down), ("Upload", &self.res.up)] {
                ui.vertical(|ui| {
                    ui.label(name);
                    match r {
                        Some(r) => ui.label(RichText::new(format!("{:.1} Mbps", r.mbps)).size(32.0).strong()),
                        None => ui.label("-"),
                    };
                });
                ui.add_space(24.0);
            }
        });
        Plot::new("speed").legend(Legend::default()).height(ui.available_height() - 8.0).y_axis_label("Mbps").x_axis_label("seconds").show(ui, |p| {
            p.line(Line::new("Download", self.res.down_samples.clone()));
            p.line(Line::new("Upload", self.res.up_samples.clone()));
        });
    }

    fn breakdown_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(m) = &self.res.mtu {
                match m {
                    Some(v) => ui.label(format!("Path MTU: {v} bytes (largest packet that is not split up)")),
                    None => ui.label("Path MTU: below 576 bytes, or no answer"),
                };
            }
            let r = &self.res;
            if r.udp_up.is_empty() && r.udp_down.is_empty() && r.tcp_up.is_empty() && r.tcp_down.is_empty() {
                ui.label("No sweep results yet. Tick the breakdown tests above and press Start.");
            }
            ui.label("Green = clean. Amber = early warning. Red = broken. Grey = not tested (the sweep stops a size after two bad rates).");
            for (title, cells) in [("UDP upload", udp_heat(&r.udp_up)), ("UDP download", udp_heat(&r.udp_down))] {
                heat_section(ui, title, &cells, "size (B) / rate (Mbps)", "packet loss");
            }
            for (title, cells) in [("TCP upload", tcp_heat(&r.tcp_up)), ("TCP download", tcp_heat(&r.tcp_down))] {
                heat_section(ui, title, &cells, "write size (B) / rate (Mbps)", "goodput Mbps");
            }
            let mut lines = Vec::new();
            for (t, cells) in [("UDP up", &r.udp_up), ("UDP down", &r.udp_down)] {
                if !cells.is_empty() {
                    lines.push((t, summary(&find_breaks(cells))));
                }
            }
            for (t, cells) in [("TCP up", &r.tcp_up), ("TCP down", &r.tcp_down)] {
                if !cells.is_empty() {
                    lines.push((t, summary(&tcp_find_breaks(cells))));
                }
            }
            for (t, ls) in lines {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{t} summary")).strong());
                for l in ls {
                    ui.label(l);
                }
            }
        });
    }
}

struct HeatCell {
    size: u16,
    rate: f64,
    sev: u8, // 0 clean, 1 warning, 2 bad
    text: String,
    tip: String,
}

fn udp_heat(cells: &[CellResult]) -> Vec<HeatCell> {
    cells
        .iter()
        .map(|c| {
            let base = cells.iter().filter(|x| x.size == c.size).map(|x| x.target_mbps).fold(f64::MAX, f64::min);
            let base_j = cells.iter().find(|x| x.size == c.size && x.target_mbps == base).map(|x| x.jitter_ms).unwrap_or(0.0);
            let reason = bad_reason(c, base_j);
            let sev = if reason.is_some() { 2 } else if c.loss_pct > 0.1 || c.jitter_ms > 2.0 * base_j.max(0.5) { 1 } else { 0 };
            HeatCell {
                size: c.size,
                rate: c.target_mbps,
                sev,
                text: format!("{:.1}%", c.loss_pct),
                tip: format!(
                    "{} B at {} Mbps\nloss {:.2}%  reordered {}\njitter {:.3} ms\ngoodput {:.1} Mbps (sender reached {:.1})\n{}",
                    c.size, c.target_mbps, c.loss_pct, c.reordered, c.jitter_ms, c.achieved_mbps, c.send_mbps,
                    reason.unwrap_or("clean")
                ),
            }
        })
        .collect()
}

fn tcp_heat(cells: &[TcpCell]) -> Vec<HeatCell> {
    cells
        .iter()
        .map(|c| {
            let reason = tcp_bad_reason(c);
            let sev = if reason.is_some() {
                2
            } else if c.goodput_mbps < c.target_mbps * 0.97 || c.rtt_load_p95_ms > 2.0 * c.rtt_idle_ms + 2.0 {
                1
            } else {
                0
            };
            HeatCell {
                size: c.size,
                rate: c.target_mbps,
                sev,
                text: format!("{:.0}", c.goodput_mbps),
                tip: format!(
                    "{} B writes at {} Mbps\ngoodput {:.1} Mbps\nRTT idle {:.2} ms, under load (95th) {:.2} ms\nlongest stall {:.0} ms\n{}",
                    c.size, c.target_mbps, c.goodput_mbps, c.rtt_idle_ms, c.rtt_load_p95_ms, c.max_stall_ms,
                    reason.unwrap_or("clean")
                ),
            }
        })
        .collect()
}

fn heat_section(ui: &mut egui::Ui, title: &str, cells: &[HeatCell], axes: &str, shows: &str) {
    if cells.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.heading(title);
    ui.label(format!("Rows: {axes}. Cell text: {shows}."));
    let mut sizes: Vec<u16> = cells.iter().map(|c| c.size).collect();
    sizes.sort_unstable();
    sizes.dedup();
    let mut rates: Vec<f64> = cells.iter().map(|c| c.rate).collect();
    rates.sort_by(|a, b| a.partial_cmp(b).unwrap());
    rates.dedup();
    let (cw, ch, lw) = (58.0, 26.0, 56.0);
    egui::Grid::new(title).spacing([2.0, 2.0]).show(ui, |ui| {
        ui.label("");
        for r in &rates {
            ui.label(RichText::new(format!("{r}")).small());
        }
        ui.end_row();
        for s in &sizes {
            ui.add_sized([lw, ch], egui::Label::new(format!("{s}")));
            for r in &rates {
                let cell = cells.iter().find(|c| c.size == *s && c.rate == *r);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(cw, ch), egui::Sense::hover());
                let (fill, text) = match cell {
                    Some(c) => (
                        match c.sev {
                            0 => Color32::from_rgb(52, 150, 80),
                            1 => Color32::from_rgb(214, 156, 36),
                            _ => Color32::from_rgb(198, 62, 54),
                        },
                        c.text.as_str(),
                    ),
                    None => (Color32::from_gray(70), "-"),
                };
                ui.painter().rect_filled(rect, 3.0, fill);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(12.0), Color32::WHITE);
                if let Some(c) = cell {
                    resp.on_hover_text(&c.tip);
                }
            }
            ui.end_row();
        }
    });
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 720.0]).with_title("Network Tester"),
        ..Default::default()
    };
    eframe::run_native("Network Tester", opts, Box::new(|_cc| Ok(Box::new(App::new()))))
}
