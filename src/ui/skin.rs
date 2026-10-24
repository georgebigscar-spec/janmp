//! The default dark skin, modelled after AIMP's stock look: charcoal panels, light-grey
//! text and a single warm accent colour.

use egui::{Color32, CornerRadius, Stroke, Visuals};

#[derive(Clone, Copy)]
pub struct Skin {
    pub accent: Color32,
    pub accent_dim: Color32,
    /// Window background behind everything.
    pub bg: Color32,
    /// Player panel background (slightly lighter than `bg`).
    pub panel: Color32,
    /// Playlist background.
    pub list_bg: Color32,
    pub list_alt: Color32,
    pub group_header: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_bright: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub control: Color32,
    pub control_hover: Color32,
}

impl Skin {
    pub fn new(accent: [u8; 3]) -> Self {
        let accent = Color32::from_rgb(accent[0], accent[1], accent[2]);
        Self {
            accent,
            accent_dim: accent.gamma_multiply(0.35),
            bg: Color32::from_rgb(0x1B, 0x1B, 0x1D),
            panel: Color32::from_rgb(0x26, 0x26, 0x29),
            list_bg: Color32::from_rgb(0x1E, 0x1E, 0x21),
            list_alt: Color32::from_rgb(0x22, 0x22, 0x25),
            group_header: Color32::from_rgb(0x2A, 0x2A, 0x2E),
            border: Color32::from_rgb(0x3A, 0x3A, 0x3F),
            text: Color32::from_rgb(0xC8, 0xC8, 0xCC),
            text_dim: Color32::from_rgb(0x84, 0x84, 0x8A),
            text_bright: Color32::from_rgb(0xF2, 0xF2, 0xF4),
            selection: Color32::from_rgb(0x3A, 0x3D, 0x45),
            hover: Color32::from_rgb(0x2E, 0x2E, 0x33),
            control: Color32::from_rgb(0xB8, 0xB8, 0xBE),
            control_hover: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = Visuals::dark();
        let r = CornerRadius::same(3);
        v.panel_fill = self.bg;
        v.window_fill = self.panel;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(4);
        v.extreme_bg_color = Color32::from_rgb(0x14, 0x14, 0x16);
        v.faint_bg_color = self.list_alt;
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent.gamma_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0, self.text_bright);
        v.slider_trailing_fill = true;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.bg_fill = Color32::from_rgb(0x33, 0x33, 0x38);
        w.inactive.weak_bg_fill = Color32::from_rgb(0x2E, 0x2E, 0x33);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.bg_fill = Color32::from_rgb(0x40, 0x40, 0x46);
        w.hovered.weak_bg_fill = Color32::from_rgb(0x3A, 0x3A, 0x40);
        w.hovered.bg_stroke = Stroke::new(1.0, self.accent.gamma_multiply(0.6));
        w.hovered.fg_stroke = Stroke::new(1.5, self.text_bright);
        w.active.bg_fill = self.accent.gamma_multiply(0.7);
        w.active.weak_bg_fill = self.accent.gamma_multiply(0.6);
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(2.0, self.text_bright);
        w.open.weak_bg_fill = Color32::from_rgb(0x3A, 0x3A, 0x40);
        for wv in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            wv.corner_radius = r;
        }
        ctx.set_theme(egui::Theme::Dark);
        ctx.set_visuals(v);
        ctx.all_styles_mut(|s| {
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(6.0, 3.0);
            s.spacing.slider_rail_height = 4.0;
        });
    }
}
