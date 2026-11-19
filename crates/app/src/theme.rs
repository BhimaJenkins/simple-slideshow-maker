//! A dark, Premiere-style look.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub const BG_APP: Color32 = Color32::from_rgb(0x16, 0x17, 0x1a);
pub const BG_PANEL: Color32 = Color32::from_rgb(0x1f, 0x20, 0x24);
pub const BG_RAISED: Color32 = Color32::from_rgb(0x2a, 0x2c, 0x31);
pub const BG_TRACK: Color32 = Color32::from_rgb(0x25, 0x27, 0x2c);
pub const BG_VIEWER: Color32 = Color32::from_rgb(0x0e, 0x0f, 0x11);
pub const BORDER: Color32 = Color32::from_rgb(0x34, 0x36, 0x3c);
pub const TEXT: Color32 = Color32::from_rgb(0xe4, 0xe6, 0xeb);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8c, 0x90, 0x99);
pub const ACCENT: Color32 = Color32::from_rgb(0x3d, 0x8b, 0xfd);
pub const PLAYHEAD: Color32 = Color32::from_rgb(0xff, 0x5a, 0x5f);

pub const CLIP_MEDIA: Color32 = Color32::from_rgb(0x2f, 0x5f, 0xc4);
pub const CLIP_TEXT: Color32 = Color32::from_rgb(0x8a, 0x4f, 0xd8);
pub const CLIP_MUSIC: Color32 = Color32::from_rgb(0x1f, 0x9d, 0x63);

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG_PANEL;
    v.window_fill = BG_PANEL;
    v.extreme_bg_color = BG_APP;
    v.faint_bg_color = BG_RAISED;
    v.override_text_color = Some(TEXT);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.selection.bg_fill = ACCENT.linear_multiply(0.6);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.weak_bg_fill = BG_RAISED;
    v.widgets.inactive.bg_fill = BG_RAISED;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x34, 0x37, 0x3e);
    v.widgets.active.weak_bg_fill = ACCENT.linear_multiply(0.5);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.slider_width = 150.0;
    });
}

/// A filled accent button for the main action.
pub fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong()).fill(ACCENT))
}

pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title.to_uppercase()).size(11.0).color(TEXT_DIM).strong());
    ui.add_space(2.0);
}
