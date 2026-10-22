//! Clickable path breadcrumbs above the editor text.
use super::Workbench;
use crate::{
    app::command::Command,
    icon,
    model::{breadcrumbs, previewable::AlternateView},
};
use eframe::egui::{self, FontId, RichText};

const FONT_SIZE: f32 = 11.0;
const LEFT_INSET: f32 = 10.0;
/// Gap between the floating Preview button and the pane edges; the right
/// margin also clears the editor's vertical scrollbar.
const PREVIEW_MARGIN: egui::Vec2 = egui::vec2(18.0, 8.0);
const PREVIEW_GLYPH_SIZE: f32 = 15.0;

impl Workbench<'_> {
    /// Directories reveal themselves in the Explorer; the file opens the symbol
    /// picker when a language server can provide one. Leading segments collapse
    /// into an ellipsis before the file name ever truncates.
    pub(super) fn breadcrumb_bar(&mut self, ui: &mut egui::Ui, index: usize) {
        let colors = crate::ui::theme::palette(ui.visuals());
        // Below the tab strip: where this pane's content begins.
        let content_top = ui.cursor().top();
        let file = &self.editor.files[index];
        let views = file.alternate_views();
        let document = file.id;
        let crumbs = breadcrumbs::crumbs(
            self.editor.workspace.as_deref(),
            file.path.as_deref(),
            &file.name,
        );
        let font = FontId::proportional(FONT_SIZE);
        let measure = |ui: &egui::Ui, text: &str| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), font.clone(), colors.text)
                    .size()
                    .x
            })
        };
        let padding = 2.0 * ui.spacing().button_padding.x;
        let widths: Vec<f32> = crumbs
            .iter()
            .map(|crumb| measure(ui, &crumb.label) + padding)
            .collect();
        let separator = measure(ui, icon::CARET_RIGHT) + 4.0;
        let ellipsis = measure(ui, "…") + padding;
        let start = breadcrumbs::visible_start(
            &widths,
            separator,
            ellipsis,
            (ui.available_width() - 2.0 * LEFT_INSET).max(0.0),
        );
        let symbols_reason = self.palette_disabled_reason(&Command::ShowDocumentSymbols);
        let mut command = None;
        ui.horizontal(|ui| {
            ui.add_space(LEFT_INSET);
            ui.spacing_mut().item_spacing.x = 2.0;
            let caret = |ui: &mut egui::Ui| {
                ui.label(
                    RichText::new(icon::CARET_RIGHT)
                        .size(FONT_SIZE)
                        .color(colors.muted),
                );
            };
            if start > 0 {
                let hidden: Vec<&str> = crumbs[..start]
                    .iter()
                    .map(|crumb| crumb.label.as_str())
                    .collect();
                ui.label(RichText::new("…").size(FONT_SIZE).color(colors.muted))
                    .on_hover_text(hidden.join(" › "));
                caret(ui);
            }
            for (position, crumb) in crumbs.iter().enumerate().skip(start) {
                if position > start {
                    caret(ui);
                }
                let last = position + 1 == crumbs.len();
                let text = RichText::new(&crumb.label).size(FONT_SIZE).color(if last {
                    colors.text
                } else {
                    colors.muted
                });
                if crumb.is_file {
                    let enabled = symbols_reason.is_none();
                    let response = ui.add_enabled(enabled, egui::Button::new(text).frame(false));
                    let response = if enabled {
                        response.on_hover_text("Go to Symbol in Editor")
                    } else {
                        response.on_disabled_hover_text(crumb.path.as_ref().map_or_else(
                            || symbols_reason.unwrap_or_default().to_owned(),
                            |path| path.display().to_string(),
                        ))
                    };
                    if response.clicked() {
                        command = Some(Command::ShowDocumentSymbols);
                    }
                } else if let Some(path) = &crumb.path {
                    let response = ui
                        .add(egui::Button::new(text).frame(false))
                        .on_hover_text(format!("Reveal {} in Explorer", path.display()));
                    if response.clicked() {
                        command = Some(Command::RevealInExplorer(path.clone()));
                    }
                }
            }
        });
        for (slot, view) in views.into_iter().enumerate() {
            if self.view_button(ui, document, view, slot, content_top) {
                command = Some(match view {
                    AlternateView::Preview(_) => Command::OpenPreview(document),
                    AlternateView::Visual(_) => Command::OpenVisualEditor(document),
                });
            }
        }
        if let Some(command) = command {
            self.command(command);
        }
    }

    /// Floating alternate-view button at the pane's top-right corner, sized like
    /// the workbench panel toggles; further views stack to its left. Returns
    /// whether it was clicked.
    fn view_button(
        &self,
        ui: &egui::Ui,
        document: crate::model::DocumentId,
        view: AlternateView,
        slot: usize,
        content_top: f32,
    ) -> bool {
        let (glyph, tooltip) = match view {
            AlternateView::Preview(_) => (icon::PREVIEW, "Open Preview (Cmd/Ctrl+Shift+V)"),
            AlternateView::Visual(_) => (icon::TABLE, "Open Visual Editor"),
        };
        let colors = crate::ui::theme::palette(ui.visuals());
        let (toggle_width, toggle_height) = super::task_launcher::TOGGLE_SIZE;
        // Padding around the glyph so the bordered button reads as a control.
        let (width, height) = (toggle_width + 6.0, toggle_height + 4.0);
        let pane = ui.max_rect();
        let position = egui::pos2(
            pane.right() - PREVIEW_MARGIN.x - width - slot as f32 * (width + 4.0),
            content_top + PREVIEW_MARGIN.y,
        );
        egui::Area::new(egui::Id::new(("view-button", document, slot)))
            .order(egui::Order::Middle)
            .fixed_pos(position)
            .show(ui.ctx(), |ui| {
                let response = ui.add(
                    egui::Button::new(
                        RichText::new(glyph)
                            .size(PREVIEW_GLYPH_SIZE)
                            .color(colors.text),
                    )
                    .fill(colors.input)
                    .stroke(egui::Stroke::new(1.0_f32, colors.float_border))
                    .corner_radius(3)
                    .min_size(egui::vec2(width, height)),
                );
                response.on_hover_text(tooltip).clicked()
            })
            .inner
    }
}

#[cfg(test)]
mod tests {
    use crate::{app::LumenApp, app::view_state::ExplorerSelection, model::Document};
    use eframe::egui;
    use std::path::PathBuf;

    struct Fixture {
        app: LumenApp,
        ctx: egui::Context,
        root: PathBuf,
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("lumen-crumbs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/ui")).unwrap();
        std::fs::write(root.join("src/ui/a.rs"), "fn a() {}\n").unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.clone());
        app.editor.files.push(Document::from_path(
            root.join("src/ui/a.rs"),
            "fn a() {}\n".into(),
        ));
        app.editor.set_active_index(Some(0));
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        Fixture { app, ctx, root }
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
            |ctx| app.render_component(ctx, |workbench| workbench.editor(ctx)),
        )
    }

    fn settle(f: &mut Fixture, width: f32) -> egui::FullOutput {
        let mut output = frame(&mut f.app, &f.ctx, width, vec![]);
        for _ in 0..3 {
            output = frame(&mut f.app, &f.ctx, width, vec![]);
        }
        output
    }

    fn text_rect(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
    }

    #[test]
    fn the_header_shows_the_relative_path_and_a_directory_click_reveals_it() {
        let mut f = fixture("click");
        let output = settle(&mut f, 1000.0);
        for crumb in ["src", "ui", "a.rs"] {
            assert!(text_rect(&output, crumb).is_some(), "missing {crumb}");
        }
        let target = text_rect(&output, "ui").unwrap().center();
        for pressed in [true, false] {
            frame(
                &mut f.app,
                &f.ctx,
                1000.0,
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(f.app.view.expanded_dirs.contains(&f.root.join("src")));
        assert!(f.app.view.expanded_dirs.contains(&f.root.join("src/ui")));
        assert!(matches!(
            &f.app.view.explorer_selection,
            Some(ExplorerSelection::Directory(path)) if path == &f.root.join("src/ui")
        ));
        assert_eq!(f.app.editor.files[0].text, "fn a() {}\n", "no edits");
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn narrow_headers_collapse_leading_segments_but_keep_the_file_name() {
        let mut f = fixture("narrow");
        let output = settle(&mut f, 150.0);
        assert!(text_rect(&output, "a.rs").is_some(), "file name stays");
        assert!(
            text_rect(&output, "…").is_some(),
            "leading segments collapse"
        );
        let file = text_rect(&output, "a.rs").unwrap();
        assert!(file.right() <= 150.0, "{file:?} escapes the pane");
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn untitled_documents_show_just_their_name() {
        let mut app = LumenApp::default();
        app.editor.new_file();
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let mut output = frame(&mut app, &ctx, 800.0, vec![]);
        for _ in 0..3 {
            output = frame(&mut app, &ctx, 800.0, vec![]);
        }
        let name = app.editor.files[0].name.clone();
        assert!(text_rect(&output, &name).is_some());
        assert!(text_rect(&output, "…").is_none());
    }

    #[test]
    fn reveal_expands_ancestors_and_refuses_outside_paths() {
        let mut f = fixture("reveal");
        let file = f.root.join("src/ui/a.rs");
        f.app.reveal_in_explorer(file.clone());
        assert!(f.app.view.expanded_dirs.contains(&f.root.join("src/ui")));
        assert!(matches!(
            &f.app.view.explorer_selection,
            Some(ExplorerSelection::File(path)) if path == &file
        ));
        let before = f.app.view.expanded_dirs.clone();
        f.app.view.explorer_selection = None;
        f.app
            .reveal_in_explorer(std::env::temp_dir().join("elsewhere"));
        assert_eq!(f.app.view.expanded_dirs, before);
        assert!(f.app.view.explorer_selection.is_none());
        assert!(f.app.view.status.contains("outside"));
        f.app.editor.workspace = None;
        f.app.reveal_in_explorer(file);
        assert!(f.app.view.status.contains("Open a folder"));
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn welcome_offers_new_file_and_recent_files_that_fail_safely_when_missing() {
        let mut app = LumenApp::default();
        let missing = std::env::temp_dir().join("lumen-definitely-missing/recent.rs");
        app.view.navigation.recent = vec![missing.clone()];
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let mut output = frame(&mut app, &ctx, 900.0, vec![]);
        for _ in 0..3 {
            output = frame(&mut app, &ctx, 900.0, vec![]);
        }
        assert!(text_rect(&output, "Recent files").is_some());
        let recent = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.starts_with("recent.rs") => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .expect("recent entry");
        let new_file = text_rect(
            &output,
            &format!("{}  New Text File", crate::icon::FILE_PLUS),
        )
        .expect("New Text File button");
        for target in [recent.center(), new_file.center()] {
            for pressed in [true, false] {
                frame(
                    &mut app,
                    &ctx,
                    900.0,
                    vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton {
                            pos: target,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            if target == recent.center() {
                assert!(app.editor.files.is_empty(), "a missing file opens nothing");
                assert!(!app.view.status.is_empty());
            }
        }
        assert_eq!(
            app.editor.files.len(),
            1,
            "New Text File opened one untitled buffer"
        );
        assert!(app.editor.files[0].path.is_none());
    }

    fn click_at(app: &mut LumenApp, ctx: &egui::Context, target: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                900.0,
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    fn welcome(app: &mut LumenApp, ctx: &egui::Context) -> egui::FullOutput {
        let mut output = frame(app, ctx, 900.0, vec![]);
        for _ in 0..3 {
            output = frame(app, ctx, 900.0, vec![]);
        }
        output
    }

    fn entry(output: &egui::FullOutput, prefix: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.starts_with(prefix) => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no entry starting with {prefix:?}"))
    }

    #[test]
    fn welcome_lists_recent_folders_and_opening_one_makes_it_the_workspace() {
        let root =
            std::env::temp_dir().join(format!("lumen-welcome-folder-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = LumenApp::default();
        app.view.navigation.recent_folders = vec![root.clone()];
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let output = welcome(&mut app, &ctx);
        assert!(text_rect(&output, "Recent folders").is_some());
        let name = root.file_name().unwrap().to_string_lossy().into_owned();
        click_at(&mut app, &ctx, entry(&output, &name).center());
        assert_eq!(app.editor.workspace.as_deref(), Some(root.as_path()));
        assert_eq!(app.view.navigation.recent_folders, vec![root.clone()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_recent_folder_is_pruned_without_touching_the_workspace() {
        let missing = std::env::temp_dir().join("lumen-no-such-folder/gone");
        let mut app = LumenApp::default();
        app.view.navigation.recent_folders = vec![missing];
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let output = welcome(&mut app, &ctx);
        click_at(&mut app, &ctx, entry(&output, "gone").center());
        assert!(app.editor.workspace.is_none());
        assert!(app.view.navigation.recent_folders.is_empty());
        assert!(app.view.status.starts_with("Folder not found"));
    }
}
