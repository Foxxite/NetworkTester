//! Colours, style and small reusable widgets. The look is inspired by speed.cloudflare.com:
//! calm background, white/dark cards, big numbers, one orange accent.
use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, RichText, Stroke, TextStyle};

pub const ORANGE: Color32 = Color32::from_rgb(246, 130, 31);

#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: Color32,
    pub card: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
    pub blue: Color32,
}

pub fn pal(dark: bool) -> Pal {
    if dark {
        Pal {
            bg: Color32::from_rgb(20, 22, 27),
            card: Color32::from_rgb(30, 33, 40),
            stroke: Color32::from_rgb(48, 52, 62),
            text: Color32::from_rgb(232, 234, 238),
            muted: Color32::from_rgb(150, 157, 170),
            good: Color32::from_rgb(52, 168, 98),
            warn: Color32::from_rgb(226, 160, 36),
            bad: Color32::from_rgb(214, 76, 66),
            blue: Color32::from_rgb(74, 158, 230),
        }
    } else {
        Pal {
            bg: Color32::from_rgb(244, 245, 247),
            card: Color32::WHITE,
            stroke: Color32::from_rgb(226, 229, 234),
            text: Color32::from_rgb(28, 30, 34),
            muted: Color32::from_rgb(107, 114, 128),
            good: Color32::from_rgb(34, 150, 80),
            warn: Color32::from_rgb(214, 140, 10),
            bad: Color32::from_rgb(204, 56, 46),
            blue: Color32::from_rgb(30, 120, 200),
        }
    }
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = pal(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.extreme_bg_color = if dark { Color32::from_rgb(14, 15, 19) } else { Color32::WHITE };
    v.faint_bg_color = p.card;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = ORANGE.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, ORANGE);
    v.hyperlink_color = ORANGE;
    let r = CornerRadius::same(8);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.inactive.bg_fill = p.card;
    v.widgets.inactive.weak_bg_fill = p.card;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ORANGE);
    v.widgets.hovered.weak_bg_fill = ORANGE.gamma_multiply(0.15);
    v.widgets.active.weak_bg_fill = ORANGE.gamma_multiply(0.3);
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 10.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.interact_size.y = 30.0;
        s.text_styles.insert(TextStyle::Heading, FontId::proportional(22.0));
        s.text_styles.insert(TextStyle::Body, FontId::proportional(15.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(15.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(12.5));
    });
}

/// A rounded box with a thin border. Fills the width it is given.
pub fn card<R>(ui: &mut egui::Ui, p: &Pal, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.stroke))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub fn muted(p: &Pal, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(p.muted)
}

pub fn section_title(ui: &mut egui::Ui, p: &Pal, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).heading().strong());
    if !subtitle.is_empty() {
        ui.label(muted(p, subtitle));
    }
}

/// Small coloured pill, e.g. "Good".
pub fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.2))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).strong().size(13.0));
        });
}

/// Big-number tile. `value` is None while there is no result.
pub fn tile(
    ui: &mut egui::Ui,
    p: &Pal,
    label: &str,
    hint: &str,
    value: Option<String>,
    unit: &str,
    note: Option<(String, Color32)>,
    accent: Color32,
) {
    card(ui, p, |ui| {
        ui.set_min_height(104.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(4.0, 14.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, accent);
            ui.label(RichText::new(label.to_uppercase()).small().strong().color(p.muted)).on_hover_text(hint);
        });
        ui.horizontal(|ui| {
            match &value {
                Some(v) => {
                    ui.label(RichText::new(v).size(38.0).strong());
                    ui.label(muted(p, unit).size(15.0));
                }
                None => {
                    ui.label(RichText::new("--").size(38.0).strong().color(p.stroke));
                }
            }
        });
        match note {
            Some((n, c)) => ui.label(RichText::new(n).small().color(c)),
            None => ui.label(RichText::new(" ").small()),
        };
    });
}

#[derive(Clone, Copy, PartialEq)]
pub enum Grade {
    Good,
    Okay,
    Poor,
    Unknown,
}

impl Grade {
    pub fn label(self) -> &'static str {
        match self {
            Grade::Good => "Good",
            Grade::Okay => "Okay",
            Grade::Poor => "Poor",
            Grade::Unknown => "Not tested",
        }
    }
    pub fn color(self, p: &Pal) -> Color32 {
        match self {
            Grade::Good => p.good,
            Grade::Okay => p.warn,
            Grade::Poor => p.bad,
            Grade::Unknown => p.muted,
        }
    }
    /// One step worse (Good -> Okay -> Poor).
    pub fn worse(self) -> Grade {
        match self {
            Grade::Good => Grade::Okay,
            Grade::Okay => Grade::Poor,
            g => g,
        }
    }
}

/// A flat primary button in the accent colour.
pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).size(16.0).strong().color(Color32::WHITE))
        .fill(ORANGE)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(8))
        .min_size(egui::vec2(120.0, 36.0))
}
