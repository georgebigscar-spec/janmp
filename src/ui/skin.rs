//! Skins: the default dark one modelled after AIMP's stock look, a light one and
//! "Ocean Sunset".

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub use crate::config::Theme;

#[derive(Clone, Copy)]
pub struct Skin {
    pub dark: bool,
    pub accent: Color32,
    pub accent_dim: Color32,
    /// Top colour of the spectrum bars.
    pub highlight: Color32,
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
    /// Empty part of sliders and the seek bar.
    pub rail: Color32,
    /// Unplayed part of the waveform.
    pub wave: Color32,
    /// Text edits and other sunken fields.
    pub field: Color32,
    pub widget: Color32,
    pub widget_weak: Color32,
    pub widget_hover: Color32,
    pub widget_hover_weak: Color32,
}

const fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

impl Theme {
    /// The accent a theme comes with; picking a theme resets the accent to it.
    pub fn default_accent(self) -> [u8; 3] {
        match self {
            Theme::Dark => [0xFF, 0x8C, 0x1A],
            Theme::Light => [0xE8, 0x6A, 0x00],
            Theme::OceanSunset => [0xE5, 0x6B, 0x6F],
        }
    }
}

impl Skin {
    pub fn new(theme: Theme, accent: [u8; 3]) -> Self {
        let accent = Color32::from_rgb(accent[0], accent[1], accent[2]);
        match theme {
            // AIMP's stock look: charcoal panels, light-grey text, one warm accent.
            Theme::Dark => Self {
                dark: true,
                accent,
                accent_dim: accent.gamma_multiply(0.35),
                highlight: accent,
                bg: rgb(0x1B1B1D),
                panel: rgb(0x262629),
                list_bg: rgb(0x1E1E21),
                list_alt: rgb(0x222225),
                group_header: rgb(0x2A2A2E),
                border: rgb(0x3A3A3F),
                text: rgb(0xC8C8CC),
                text_dim: rgb(0x84848A),
                text_bright: rgb(0xF2F2F4),
                selection: rgb(0x3A3D45),
                hover: rgb(0x2E2E33),
                control: rgb(0xB8B8BE),
                control_hover: rgb(0xFFFFFF),
                rail: rgb(0x3A3A40),
                wave: rgb(0x5A5A64),
                field: rgb(0x141416),
                widget: rgb(0x333338),
                widget_weak: rgb(0x2E2E33),
                widget_hover: rgb(0x404046),
                widget_hover_weak: rgb(0x3A3A40),
            },
            Theme::Light => Self {
                dark: false,
                accent,
                accent_dim: accent.gamma_multiply(0.3),
                highlight: accent,
                bg: rgb(0xE9E9EC),
                panel: rgb(0xF6F6F8),
                list_bg: rgb(0xFFFFFF),
                list_alt: rgb(0xF5F5F7),
                group_header: rgb(0xEAEAEE),
                border: rgb(0xCDCDD3),
                text: rgb(0x2B2B30),
                text_dim: rgb(0x77777F),
                text_bright: rgb(0x0E0E10),
                selection: rgb(0xDCE3EE),
                hover: rgb(0xEFEFF3),
                control: rgb(0x4A4A52),
                control_hover: rgb(0x000000),
                rail: rgb(0xD2D2D8),
                wave: rgb(0xB4B4BD),
                field: rgb(0xFFFFFF),
                widget: rgb(0xE2E2E7),
                widget_weak: rgb(0xEBEBEF),
                widget_hover: rgb(0xD6D6DD),
                widget_hover_weak: rgb(0xDEDEE4),
            },
            // Dusk blue panels, dusty lavender and rosewood for selection, coral accent,
            // light bronze highlights.
            Theme::OceanSunset => Self {
                dark: true,
                accent,
                accent_dim: accent.gamma_multiply(0.35),
                highlight: rgb(0xEAAC8B),
                bg: rgb(0x22334A),
                panel: rgb(0x355070),
                list_bg: rgb(0x283C57),
                list_alt: rgb(0x2C415E),
                group_header: rgb(0x3D4E6E),
                border: rgb(0x4C6283),
                text: rgb(0xEADFD8),
                text_dim: rgb(0xB0A2B8),
                text_bright: rgb(0xFFF3EA),
                selection: rgb(0x6D597A),
                hover: rgb(0x3A4F6E),
                control: rgb(0xEAAC8B),
                control_hover: rgb(0xFFE2CF),
                rail: rgb(0x4A5E7E),
                wave: rgb(0x7D8FAD),
                field: rgb(0x1B2A3E),
                widget: rgb(0x46597A),
                widget_weak: rgb(0x3F5373),
                widget_hover: rgb(0x6D597A),
                widget_hover_weak: rgb(0x5E5072),
            },
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = if self.dark { Visuals::dark() } else { Visuals::light() };
        let r = CornerRadius::same(3);
        v.panel_fill = self.bg;
        v.window_fill = self.panel;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(4);
        v.extreme_bg_color = self.field;
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
        w.inactive.bg_fill = self.widget;
        w.inactive.weak_bg_fill = self.widget_weak;
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.bg_fill = self.widget_hover;
        w.hovered.weak_bg_fill = self.widget_hover_weak;
        w.hovered.bg_stroke = Stroke::new(1.0, self.accent.gamma_multiply(0.6));
        w.hovered.fg_stroke = Stroke::new(1.5, self.text_bright);
        w.active.bg_fill = self.accent.gamma_multiply(0.7);
        w.active.weak_bg_fill = self.accent.gamma_multiply(0.6);
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(2.0, self.text_bright);
        w.open.weak_bg_fill = self.widget_hover_weak;
        for wv in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            wv.corner_radius = r;
        }
        ctx.set_theme(if self.dark { egui::Theme::Dark } else { egui::Theme::Light });
        ctx.set_visuals(v);
        ctx.all_styles_mut(|s| {
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(6.0, 3.0);
            s.spacing.slider_rail_height = 4.0;
            // Labels are not selectable, so text in the title bar and the player panel drags
            // the window like any other empty spot.
            s.interaction.selectable_labels = false;
        });
    }
}
