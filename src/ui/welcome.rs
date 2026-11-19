//! The page shown in an editor group with no open document: the app, what to do
//! first, what was open recently, and where the main features live. Every entry
//! dispatches an existing command; nothing here opens a dialog or reads a file.
use super::Workbench;
use crate::{
    app::{command::Command, view_state::Activity},
    icon,
};
use eframe::egui::{self, CornerRadius, RichText, Stroke};

/// Widest the content column grows, and the side margin kept around it.
const MAX_WIDTH: f32 = 640.0;
const MARGIN: f32 = 24.0;
/// Below this width the two columns stack.
const TWO_COLUMNS_FROM: f32 = 520.0;
const ICON_SIZE: f32 = 48.0;

/// The app icon as a small texture, decoded once per run.
fn app_icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let key = egui::Id::new("lumen_welcome_icon");
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(key)) {
        return Some(texture);
    }
    let decoded = image::load_from_memory(include_bytes!("../../assets/icons/lumen.png")).ok()?;
    let size = (ICON_SIZE * 2.0) as u32;
    let rgba = decoded
        .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let image = egui::ColorImage::from_rgba_unmultiplied([size as usize; 2], rgba.as_raw());
    let texture = ctx.load_texture("lumen-icon", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(key, texture.clone()));
    Some(texture)
}

/// One clickable line: label on the left, an optional shortcut at the right edge.
fn link_row(
    ui: &mut egui::Ui,
    label: &str,
    shortcut: Option<&str>,
    accent: bool,
) -> egui::Response {
    let colors = crate::ui::theme::palette(ui.visuals());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), colors.control_hover);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    let ink = if accent { colors.info } else { colors.text };
    let painter = ui.painter().with_clip_rect(rect);
    painter.text(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        ink,
    );
    if let Some(shortcut) = shortcut {
        painter.text(
            egui::pos2(rect.right() - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            shortcut,
            egui::FontId::proportional(crate::ui::theme::META_SIZE),
            colors.muted,
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A bordered entry point with a one-line description.
fn tile(
    ui: &mut egui::Ui,
    glyph: &str,
    title: &str,
    shortcut: Option<&str>,
    description: &str,
) -> egui::Response {
    let colors = crate::ui::theme::palette(ui.visuals());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::click());
    let radius = CornerRadius::same(4);
    if response.hovered() {
        ui.painter().rect_filled(rect, radius, colors.control_hover);
    }
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0_f32,
            if response.has_focus() {
                colors.accent
            } else {
                colors.float_border
            },
        ),
        egui::StrokeKind::Inside,
    );
    let painter = ui.painter().with_clip_rect(rect.shrink(1.0));
    painter.text(
        egui::pos2(rect.left() + 12.0, rect.top() + 18.0),
        egui::Align2::LEFT_CENTER,
        glyph,
        egui::FontId::proportional(16.0),
        colors.warning,
    );
    painter.text(
        egui::pos2(rect.left() + 38.0, rect.top() + 17.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(13.0),
        colors.text,
    );
    if let Some(shortcut) = shortcut {
        painter.text(
            egui::pos2(rect.right() - 10.0, rect.top() + 17.0),
            egui::Align2::RIGHT_CENTER,
            shortcut,
            egui::FontId::proportional(crate::ui::theme::META_SIZE),
            colors.muted,
        );
    }
    let text_width = rect.width() - 38.0 - 10.0;
    let galley = crate::ui::widgets::fit_galley(
        ui,
        description,
        egui::FontId::proportional(crate::ui::theme::META_SIZE),
        colors.muted,
        text_width,
    );
    painter.galley(
        egui::pos2(rect.left() + 38.0, rect.top() + 30.0),
        galley,
        colors.muted,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("{title}. {description}"),
        )
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.label(
        RichText::new(text.to_uppercase())
            .size(crate::ui::theme::META_SIZE)
            .strong()
            .color(colors.muted),
    );
    ui.add_space(4.0);
}

/// App icon, name and a one-line description.
fn header(ui: &mut egui::Ui, icon_texture: Option<&egui::TextureHandle>) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        if let Some(texture) = icon_texture {
            ui.image((texture.id(), egui::vec2(ICON_SIZE, ICON_SIZE)));
        }
        ui.vertical(|ui| {
            ui.label(
                RichText::new("Lumen")
                    .size(22.0)
                    .strong()
                    .color(colors.text),
            );
            let tagline = "Edit code, run tasks, commit and ask a coding agent in one window.";
            ui.add(egui::Label::new(RichText::new(tagline).color(colors.muted)).wrap());
        });
    });
}

/// The Start column; returns the command of the entry clicked.
fn start_links(ui: &mut egui::Ui) -> Option<Command> {
    heading(ui, "Start");
    let entries = [
        (
            icon::FILE_PLUS,
            "New Text File",
            Some("N"),
            Command::NewUntitled,
        ),
        (
            icon::FILE_ARROW_UP,
            "Open File",
            Some("O"),
            Command::OpenFile,
        ),
        (icon::FOLDER_OPEN, "Open Folder", None, Command::OpenFolder),
        (
            icon::GIT_BRANCH,
            "Clone Repository…",
            None,
            Command::OpenCloneDialog,
        ),
    ];
    let mut chosen = None;
    for (glyph, label, shortcut, command) in entries {
        let shortcut = shortcut.map(super::shortcut::primary);
        if link_row(ui, &format!("{glyph}  {label}"), shortcut.as_deref(), true).clicked() {
            chosen = Some(command);
        }
    }
    chosen
}

struct Entry {
    glyph: &'static str,
    title: &'static str,
    shortcut: Option<String>,
    description: &'static str,
    command: Command,
}

fn feature_entries(agents: bool) -> Vec<Entry> {
    let mut entries = vec![Entry {
        glyph: icon::MAGNIFYING_GLASS,
        title: "Command Palette",
        shortcut: Some(super::shortcut::primary("Shift+P")),
        description: "Every command, searchable",
        command: Command::Palette,
    }];
    if agents {
        entries.push(Entry {
            glyph: icon::AGENT,
            title: "Coding Agents",
            shortcut: Some(super::shortcut::primary("Shift+A")),
            description: "Ask Codex or Claude Code about this folder",
            command: Command::ToggleAgentPanel,
        });
    }
    entries.push(Entry {
        glyph: icon::PLAY,
        title: "Tasks",
        shortcut: None,
        description: "Pick a task in the title bar and press Run",
        command: Command::ActivateActivity(Activity::RunDebug),
    });
    entries.push(Entry {
        glyph: icon::GIT_BRANCH,
        title: "Source Control",
        shortcut: None,
        description: "Stage, commit, push and browse history",
        command: Command::ActivateActivity(Activity::SourceControl),
    });
    entries
}

/// The feature tiles, two to a row when there is room.
fn feature_tiles(ui: &mut egui::Ui, per_row: usize, agents: bool) -> Option<Command> {
    heading(ui, "Get to know Lumen");
    let entries = feature_entries(agents);
    let mut chosen = None;
    for row in entries.chunks(per_row) {
        ui.columns(per_row, |columns| {
            for (column, entry) in columns.iter_mut().zip(row) {
                let shortcut = entry.shortcut.as_deref();
                if tile(
                    column,
                    entry.glyph,
                    entry.title,
                    shortcut,
                    entry.description,
                )
                .clicked()
                {
                    chosen = Some(entry.command.clone());
                }
            }
        });
        ui.add_space(8.0);
    }
    chosen
}

impl Workbench<'_> {
    pub(super) fn welcome_page(&mut self, ui: &mut egui::Ui) {
        let icon_texture = app_icon(ui.ctx());
        let two_columns = ui.available_width() >= TWO_COLUMNS_FROM;
        let agents = self.view.settings.boolean("agent.enabled");
        let mut chosen: Option<Command> = None;
        egui::ScrollArea::vertical()
            .id_salt("welcome_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let content = (ui.available_width() - 2.0 * MARGIN).clamp(160.0, MAX_WIDTH);
                let pad = ((ui.available_width() - content) / 2.0).max(0.0);
                ui.add_space(super::widgets::empty_state_top(ui).min(48.0));
                ui.horizontal_top(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(content);
                        header(ui, icon_texture.as_ref());
                        ui.add_space(22.0);
                        if two_columns {
                            ui.columns(2, |columns| {
                                chosen = start_links(&mut columns[0]).or(chosen.take());
                                heading(&mut columns[1], "Recent");
                                self.recent_files_list(&mut columns[1]);
                            });
                        } else {
                            chosen = start_links(ui).or(chosen.take());
                            ui.add_space(14.0);
                            heading(ui, "Recent");
                            self.recent_files_list(ui);
                        }
                        ui.add_space(24.0);
                        let per_row = if two_columns { 2 } else { 1 };
                        chosen = feature_tiles(ui, per_row, agents).or(chosen.take());
                        ui.add_space(16.0);
                    });
                });
            });
        if let Some(command) = chosen {
            self.run_welcome_command(command);
        }
    }

    /// Activating the activity that is already showing would hide it.
    fn run_welcome_command(&mut self, command: Command) {
        let already_showing = matches!(
            &command,
            Command::ActivateActivity(activity)
                if self.view.sidebar_visible && self.view.activity == *activity
        );
        if !already_showing {
            self.command(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LumenApp;
    use crate::ui::theme::configure_ui;

    fn render(
        app: &mut LumenApp,
        ctx: &egui::Context,
        width: f32,
        height: f32,
    ) -> egui::FullOutput {
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.editor(ctx)),
            ));
        }
        output.unwrap()
    }

    fn rect_of(output: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.job.text.contains(text) => {
                Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
            }
            _ => None,
        })
    }

    #[test]
    fn the_welcome_page_offers_start_actions_and_feature_entry_points() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = LumenApp::default();
        let output = render(&mut app, &ctx, 900.0, 700.0);
        for text in [
            "Lumen",
            "New Text File",
            "Open File",
            "Open Folder",
            "Clone Repository",
            "Command Palette",
            "Coding Agents",
            "Tasks",
            "Source Control",
        ] {
            assert!(rect_of(&output, text).is_some(), "{text} is on the page");
        }
    }

    #[test]
    fn entries_dispatch_their_existing_commands() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = LumenApp::default();
        let output = render(&mut app, &ctx, 900.0, 700.0);
        let target = rect_of(&output, "Source Control").unwrap().center();
        for pressed in [true, false] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 700.0),
                    )),
                    events: vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton {
                            pos: target,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.editor(ctx)),
            );
        }
        assert_eq!(app.view.activity, Activity::SourceControl);
        assert!(app.view.sidebar_visible);
    }

    #[test]
    fn the_agents_entry_is_hidden_when_agents_are_turned_off() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = LumenApp::default();
        app.view.settings.user.persisted.insert(
            "agent.enabled".into(),
            lumen_settings_core::Value::Bool(false),
        );
        let output = render(&mut app, &ctx, 900.0, 700.0);
        assert!(rect_of(&output, "Coding Agents").is_none());
        assert!(rect_of(&output, "Command Palette").is_some());
    }

    #[test]
    fn the_page_fits_narrow_and_short_groups_without_overflowing_sideways() {
        for (width, height) in [(220.0_f32, 700.0_f32), (300.0, 300.0), (900.0, 220.0)] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut app = LumenApp::default();
            let output = render(&mut app, &ctx, width, height);
            for clipped in &output.shapes {
                if let egui::Shape::Text(text) = &clipped.shape {
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    assert!(
                        rect.right() <= width + 1.0,
                        "{width}x{height}: {:?} ends at {}",
                        text.galley.job.text,
                        rect.right()
                    );
                }
            }
            assert!(
                rect_of(&output, "New Text File").is_some(),
                "{width}x{height}"
            );
        }
    }
}
