//! The four tab bodies.
use crate::theme::{self, card, chip, muted, section_title, tile, Grade, Pal};
use crate::App;
use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, Plot};
use engine::*;

fn fmt_speed(v: f64) -> String {
    if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") }
}

/// "+12% vs previous", green when it got better.
fn delta(cur: f64, prev: Option<f64>, higher_is_better: bool, p: &Pal) -> Option<(String, Color32)> {
    let prev = prev?;
    if prev.abs() < 1e-9 {
        return None;
    }
    let pct = (cur - prev) / prev * 100.0;
    if pct.abs() < 3.0 {
        return Some(("about the same as previous run".into(), p.muted));
    }
    let better = (pct > 0.0) == higher_is_better;
    Some((format!("{pct:+.0}% vs previous run"), if better { p.good } else { p.bad }))
}

impl App {
    // ---------------------------------------------------------------- Overview
    pub(crate) fn overview_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: &Pal) {
        if !self.has_results() && !self.running {
            self.welcome(ui, p);
            return;
        }
        let r = &self.res;
        let prev = self.previous.clone().unwrap_or_default();
        let live = |s: &Vec<[f64; 2]>| if self.running { s.last().map(|x| x[1]) } else { None };

        ui.columns(4, |c| {
            let down = r.down.as_ref().map(|s| s.mbps);
            let up = r.up.as_ref().map(|s| s.mbps);
            let note = |v: Option<f64>, prev: Option<f64>, hib: bool, running_msg: Option<f64>| match (v, running_msg) {
                (Some(v), _) => delta(v, prev, hib, p),
                (None, Some(l)) => Some((format!("measuring... {} Mbps now", fmt_speed(l)), p.muted)),
                _ => None,
            };
            tile(&mut c[0], p, "Download", "TCP download speed, 4 parallel streams", down.map(fmt_speed), "Mbps",
                note(down, prev.down_mbps, true, live(&r.down_samples)), p.blue);
            tile(&mut c[1], p, "Upload", "TCP upload speed, 4 parallel streams", up.map(fmt_speed), "Mbps",
                note(up, prev.up_mbps, true, live(&r.up_samples)), theme::ORANGE);
            let ping = r.udp_ping.as_ref();
            tile(&mut c[2], p, "Latency", "Average UDP round-trip time", ping.map(|s| format!("{:.1}", s.avg_ms)), "ms",
                ping.and_then(|s| delta(s.avg_ms, prev.ping_ms, false, p))
                    .or_else(|| ping.map(|s| (format!("{} of {} packets lost", s.lost, s.sent), if s.lost > 0 { p.warn } else { p.muted }))),
                p.good);
            let jit = r.jitter.as_ref().or(r.udp_ping.as_ref());
            tile(&mut c[3], p, "Jitter", "How much the delay changes from packet to packet", jit.map(|s| format!("{:.2}", s.jitter_ms)), "ms",
                jit.and_then(|s| delta(s.jitter_ms, prev.jitter_ms, false, p))
                    .or_else(|| jit.map(|s| (format!("delay spread {:.2} ms", s.stddev_ms), p.muted))),
                p.warn);
        });

        ui.add_space(4.0);
        ui.columns(2, |c| {
            card(&mut c[0], p, |ui| {
                section_title(ui, p, "Network quality", "What your connection can handle right now");
                for (name, grade, why) in self.grades(p) {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(140.0, 22.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| ui.label(RichText::new(name).strong()),
                        );
                        chip(ui, grade.label(), grade.color(p));
                        ui.label(muted(p, why).small());
                    });
                }
            });
            card(&mut c[1], p, |ui| {
                section_title(ui, p, "What we found", "");
                let f = self.findings(p);
                if f.is_empty() {
                    ui.label(muted(p, if self.running { "Findings appear here as tests finish." } else { "Nothing to report. Tick more tests and run again." }));
                }
                for (color, text) in f {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("●").color(color));
                        ui.label(text);
                    });
                }
            });
        });

        if !self.running {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Copy summary").on_hover_text("Copy the main numbers as text").clicked() {
                    ctx.copy_text(self.summary_text());
                }
                if let Some(path) = &self.saved {
                    ui.label(muted(p, format!("Saved to {path}")).small());
                    if ui.small_button("Open folder").clicked() {
                        crate::store::open_folder();
                    }
                }
            });
        }
    }

    fn welcome(&self, ui: &mut egui::Ui, p: &Pal) {
        ui.add_space(30.0);
        card(ui, p, |ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Find out what is wrong with your connection").size(28.0).strong());
            ui.label(muted(p, "Speed tests show only part of the story. This tool also finds the packet size and rate where your link starts to fail.").size(16.0));
            ui.add_space(10.0);
            for (title, text) in [
                ("Speed", "Download and upload, measured with parallel TCP streams."),
                ("Latency and jitter", "How fast packets travel, and how steady that is. Matters for calls and games."),
                ("Breaking point", "Sends UDP and TCP at growing packet sizes and rates until loss or delay appears."),
                ("Path MTU", "The largest packet your path can carry without splitting it."),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("●").color(theme::ORANGE));
                    ui.label(RichText::new(title).strong());
                    ui.label(muted(p, text));
                });
            }
            ui.add_space(10.0);
            ui.separator();
            ui.label(RichText::new("How to start").strong());
            ui.label("1. Run the NetworkTester server at the other end of the link you want to test (see the README: Docker, or Ubuntu/Debian without Docker).");
            ui.label("2. Type the server address in the box above.");
            ui.label("3. Press Start test.");
            ui.add_space(8.0);
        });
    }

    /// Streaming / gaming / calls verdicts.
    fn grades(&self, p: &Pal) -> Vec<(&'static str, Grade, String)> {
        let r = &self.res;
        let loss_pct = r.udp_ping.as_ref().map(|s| 100.0 * s.lost as f64 / s.sent.max(1) as f64).unwrap_or(0.0);
        let ping = r.udp_ping.as_ref().map(|s| s.avg_ms);
        let jitter = r.jitter.as_ref().or(r.udp_ping.as_ref()).map(|s| s.jitter_ms);
        let lossy = |g: Grade| if loss_pct > 2.0 { g.worse() } else { g };
        let _ = p;

        let streaming = match r.down.as_ref().map(|s| s.mbps) {
            None => (Grade::Unknown, "needs the speed test".to_string()),
            Some(d) => (
                lossy(if d >= 25.0 { Grade::Good } else if d >= 8.0 { Grade::Okay } else { Grade::Poor }),
                format!("{} Mbps download", fmt_speed(d)),
            ),
        };
        let gaming = match (ping, jitter) {
            (Some(l), Some(j)) => (
                lossy(if l < 40.0 && j < 10.0 { Grade::Good } else if l < 100.0 && j < 30.0 { Grade::Okay } else { Grade::Poor }),
                format!("{l:.0} ms latency, {j:.1} ms jitter"),
            ),
            _ => (Grade::Unknown, "needs ping and jitter".into()),
        };
        let up = r.up.as_ref().map(|s| s.mbps);
        let calls = match (ping, jitter) {
            (Some(l), Some(j)) => {
                let base = if j < 15.0 && l < 150.0 { Grade::Good } else if j < 30.0 && l < 250.0 { Grade::Okay } else { Grade::Poor };
                let g = if up.is_some_and(|u| u < 2.0) { base.worse() } else { base };
                (lossy(g), format!("{j:.1} ms jitter{}", up.map(|u| format!(", {} Mbps upload", fmt_speed(u))).unwrap_or_default()))
            }
            _ => (Grade::Unknown, "needs ping and jitter".into()),
        };
        vec![
            ("Video streaming", streaming.0, streaming.1),
            ("Online gaming", gaming.0, gaming.1),
            ("Video calls", calls.0, calls.1),
        ]
    }

    /// Plain sentences about the main problems found.
    fn findings(&self, p: &Pal) -> Vec<(Color32, String)> {
        let r = &self.res;
        let mut out = Vec::new();
        match r.mtu {
            Some(Some(m)) if m < 1500 => out.push((p.warn, format!("Path MTU is {m} bytes, below the usual 1500. Big packets may be split up or dropped."))),
            Some(Some(m)) => out.push((p.good, format!("Path MTU is {m} bytes."))),
            Some(None) => out.push((p.bad, "The MTU probe got no answer. Something blocks large UDP packets.".into())),
            None => {}
        }
        if let Some(s) = &r.udp_ping {
            if s.lost > 0 {
                out.push((p.warn, format!("{} of {} ping packets were lost.", s.lost, s.sent)));
            }
        }
        let sweeps: [(&str, Vec<Break>, f64); 4] = [
            ("UDP upload", find_breaks(&r.udp_up), max_rate_udp(&r.udp_up)),
            ("UDP download", find_breaks(&r.udp_down), max_rate_udp(&r.udp_down)),
            ("TCP upload", tcp_find_breaks(&r.tcp_up), max_rate_tcp(&r.tcp_up)),
            ("TCP download", tcp_find_breaks(&r.tcp_down), max_rate_tcp(&r.tcp_down)),
        ];
        for (name, breaks, max_rate) in sweeps {
            if breaks.is_empty() {
                continue;
            }
            let worst = breaks.iter().filter_map(|b| b.first_bad_mbps.map(|m| (m, b))).min_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            match worst {
                Some((rate, b)) => out.push((
                    p.bad,
                    format!("{name} starts to fail at {rate} Mbps with {}-byte packets ({}).", b.size, b.reason.unwrap_or("problem")),
                )),
                None => out.push((p.good, format!("{name} is clean up to {max_rate} Mbps."))),
            }
        }
        out
    }

    fn summary_text(&self) -> String {
        let r = &self.res;
        let f = |v: Option<f64>, u: &str| v.map(|x| format!("{x:.1} {u}")).unwrap_or_else(|| "-".into());
        let mut s = format!(
            "Network Tester results for {}\nDownload: {}\nUpload: {}\nLatency: {}\nJitter: {}\n",
            self.server.trim(),
            f(r.down.as_ref().map(|x| x.mbps), "Mbps"),
            f(r.up.as_ref().map(|x| x.mbps), "Mbps"),
            f(r.udp_ping.as_ref().map(|x| x.avg_ms), "ms"),
            f(r.jitter.as_ref().or(r.udp_ping.as_ref()).map(|x| x.jitter_ms), "ms"),
        );
        for (_, text) in self.findings(&theme::pal(self.dark)) {
            s += &format!("- {text}\n");
        }
        s
    }

    // ---------------------------------------------------------------- Speed
    pub(crate) fn speed_ui(&mut self, ui: &mut egui::Ui, p: &Pal) {
        let r = &self.res;
        ui.columns(2, |c| {
            tile(&mut c[0], p, "Download", "TCP download speed", r.down.as_ref().map(|s| fmt_speed(s.mbps)), "Mbps", None, p.blue);
            tile(&mut c[1], p, "Upload", "TCP upload speed", r.up.as_ref().map(|s| fmt_speed(s.mbps)), "Mbps", None, theme::ORANGE);
        });
        card(ui, p, |ui| {
            section_title(ui, p, "Speed over time", "4 parallel TCP streams. The first 2 seconds are warm-up and are not counted in the result.");
            if r.down_samples.is_empty() && r.up_samples.is_empty() {
                ui.add_space(30.0);
                ui.label(muted(p, "No speed test yet. Run a test with Speed ticked."));
                ui.add_space(30.0);
                return;
            }
            Plot::new("speed")
                .legend(Legend::default())
                .height(320.0)
                .y_axis_label("Mbps")
                .x_axis_label("seconds")
                .allow_zoom(false)
                .allow_drag(false)
                .allow_scroll(false)
                .include_y(0.0)
                .show(ui, |pl| {
                    pl.line(Line::new("Download", r.down_samples.clone()).color(p.blue).width(2.5));
                    pl.line(Line::new("Upload", r.up_samples.clone()).color(theme::ORANGE).width(2.5));
                });
        });
    }

    // ---------------------------------------------------------------- Latency
    pub(crate) fn latency_ui(&mut self, ui: &mut egui::Ui, p: &Pal) {
        let r = &self.res;
        ui.columns(3, |c| {
            ping_card(&mut c[0], p, "UDP ping", "100 probes, 20 ms apart", r.udp_ping.as_ref());
            ping_card(&mut c[1], p, "TCP ping", "100 round trips on an open connection", r.tcp_ping.as_ref());
            card(&mut c[2], p, |ui| {
                section_title(ui, p, "Jitter", "50 packets per second for 10 s");
                match &r.jitter {
                    None => {
                        ui.add_space(8.0);
                        ui.label(muted(p, "No result yet."));
                    }
                    Some(s) => {
                        ui.label(RichText::new(format!("{:.2} ms", s.jitter_ms)).size(34.0).strong());
                        let (txt, g) = if s.jitter_ms < 5.0 {
                            ("Great for calls, video and games", Grade::Good)
                        } else if s.jitter_ms < 30.0 {
                            ("Usable. Calls may glitch sometimes", Grade::Okay)
                        } else {
                            ("Poor. Real-time apps will suffer", Grade::Poor)
                        };
                        ui.horizontal(|ui| chip(ui, txt, g.color(p)));
                        ui.add_space(4.0);
                        kv(ui, p, "j", &[
                            ("Delay spread (std. dev.)", format!("{:.2} ms", s.stddev_ms)),
                            ("Packet loss", format!("{} of {}", s.lost, s.sent)),
                        ]);
                    }
                }
            });
        });
        if let Some(prev) = &self.previous {
            card(ui, p, |ui| {
                ui.label(RichText::new("Previous saved run").strong());
                ui.label(muted(p, prev.describe()));
            });
        }
    }

    // ---------------------------------------------------------------- Breaking point
    pub(crate) fn breakdown_ui(&mut self, ui: &mut egui::Ui, p: &Pal) {
        let r = &self.res;
        if let Some(m) = &r.mtu {
            card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Path MTU").strong());
                    match m {
                        Some(v) => ui.label(format!("{v} bytes - the largest packet that crosses the path without being split up.")),
                        None => ui.label("No answer. Large UDP packets may be blocked."),
                    };
                });
            });
        }
        let none = r.udp_up.is_empty() && r.udp_down.is_empty() && r.tcp_up.is_empty() && r.tcp_down.is_empty();
        if none {
            card(ui, p, |ui| {
                section_title(ui, p, "Where does the connection break down?", "");
                ui.label("This test sends data at growing packet sizes and rates. A cell turns red when loss, delay or stalls appear. The first red cell in a row is the breaking point.");
                ui.add_space(6.0);
                ui.label(muted(p, "No sweep results yet. Open \"Choose what to test\", tick the breaking-point tests and press Start test."));
            });
            return;
        }
        ui.horizontal(|ui| {
            for (t, c) in [("Clean", p.good), ("Early warning", p.warn), ("Broken", p.bad), ("Not tested", p.stroke)] {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 3.0, c);
                ui.label(muted(p, t).small());
            }
        });
        let udp_hint = "Each cell shows packet loss. Hover for details.";
        let tcp_hint = "Each cell shows goodput in Mbps. Hover for details.";
        heat_card(ui, p, "UDP upload", udp_hint, &udp_heat(&r.udp_up), &summary(&find_breaks(&r.udp_up)));
        heat_card(ui, p, "UDP download", udp_hint, &udp_heat(&r.udp_down), &summary(&find_breaks(&r.udp_down)));
        heat_card(ui, p, "TCP upload", tcp_hint, &tcp_heat(&r.tcp_up), &summary(&tcp_find_breaks(&r.tcp_up)));
        heat_card(ui, p, "TCP download", tcp_hint, &tcp_heat(&r.tcp_down), &summary(&tcp_find_breaks(&r.tcp_down)));
    }
}

fn max_rate_udp(c: &[CellResult]) -> f64 {
    c.iter().map(|x| x.target_mbps).fold(0.0, f64::max)
}
fn max_rate_tcp(c: &[TcpCell]) -> f64 {
    c.iter().map(|x| x.target_mbps).fold(0.0, f64::max)
}

fn kv(ui: &mut egui::Ui, p: &Pal, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id).num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        for (k, v) in rows {
            ui.label(muted(p, *k));
            ui.label(v);
            ui.end_row();
        }
    });
}

fn ping_card(ui: &mut egui::Ui, p: &Pal, title: &str, sub: &str, s: Option<&PingStats>) {
    card(ui, p, |ui| {
        section_title(ui, p, title, sub);
        match s {
            None => {
                ui.add_space(8.0);
                ui.label(muted(p, "No result yet."));
            }
            Some(s) => {
                ui.label(RichText::new(format!("{:.2} ms", s.avg_ms)).size(34.0).strong());
                ui.label(muted(p, "average").small());
                kv(ui, p, title, &[
                    ("Fastest", format!("{:.2} ms", s.min_ms)),
                    ("Slowest", format!("{:.2} ms", s.max_ms)),
                    ("95th percentile", format!("{:.2} ms", s.p95_ms)),
                    ("Packet loss", format!("{} of {}", s.lost, s.sent)),
                ]);
            }
        }
    });
}

// ------------------------------------------------------------------ heat maps
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
                    "{} byte packets at {} Mbps\nloss {:.2}%   reordered {}\njitter {:.3} ms\ngoodput {:.1} Mbps (sender reached {:.1})\n{}",
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
                    "{} byte writes at {} Mbps\ngoodput {:.1} Mbps\nRTT idle {:.2} ms, under load (95th) {:.2} ms\nlongest stall {:.0} ms\n{}",
                    c.size, c.target_mbps, c.goodput_mbps, c.rtt_idle_ms, c.rtt_load_p95_ms, c.max_stall_ms,
                    reason.unwrap_or("clean")
                ),
            }
        })
        .collect()
}

fn heat_card(ui: &mut egui::Ui, p: &Pal, title: &str, hint: &str, cells: &[HeatCell], lines: &[String]) {
    if cells.is_empty() {
        return;
    }
    card(ui, p, |ui| {
        section_title(ui, p, title, hint);
        let mut sizes: Vec<u16> = cells.iter().map(|c| c.size).collect();
        sizes.sort_unstable();
        sizes.dedup();
        let mut rates: Vec<f64> = cells.iter().map(|c| c.rate).collect();
        rates.sort_by(|a, b| a.partial_cmp(b).unwrap());
        rates.dedup();
        let (cw, ch, lw) = (66.0, 30.0, 70.0);
        ui.label(muted(p, "Rows: packet size in bytes.  Columns: send rate in Mbps.").small());
        egui::Grid::new(title).spacing([3.0, 3.0]).show(ui, |ui| {
            ui.label("");
            for r in &rates {
                ui.add_sized([cw, 18.0], egui::Label::new(muted(p, format!("{r}")).small()));
            }
            ui.end_row();
            for s in &sizes {
                ui.add_sized([lw, ch], egui::Label::new(RichText::new(format!("{s} B")).small().color(p.muted)));
                for r in &rates {
                    let cell = cells.iter().find(|c| c.size == *s && c.rate == *r);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(cw, ch), egui::Sense::hover());
                    let (fill, text) = match cell {
                        Some(c) => (
                            match c.sev {
                                0 => p.good,
                                1 => p.warn,
                                _ => p.bad,
                            },
                            c.text.as_str(),
                        ),
                        None => (p.stroke, ""),
                    };
                    ui.painter().rect_filled(rect, 6.0, fill);
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), Color32::WHITE);
                    if let Some(c) = cell {
                        resp.on_hover_text(&c.tip);
                    }
                }
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        for l in lines {
            let bad = l.contains("breaks at");
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("●").color(if bad { p.bad } else { p.good }));
                ui.label(l);
            });
        }
    });
}
