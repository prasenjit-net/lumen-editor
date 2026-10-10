use eframe::egui::{self, Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle, Vec2};
pub(crate) const BG: Color32 = Color32::from_rgb(30, 30, 30);
pub(crate) const PANEL: Color32 = Color32::from_rgb(37, 37, 38);
pub(crate) const SIDEBAR: Color32 = Color32::from_rgb(51, 51, 51);
pub(crate) const TAB: Color32 = Color32::from_rgb(45, 45, 45);
#[cfg(not(target_os = "windows"))]
pub(crate) const TITLE: Color32 = Color32::from_rgb(60, 60, 60);
pub(crate) const STATUS: Color32 = Color32::from_rgb(0, 122, 204);
pub(crate) const ACTIVE: Color32 = Color32::from_rgb(4, 57, 94);
pub(crate) const TEXT: Color32 = Color32::from_rgb(204, 204, 204);
pub(crate) const MUTED: Color32 = Color32::from_rgb(157, 157, 157);
pub(crate) const BORDER: Color32 = Color32::from_rgb(43, 43, 43);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(0, 122, 204);
pub(crate) const HOVER: Color32 = Color32::from_rgb(43, 43, 43);
pub(crate) const INPUT: Color32 = Color32::from_rgb(49, 49, 49);

pub(crate) fn configure_ui(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark));
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.spacing.item_spacing = Vec2::new(6.0, 0.0);
    style.spacing.button_padding = Vec2::new(6.0, 2.0);
    style.spacing.interact_size = Vec2::new(20.0, 22.0);
    style.interaction.resize_grab_radius_side = 5.0;
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(12.5));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(18.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.5));

    let visuals = &mut style.visuals;
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = CornerRadius::same(6);
    visuals.menu_corner_radius = CornerRadius::same(5);
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow::NONE;
    visuals.selection.bg_fill = ACTIVE;
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.text_edit_bg_color = Some(INPUT);
    visuals.extreme_bg_color = BG;
    visuals.weak_text_color = Some(MUTED);
    visuals.hyperlink_color = ACCENT;
    // Text fields use noninteractive visuals when idle. Define all five states
    // so buttons, inputs, menus and selections share the same palette and radius.
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = INPUT;
        widget.weak_bg_fill = Color32::TRANSPARENT;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widget.corner_radius = CornerRadius::same(3);
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.fg_stroke.color = MUTED;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(60));
    visuals.widgets.hovered.bg_fill = HOVER;
    visuals.widgets.hovered.weak_bg_fill = HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.active.bg_fill = ACTIVE;
    visuals.widgets.active.weak_bg_fill = ACTIVE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.open.bg_fill = ACTIVE;
    visuals.widgets.open.weak_bg_fill = ACTIVE;
    visuals.widgets.open.bg_stroke.color = ACCENT;
    ctx.set_style(style);
}
