//! What the cards docked above the message box share: the card frame with its
//! coloured edge, and a body that scrolls past a height limit.
use crate::ui::theme;
use eframe::egui::{self, Frame, Margin, ScrollArea};

/// A docked card: a quiet panel with an `edge` coloured left border.
pub(super) fn card<R>(
    ui: &mut egui::Ui,
    edge: egui::Color32,
    colors: &theme::Palette,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let shown = Frame::default()
        .fill(colors.panel)
        .corner_radius(3)
        .inner_margin(Margin {
            left: 14,
            right: 12,
            top: 8,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        });
    let rect = shown.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            rect.left_top(),
            egui::pos2(rect.left() + 3.0, rect.bottom()),
        ),
        egui::CornerRadius {
            nw: 3,
            sw: 3,
            ne: 0,
            se: 0,
        },
        edge,
    );
    shown.inner
}

/// Content that scrolls once it is taller than `limit`. A docked card sits in
/// a panel that sizes itself from its content, where a scroll area would never
/// grow past the panel's last height; so this asks for the height its content
/// had in the frame before.
pub(super) fn scrolling(
    ui: &mut egui::Ui,
    id: egui::Id,
    limit: f32,
    add: impl FnOnce(&mut egui::Ui),
) {
    let wanted = ui
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or(limit)
        .min(limit);
    let output = ScrollArea::vertical()
        .id_salt(id)
        .max_height(limit)
        .min_scrolled_height(wanted)
        .auto_shrink([false, true])
        .show(ui, add);
    let content = output.content_size.y;
    if (content.min(limit) - wanted).abs() > 0.5 {
        ui.data_mut(|data| data.insert_temp(id, content));
        ui.ctx().request_repaint();
    }
}
