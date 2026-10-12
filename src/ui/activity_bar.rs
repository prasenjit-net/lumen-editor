use super::Workbench;
use super::theme::{ACCENT, SIDEBAR};
use super::widgets::{self, canvas, surface};
use crate::app::view_state::Activity;
use crate::icon;
use eframe::egui::{self, Stroke};
impl Workbench<'_> {
    pub(crate) fn activity_bar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("activity")
            .exact_width(52.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(canvas())
            .show(ctx, |ui| {
                surface(SIDEBAR).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.vertical_centered(|ui| {
                        for (activity, glyph) in [
                            (Activity::Explorer, icon::FILES),
                            (Activity::Search, icon::MAGNIFYING_GLASS),
                            (Activity::SourceControl, icon::GIT_BRANCH),
                            (Activity::RunDebug, icon::DEBUG),
                            (Activity::Extensions, icon::PUZZLE_PIECE),
                        ] {
                            let response =
                                widgets::icon_button(ui, glyph, self.view.activity == activity);
                            if self.view.activity == activity {
                                ui.painter().vline(
                                    response.rect.left(),
                                    response.rect.y_range(),
                                    Stroke::new(2.0_f32, ACCENT),
                                );
                            }
                            if response.clicked() {
                                self.view.activity = activity;
                                self.view.sidebar_visible = true;
                            }
                        }
                        ui.add_space((ui.available_height() - 50.0).max(0.0));
                        if widgets::icon_button(ui, icon::GEAR, self.view.settings.open)
                            .on_hover_text("Settings (Cmd/Ctrl+,)")
                            .clicked()
                        {
                            self.command(crate::app::command::Command::Settings);
                        }
                    });
                });
            });
    }
}
