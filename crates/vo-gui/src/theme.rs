//! Colours and small reusable widgets.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};

pub const ACCENT: Color32 = Color32::from_rgb(46, 214, 196);
pub const ACCENT2: Color32 = Color32::from_rgb(160, 132, 255);
pub const BG: Color32 = Color32::from_rgb(18, 20, 28);
pub const CARD: Color32 = Color32::from_rgb(28, 31, 43);
pub const MUTED: Color32 = Color32::from_rgb(140, 146, 168);
pub const WARN: Color32 = Color32::from_rgb(255, 190, 70);
pub const ERR: Color32 = Color32::from_rgb(255, 95, 95);
pub const DARK: Color32 = Color32::from_rgb(14, 15, 21);
pub const BUTTON: Color32 = Color32::from_rgb(40, 44, 60);
pub const STOP: Color32 = Color32::from_rgb(110, 48, 60);

pub fn style(ctx: &egui::Context) {
    // Always dark, whatever the system theme (otherwise a light Windows theme would
    // switch egui to its light style, which the colours below don't touch).
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 7.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        s.spacing.slider_width = 210.0;
        let v = &mut s.visuals;
        v.panel_fill = BG;
        v.window_fill = BG;
        v.extreme_bg_color = DARK;
        v.selection.bg_fill = ACCENT.linear_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.hyperlink_color = ACCENT;
        v.slider_trailing_fill = true;
        v.override_text_color = Some(Color32::from_rgb(218, 222, 234));
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(6);
        }
        v.widgets.inactive.weak_bg_fill = BUTTON;
        v.widgets.inactive.bg_fill = BUTTON;
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(52, 57, 78);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(60, 66, 90);
    });
}

pub fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(12.5).color(MUTED).strong());
            ui.add_space(2.0);
            add(ui)
        })
        .inner
}

/// Horizontal level meter: value in 0..1, with a caption.
pub fn meter(ui: &mut egui::Ui, label: &str, frac: f32, caption: &str, color: Color32) {
    ui.horizontal(|ui| {
        cell(ui, 92.0, RichText::new(label).color(MUTED));
        let w = (ui.available_width() - 70.0).max(60.0);
        bar(ui, w, 10.0, frac, color);
        ui.label(RichText::new(caption).monospace().size(11.5).color(MUTED));
    });
}

/// A plain progress/level bar.
pub fn bar(ui: &mut egui::Ui, w: f32, h: f32, frac: f32, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
    let p = ui.painter();
    let r = CornerRadius::same((h / 2.0) as u8);
    p.rect_filled(rect, r, DARK);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    if fill.width() > 1.0 {
        p.rect_filled(fill, r, color);
    }
    resp
}

/// A label in a fixed-width, left-aligned cell (for form-like rows).
pub fn cell(ui: &mut egui::Ui, w: f32, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.label(text)
    })
    .inner
}

pub fn db_frac(db: f32) -> f32 {
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

pub fn slider_row(ui: &mut egui::Ui, label: &str, hint: &str, value: &mut f32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        cell(ui, 160.0, label).on_hover_text(hint);
        ui.spacing_mut().slider_width = (ui.available_width() - 52.0).max(80.0);
        let r = ui.add(egui::Slider::new(value, 0.0..=1.0).show_value(false));
        changed = r.changed();
        ui.label(RichText::new(format!("{:>3.0}%", *value * 100.0)).monospace().color(MUTED));
    });
    changed
}

/// A coloured button with dark or light text.
pub fn pill_button(text: impl Into<String>, fill: Color32, dark_text: bool) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).strong().color(if dark_text { Color32::BLACK } else { Color32::WHITE })).fill(fill)
}

/// Open a file or folder with the system's default app.
pub fn open_path(p: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}

/// Show a file in its folder (selected, on Windows).
pub fn reveal(p: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
    }
    #[cfg(not(windows))]
    {
        if let Some(d) = p.parent() {
            let _ = std::process::Command::new("xdg-open").arg(d).spawn();
        }
    }
}
