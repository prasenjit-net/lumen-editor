use crate::{icon, ui::theme::*};
use eframe::egui::{self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Stroke, Vec2};
use std::path::Path;
pub(crate) fn surface(fill: Color32) -> Frame {
    Frame::default()
        .fill(fill)
        .corner_radius(CornerRadius::same(5))
}
pub(crate) fn canvas() -> Frame {
    Frame::default()
        .fill(Color32::from_gray(24))
        .inner_margin(Margin::same(2))
}
pub(crate) fn explorer_row(
    ui: &mut egui::Ui,
    text: String,
    indent: usize,
    selected: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::ZERO,
            if selected { ACTIVE } else { HOVER },
        );
    }
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .text(
            egui::pos2(rect.left() + indent as f32, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.0),
            TEXT,
        );
    response
}
pub(crate) fn icon_button(ui: &mut egui::Ui, icon: &str, selected: bool) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(icon).size(24.0).color(if selected {
            Color32::from_gray(215)
        } else {
            MUTED
        }))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::ZERO)
        .min_size(Vec2::splat(48.0)),
    )
}
pub(crate) fn file_icon(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "rs" | "go" | "c" | "h" | "cpp" | "hpp" | "js" | "ts" | "tsx" | "jsx" => icon::FILE_CODE,
        "toml" | "json" | "yaml" | "yml" => icon::GEAR,
        "md" | "txt" => icon::FILE_TEXT,
        _ => icon::FILE,
    }
}
