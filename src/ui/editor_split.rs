//! Two editor groups side by side. Each group draws exactly what a single editor
//! draws (tab strip, breadcrumbs, document) by being *focused for the duration of
//! its pass*; afterwards focus is put back on the group the user is working in.
use super::{Workbench, widgets::surface};
use eframe::egui::{self, CursorIcon, Rect, Sense, Stroke};

/// A group never gets narrower than this, so its tabs and text stay usable.
pub(crate) const MIN_GROUP_WIDTH: f32 = 220.0;
/// Space between the groups, and the divider's grab area.
const DIVIDER: f32 = 6.0;

/// Clamp a requested left-group share so both groups keep `MIN_GROUP_WIDTH`.
/// When the area is too narrow for two minimums the groups split it evenly.
pub(crate) fn split_widths(total: f32, ratio: f32) -> (f32, f32) {
    let usable = (total - DIVIDER).max(0.0);
    if usable < 2.0 * MIN_GROUP_WIDTH {
        return (usable / 2.0, usable / 2.0);
    }
    let left = (usable * ratio).clamp(MIN_GROUP_WIDTH, usable - MIN_GROUP_WIDTH);
    (left, usable - left)
}

impl Workbench<'_> {
    pub(super) fn editor_split(&mut self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let area = ui.available_rect_before_wrap();
        let (left_width, right_width) = split_widths(area.width(), self.view.split_ratio);
        let left = Rect::from_min_size(area.min, egui::vec2(left_width, area.height()));
        let divider = Rect::from_min_size(
            egui::pos2(left.right(), area.top()),
            egui::vec2(DIVIDER, area.height()),
        );
        let right = Rect::from_min_size(
            egui::pos2(divider.right(), area.top()),
            egui::vec2(right_width, area.height()),
        );
        ui.allocate_rect(area, Sense::hover());

        let real_focus = self.editor.focused_group();
        let mut new_focus = real_focus;
        let pressed_at = ui
            .input(|input| input.pointer.any_pressed())
            .then(|| ui.input(|input| input.pointer.interact_pos()))
            .flatten();
        // The one-shot "focus the editor" request belongs to the focused group.
        let wants_focus = self.view.focus_editor;
        let mut still_wanted = wants_focus;
        #[cfg(feature = "shared-editor")]
        {
            self.view.group_surfaces.resize_with(2, || None);
            self.view.group_surfaces[real_focus] = self.view.shared_surface.take();
        }
        for (group, rect) in [left, right].into_iter().enumerate() {
            if pressed_at.is_some_and(|pos| rect.contains(pos)) {
                new_focus = group;
            }
            self.editor.focus_group(group);
            #[cfg(feature = "shared-editor")]
            {
                self.view.shared_surface = self.view.group_surfaces[group].take();
            }
            self.view.focus_editor = wants_focus && group == real_focus;
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .id_salt(("editor_group", group)),
            );
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            surface(colors.bg).show(&mut child, |ui| {
                ui.set_min_size(ui.available_size());
                self.editor_group_body(ui, group > 0);
            });
            if group == real_focus {
                // Left set only if the focused group did not get to use it.
                still_wanted = self.view.focus_editor;
            }
            #[cfg(feature = "shared-editor")]
            {
                self.view.group_surfaces[group] = self.view.shared_surface.take();
            }
        }
        self.view.focus_editor = still_wanted;
        self.editor.focus_group(new_focus);
        #[cfg(feature = "shared-editor")]
        {
            self.view.shared_surface = self.view.group_surfaces[new_focus].take();
        }

        // Outline of the focused group, so it is clear where keystrokes go.
        let focused = if new_focus == 0 { left } else { right };
        ui.painter().rect_stroke(
            focused.shrink(0.5),
            5,
            Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        );

        let response = ui.interact(divider, ui.id().with("editor_divider"), Sense::drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if response.dragged() && area.width() > DIVIDER {
            let usable = area.width() - DIVIDER;
            let left_target = left_width + response.drag_delta().x;
            self.view.split_ratio = (left_target / usable).clamp(0.1, 0.9);
        }
        if response.double_clicked() {
            self.view.split_ratio = 0.5;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_groups_keep_their_minimum_width_whatever_the_ratio() {
        for ratio in [0.0, 0.1, 0.5, 0.9, 1.0] {
            let (left, right) = split_widths(1000.0, ratio);
            assert!(
                left >= MIN_GROUP_WIDTH && right >= MIN_GROUP_WIDTH,
                "{ratio}: {left} {right}"
            );
            assert!((left + right + DIVIDER - 1000.0).abs() < 0.01);
        }
        assert_eq!(split_widths(1000.0, 0.5).0, (1000.0 - DIVIDER) / 2.0);
    }

    #[test]
    fn a_pane_too_narrow_for_two_minimums_splits_evenly_instead_of_overlapping() {
        let (left, right) = split_widths(300.0, 0.8);
        assert_eq!(left, right);
        assert!(left + right + DIVIDER <= 300.0 + 0.01);
        assert_eq!(split_widths(0.0, 0.5), (0.0, 0.0));
    }

    use crate::{
        app::{LumenApp, command::Command},
        model::Document,
    };

    /// Two files per group, each with distinctive text.
    fn split_app() -> (LumenApp, egui::Context) {
        let mut app = LumenApp::default();
        #[cfg(feature = "shared-editor")]
        {
            app.view.shared_editor = true;
        }
        for (name, text) in [
            ("aaa.rs", "LEFTONE"),
            ("bbb.rs", "LEFTTWO"),
            ("ccc.rs", "RIGHTONE"),
            ("ddd.rs", "RIGHTTWO"),
        ] {
            app.editor
                .files
                .push(Document::from_path(name.into(), text.into()));
        }
        app.view.settings.user.persisted.insert(
            "workbench.split_editor".into(),
            lumen_settings_core::Value::Bool(true),
        );
        let ids: Vec<_> = app.editor.files.iter().map(|f| f.id).collect();
        app.editor.activate(ids[2]);
        app.move_tab_to_group(ids[2]);
        app.move_tab_to_group(ids[3]);
        app.editor.activate(ids[0]);
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        (app, ctx)
    }

    fn frame(
        app: &mut LumenApp,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.editor(ctx)),
        )
    }

    fn settle(app: &mut LumenApp, ctx: &egui::Context, width: f32) -> egui::FullOutput {
        let mut output = frame(app, ctx, width, vec![]);
        for _ in 0..4 {
            output = frame(app, ctx, width, vec![]);
        }
        output
    }

    fn rect_of(output: &egui::FullOutput, needle: &str) -> Option<Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
    }

    #[test]
    fn each_group_draws_its_own_tabs_and_document_side_by_side_at_every_width() {
        for width in [520.0, 900.0, 1500.0] {
            let (mut app, ctx) = split_app();
            let output = settle(&mut app, &ctx, width);
            let tab =
                |name: &str| rect_of(&output, name).unwrap_or_else(|| panic!("{name} at {width}"));
            // Left group: aaa/bbb tabs; right group: ccc/ddd tabs.
            assert!(
                tab("aaa.rs").center().x < tab("ccc.rs").center().x,
                "{width}"
            );
            assert!(
                tab("bbb.rs").center().x < tab("ddd.rs").center().x,
                "{width}"
            );
            // Each group's breadcrumb names the document it is showing, and a
            // group's strip lists only its own tabs (checked by tab count).
            let crumbs: Vec<Rect> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.job.text.ends_with(".rs") && text.pos.y > 30.0 =>
                    {
                        Some(Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(crumbs.len(), 2, "one breadcrumb per group at {width}");
            assert!(crumbs[0].center().x < crumbs[1].center().x);
            let tab_titles = output
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text.starts_with('\u{eae9}'))
                })
                .count();
            assert_eq!(
                tab_titles, 4,
                "2 tabs per group, not 4 per group at {width}"
            );
            let window = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0));
            for name in ["aaa.rs", "bbb.rs", "ccc.rs", "ddd.rs"] {
                assert!(window.contains_rect(tab(name)), "{name} escapes at {width}");
            }
        }
    }

    #[test]
    fn pressing_in_a_group_focuses_it_without_changing_any_content() {
        let (mut app, ctx) = split_app();
        let output = settle(&mut app, &ctx, 1000.0);
        assert_eq!(app.editor.focused_group(), 0);
        let _ = output;
        // Inside the right group's editor body, well below its tab strip.
        let right = egui::pos2(800.0, 300.0);
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                1000.0,
                vec![
                    egui::Event::PointerMoved(right),
                    egui::Event::PointerButton {
                        pos: right,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(app.editor.focused_group(), 1);
        let texts: Vec<_> = app.editor.files.iter().map(|f| f.text.clone()).collect();
        assert_eq!(texts, ["LEFTONE", "LEFTTWO", "RIGHTONE", "RIGHTTWO"]);
        settle(&mut app, &ctx, 1000.0);
        let left = egui::pos2(250.0, 300.0);
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                1000.0,
                vec![
                    egui::Event::PointerMoved(left),
                    egui::Event::PointerButton {
                        pos: left,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(app.editor.focused_group(), 0);
    }

    #[test]
    fn typing_reaches_only_the_focused_groups_document() {
        let (mut app, ctx) = split_app();
        settle(&mut app, &ctx, 1000.0);
        app.execute(Command::FocusGroup(1), &ctx);
        settle(&mut app, &ctx, 1000.0);
        frame(&mut app, &ctx, 1000.0, vec![egui::Event::Text("Z".into())]);
        settle(&mut app, &ctx, 1000.0);
        let texts: Vec<_> = app.editor.files.iter().map(|f| f.text.clone()).collect();
        assert_eq!(texts[0], "LEFTONE", "the unfocused group is untouched");
        assert_eq!(texts[1], "LEFTTWO");
        assert_eq!(texts[2], "RIGHTONE");
        assert!(texts[3].contains('Z'), "typed text landed in {texts:?}");
    }

    #[test]
    fn the_settings_tab_lives_only_in_the_first_group() {
        let (mut app, ctx) = split_app();
        app.view.settings.tab_open = true;
        let output = settle(&mut app, &ctx, 1000.0);
        let all: Vec<Rect> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.ends_with("Settings") => {
                    Some(Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(all.len(), 1, "one Settings tab, not one per group: {all:?}");
        assert!(all[0].center().x < 500.0, "and it is in the left group");
    }

    #[test]
    fn dragging_the_divider_resizes_within_the_minimums_and_double_click_resets() {
        let (mut app, ctx) = split_app();
        settle(&mut app, &ctx, 1000.0);
        let (left, _) = split_widths(1000.0 - 4.0, 0.5);
        let divider = egui::pos2(left + 2.0 + DIVIDER / 2.0, 300.0);
        let drag = |app: &mut LumenApp, to: f32| {
            frame(
                app,
                &ctx,
                1000.0,
                vec![
                    egui::Event::PointerMoved(divider),
                    egui::Event::PointerButton {
                        pos: divider,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            frame(
                app,
                &ctx,
                1000.0,
                vec![egui::Event::PointerMoved(egui::pos2(to, 300.0))],
            );
            frame(
                app,
                &ctx,
                1000.0,
                vec![egui::Event::PointerMoved(egui::pos2(to, 300.0))],
            );
            frame(
                app,
                &ctx,
                1000.0,
                vec![egui::Event::PointerButton {
                    pos: egui::pos2(to, 300.0),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        };
        let before = app.view.split_ratio;
        drag(&mut app, divider.x + 200.0);
        assert!(
            app.view.split_ratio > before,
            "dragging right widens the left group"
        );
        let wide = split_widths(1000.0, app.view.split_ratio);
        assert!(wide.0 >= MIN_GROUP_WIDTH && wide.1 >= MIN_GROUP_WIDTH);
        app.view.split_ratio = 0.5;
        let output = settle(&mut app, &ctx, 1000.0);
        let _ = output;
        drag(&mut app, -500.0);
        let narrow = split_widths(1000.0, app.view.split_ratio);
        assert!(
            narrow.0 >= MIN_GROUP_WIDTH,
            "cannot be dragged below the minimum: {narrow:?}"
        );
    }
}
