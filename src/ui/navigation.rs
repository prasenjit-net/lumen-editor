//! Compact Quick Open and Go to Line overlays.
use super::Workbench;
use crate::{
    app::{
        command::Command,
        view_state::{NavigationMode, NavigationView},
    },
    icon,
};
use eframe::egui::{self, CornerRadius, Frame, RichText, Stroke, TextEdit, Vec2};

impl Workbench<'_> {
    pub(crate) fn navigation_palette(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(mode) = self.view.navigation.mode else {
            return;
        };
        let root = self.editor.workspace.as_deref();
        let document = self.editor.active().map(|file| file.id);
        let width = (ctx.content_rect().width() - 24.0).clamp(240.0, 560.0);
        let mut open = true;
        let mut action = None;
        egui::Window::new(match mode {
            NavigationMode::QuickOpen => "Go to File",
            NavigationMode::GoToLine => "Go to Line/Column",
        })
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(width)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 48.0))
            .frame(
                Frame::window(&ctx.style())
                    .fill(colors.panel)
                    .corner_radius(CornerRadius::same(6))
                    .stroke(Stroke::new(1.0_f32, colors.border)),
            )
            .show(ctx, |ui| {
                ui.set_width(width - 16.0);
                let nav = &mut self.view.navigation;
                if ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                }) {
                    nav.mode = None;
                    return;
                }
                let mut selection_moved = false;
                if mode == NavigationMode::QuickOpen {
                    if ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                    }) {
                        nav.selected = (nav.selected + 1).min(nav.matches.len().saturating_sub(1));
                        selection_moved = true;
                    }
                    if ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                    }) {
                        nav.selected = nav.selected.saturating_sub(1);
                        selection_moved = true;
                    }
                }
                let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                let response = ui.add(
                    TextEdit::singleline(&mut nav.query)
                        .id(egui::Id::new("navigation_query"))
                        .hint_text(match mode {
                            NavigationMode::QuickOpen => "Search files by name or path",
                            NavigationMode::GoToLine => "Line[:Column]",
                        })
                        .desired_width(f32::INFINITY),
                );
                if nav.focus {
                    response.request_focus();
                    nav.focus = false;
                }
                if response.changed() {
                    nav.selected = 0;
                    nav.error = None;
                }
                match mode {
                    NavigationMode::QuickOpen => {
                        action = show_quick_results(ui, nav, root, enter, selection_moved);
                    }
                    NavigationMode::GoToLine => {
                        ui.label(
                            RichText::new("Line and column are 1-based; out-of-range values stop at the document end.")
                                .size(11.0)
                                .color(colors.muted),
                        );
                        if enter {
                            if let Some(document) = document {
                                action = Some(Command::NavigateTo(document, nav.query.clone()));
                            } else {
                                nav.mode = None;
                            }
                        }
                    }
                }
                if let Some(error) = &nav.error {
                    ui.label(RichText::new(error).color(colors.error));
                }
            });
        if !open {
            self.view.navigation.mode = None;
        }
        if self.view.navigation.mode.is_none() && self.editor.active().is_some() {
            self.view.focus_editor = true;
        }
        if let Some(action) = action {
            self.command(action);
        }
    }
}

fn show_quick_results(
    ui: &mut egui::Ui,
    nav: &mut NavigationView,
    root: Option<&std::path::Path>,
    enter: bool,
    selection_moved: bool,
) -> Option<Command> {
    let colors = crate::ui::theme::palette(ui.visuals());
    nav.refresh(root);
    let mut action = nav
        .matches
        .get(nav.selected)
        .cloned()
        .filter(|_| enter)
        .map(Command::OpenQuickPath);
    ui.add_space(4.0);
    let hint = if root.is_none() {
        "Open a folder to search workspace files; recent files are shown below"
    } else if nav.running {
        "Indexing workspace…"
    } else if nav.limited {
        "Showing the first 100,000 indexed files"
    } else {
        "Recent files and workspace matches"
    };
    ui.label(RichText::new(hint).size(11.0).color(colors.muted));
    if nav.errors > 0 {
        ui.label(
            RichText::new(format!("{} paths could not be indexed", nav.errors))
                .size(11.0)
                .color(colors.muted),
        );
    }
    if let Some(warning) = &nav.recent_warning {
        ui.label(RichText::new(warning).size(11.0).color(colors.muted));
    }
    egui::ScrollArea::vertical()
        .max_height(330.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, path) in nav.matches.iter().enumerate() {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default();
                let relative = root
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(path);
                let directory = relative
                    .parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_default();
                let label = format!("{}  {}    {}", icon::FILE, name, directory);
                let row = quick_result_row(ui, label, index == nav.selected);
                if index == nav.selected && selection_moved {
                    row.scroll_to_me(Some(egui::Align::Center));
                }
                if row.on_hover_text(path.display().to_string()).clicked() {
                    action = Some(Command::OpenQuickPath(path.clone()));
                }
            }
        });
    if nav.matches.is_empty() && !nav.running {
        ui.label(RichText::new("No matching files").color(colors.muted));
    }
    action
}

fn quick_result_row(ui: &mut egui::Ui, label: String, selected: bool) -> egui::Response {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.add_sized(
        [ui.available_width(), 22.0],
        egui::Button::new((RichText::new(label).color(colors.text), egui::Atom::grow()))
            .selected(selected)
            .truncate(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::LumenApp, ui::theme::configure_ui};

    fn text_position(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Pos2> {
        fn find(shape: &egui::Shape, needle: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().contains(needle) => Some(text.pos),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, needle)),
                _ => None,
            }
        }
        shapes.iter().find_map(|shape| find(&shape.shape, needle))
    }

    #[test]
    fn quick_open_result_text_starts_at_left_edge_of_full_width_row() {
        for width in [320.0, 900.0] {
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            let mut row_rect = egui::Rect::NOTHING;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 200.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        row_rect = quick_result_row(ui, "sample.rs".into(), true).rect;
                    });
                },
            );
            let text = text_position(&output.shapes, "sample.rs").expect("result text painted");
            assert!(row_rect.width() > width - 20.0);
            assert!(text.x >= row_rect.left());
            assert!(text.x < row_rect.left() + 16.0);
        }
    }

    fn frame(
        app: &mut LumenApp,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.navigation_palette(ctx)),
        )
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn quick_open_keyboard_activates_selected_file_at_narrow_and_wide_widths() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.rs");
        let second = temp.path().join("second.rs");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        for width in [320.0, 900.0] {
            let mut app = LumenApp::default();
            app.view.navigation.mode = Some(NavigationMode::QuickOpen);
            app.view.navigation.focus = true;
            app.view.navigation.recent = vec![first.clone(), second.clone()];
            let ctx = egui::Context::default();
            configure_ui(&ctx);
            frame(&mut app, &ctx, width, vec![]);
            let output = frame(&mut app, &ctx, width, vec![]);
            assert!(app.view.navigation.matches.contains(&first));
            assert!(!output.shapes.is_empty());
            frame(&mut app, &ctx, width, vec![key(egui::Key::ArrowDown)]);
            frame(&mut app, &ctx, width, vec![key(egui::Key::Enter)]);
            assert_eq!(
                app.editor.active().and_then(|file| file.path.as_ref()),
                Some(&second)
            );
            assert_eq!(app.view.navigation.mode, None);
        }
    }

    #[test]
    fn go_to_line_input_submits_without_editing_document() {
        let mut app = LumenApp::default();
        app.editor.new_file();
        app.editor.files[0].text = "first\nsecond".into();
        let document = app.editor.files[0].id;
        app.view.navigation.mode = Some(NavigationMode::GoToLine);
        app.view.navigation.focus = true;
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        frame(&mut app, &ctx, 400.0, vec![]);
        frame(&mut app, &ctx, 400.0, vec![]);
        frame(&mut app, &ctx, 400.0, vec![egui::Event::Text("2:2".into())]);
        frame(&mut app, &ctx, 400.0, vec![key(egui::Key::Enter)]);
        assert_eq!(app.view.pending_goto, Some((document, "first\ns".len())));
        assert_eq!(app.editor.files[0].text, "first\nsecond");
        assert_eq!(app.view.navigation.mode, None);
    }

    #[test]
    fn escape_cancels_navigation_and_returns_focus_to_editor() {
        let mut app = LumenApp::default();
        app.editor.new_file();
        app.view.navigation.mode = Some(NavigationMode::QuickOpen);
        app.view.navigation.focus = true;
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        frame(&mut app, &ctx, 400.0, vec![]);
        frame(&mut app, &ctx, 400.0, vec![]);
        frame(&mut app, &ctx, 400.0, vec![key(egui::Key::Escape)]);
        assert_eq!(app.view.navigation.mode, None);
        assert!(app.view.focus_editor);
        assert_eq!(app.editor.files.len(), 1);
        assert!(!app.editor.files[0].dirty);
    }
}
