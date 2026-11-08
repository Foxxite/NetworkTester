#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod store;
mod theme;
mod views;
mod worker;

use eframe::egui::{self, Color32, Margin, RichText};
use engine::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use store::{Headline, Results};
use worker::{Msg, Plan};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Overview,
    Speed,
    Latency,
    Breakdown,
}

struct App {
    server: String,
    plan: Plan,
    tab: Tab,
    dark: bool,
    applied_dark: Option<bool>,
    running: bool,
    stop: Arc<AtomicBool>,
    rx: Option<Receiver<Msg>>,
    status: String,
    steps_started: usize,
    steps_total: usize,
    error: Option<String>,
    res: Results,
    previous: Option<Headline>,
    saved: Option<String>,
    /// Test helper: start the run on the first frame (`--autostart`).
    autostart: bool,
}

impl App {
    fn new() -> Self {
        // Optional flags, mostly for testing: --server <host> --autostart --tab <0-3> --light
        let args: Vec<String> = std::env::args().collect();
        let flag = |name: &str| args.iter().position(|a| a == name);
        let tab = match flag("--tab").and_then(|i| args.get(i + 1)).map(|s| s.as_str()) {
            Some("1") => Tab::Speed,
            Some("2") => Tab::Latency,
            Some("3") => Tab::Breakdown,
            _ => Tab::Overview,
        };
        Self {
            server: flag("--server").and_then(|i| args.get(i + 1)).cloned().unwrap_or_default(),
            autostart: flag("--autostart").is_some(),
            plan: Plan { ping: true, jitter: true, speed: true, mtu: true, sweeps: [true; 4], quick: true },
            tab,
            dark: flag("--light").is_none(),
            applied_dark: None,
            running: false,
            stop: Arc::new(AtomicBool::new(false)),
            rx: None,
            status: "Ready".into(),
            steps_started: 0,
            steps_total: 1,
            error: None,
            res: Results::default(),
            previous: store::latest_headline(),
            saved: None,
        }
    }

    fn step_count(&self) -> usize {
        let p = &self.plan;
        2 * p.ping as usize + p.jitter as usize + 2 * p.speed as usize + p.mtu as usize
            + p.sweeps.iter().filter(|s| **s).count()
    }

    /// Rough run time in seconds, shown next to the Start button.
    fn estimate_secs(&self) -> usize {
        let p = &self.plan;
        let per_sweep = if p.quick { 55 } else { 190 };
        4 * p.ping as usize + 10 * p.jitter as usize + 20 * p.speed as usize + 3 * p.mtu as usize
            + per_sweep * p.sweeps.iter().filter(|s| **s).count()
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
        self.steps_started = 0;
        self.steps_total = self.step_count().max(1);
        worker::spawn(self.server.trim().to_string(), self.plan, self.stop.clone(), tx, ctx.clone());
    }

    fn drain(&mut self) {
        let Some(rx) = &self.rx else { return };
        let msgs: Vec<Msg> = rx.try_iter().collect();
        for m in msgs {
            match m {
                Msg::Status(s) => {
                    self.status = s;
                    self.steps_started += 1;
                }
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
                            // The newest file is this run now, so compare with the one before it.
                            self.previous = store::previous_headline();
                        }
                        Err(e) => self.error = Some(format!("could not save results: {e}")),
                    }
                    self.rx = None;
                    return;
                }
            }
        }
    }

    fn has_results(&self) -> bool {
        let r = &self.res;
        r.udp_ping.is_some() || r.down.is_some() || r.up.is_some() || r.jitter.is_some() || r.mtu.is_some()
            || !r.udp_up.is_empty() || !r.udp_down.is_empty() || !r.tcp_up.is_empty() || !r.tcp_down.is_empty()
    }

    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = theme::pal(self.dark);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 7.0, theme::ORANGE);
            ui.label(RichText::new("Network Tester").size(22.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(if self.dark { "Light mode" } else { "Dark mode" }).clicked() {
                    self.dark = !self.dark;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.add_enabled(
                !self.running,
                egui::TextEdit::singleline(&mut self.server)
                    .hint_text("Server address, e.g. 203.0.113.5")
                    .desired_width(280.0)
                    .margin(Margin::symmetric(10, 8)),
            );
            if self.running {
                let stop = egui::Button::new(RichText::new("Stop").size(16.0).strong().color(p.bad))
                    .stroke(egui::Stroke::new(1.5, p.bad))
                    .min_size(egui::vec2(120.0, 36.0));
                if ui.add(stop).clicked() {
                    self.stop.store(true, Ordering::Relaxed);
                    self.status = "Stopping".into();
                }
            } else {
                let label = if self.has_results() { "Run again" } else { "Start test" };
                let ready = !self.server.trim().is_empty();
                if ui.add_enabled(ready, theme::primary_button(label)).on_disabled_hover_text("Type the server address first").clicked() {
                    self.start(ctx);
                }
                ui.label(theme::muted(&p, format!("about {}", fmt_duration(self.estimate_secs()))));
            }
        });

        if self.running {
            let frac = (self.steps_started.saturating_sub(1)) as f32 / self.steps_total as f32;
            ui.add(
                egui::ProgressBar::new(frac.clamp(0.02, 1.0))
                    .fill(theme::ORANGE)
                    .desired_height(10.0)
                    .text(RichText::new(format!("{}  ({}/{})", self.status, self.steps_started, self.steps_total)).small()),
            );
        } else if self.status != "Ready" {
            ui.label(theme::muted(&p, format!("{}.", self.status)));
        }
        if let Some(e) = &self.error {
            ui.colored_label(p.bad, format!("Problem: {e}"));
        }

        ui.add_enabled_ui(!self.running, |ui| {
            egui::CollapsingHeader::new(RichText::new("Choose what to test").color(p.muted)).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::muted(&p, "Basics:"));
                    ui.checkbox(&mut self.plan.ping, "Ping").on_hover_text("Round-trip time over UDP and TCP");
                    ui.checkbox(&mut self.plan.jitter, "Jitter").on_hover_text("How much the delay varies (10 s)");
                    ui.checkbox(&mut self.plan.speed, "Speed").on_hover_text("TCP download and upload (20 s)");
                    ui.checkbox(&mut self.plan.mtu, "MTU").on_hover_text("Largest packet that is not split up");
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::muted(&p, "Find the breaking point:"));
                    ui.checkbox(&mut self.plan.sweeps[0], "UDP upload");
                    ui.checkbox(&mut self.plan.sweeps[1], "UDP download");
                    ui.checkbox(&mut self.plan.sweeps[2], "TCP upload");
                    ui.checkbox(&mut self.plan.sweeps[3], "TCP download");
                    ui.checkbox(&mut self.plan.quick, "Quick grid")
                        .on_hover_text("Fewer packet sizes and rates. Clear it for a thorough but slow test.");
                });
            });
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            for (t, name) in [(Tab::Overview, "Overview"), (Tab::Speed, "Speed"), (Tab::Latency, "Latency & jitter"), (Tab::Breakdown, "Breaking point")] {
                let on = self.tab == t;
                let b = egui::Button::new(RichText::new(name).size(15.0).strong().color(if on { Color32::WHITE } else { p.text }))
                    .fill(if on { theme::ORANGE } else { Color32::TRANSPARENT })
                    .stroke(if on { egui::Stroke::NONE } else { egui::Stroke::new(1.0, p.stroke) });
                if ui.add(b).clicked() {
                    self.tab = t;
                }
            }
        });
    }
}

fn fmt_duration(secs: usize) -> String {
    if secs < 90 {
        format!("{secs} s")
    } else {
        format!("{} min", (secs + 30) / 60)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_dark != Some(self.dark) {
            theme::apply(&ctx, self.dark);
            self.applied_dark = Some(self.dark);
        }
        if self.autostart && !self.server.trim().is_empty() {
            self.autostart = false;
            self.start(&ctx);
        }
        self.drain();
        let p = theme::pal(self.dark);

        egui::Panel::top("top")
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(24, 14)))
            .show(ui, |ui| self.header(ui, &ctx));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(24, 6)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    match self.tab {
                        Tab::Overview => self.overview_ui(ui, &ctx, &p),
                        Tab::Speed => self.speed_ui(ui, &p),
                        Tab::Latency => self.latency_ui(ui, &p),
                        Tab::Breakdown => self.breakdown_ui(ui, &p),
                    }
                    ui.add_space(20.0);
                });
            });
    }
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 780.0])
            .with_min_inner_size([760.0, 560.0])
            .with_title("Network Tester")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).expect("bad icon")),
        ..Default::default()
    };
    eframe::run_native("Network Tester", opts, Box::new(|_cc| Ok(Box::new(App::new()))))
}
