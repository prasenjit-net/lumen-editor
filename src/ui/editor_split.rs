//! Two editor groups side by side. Each group draws exactly what a single editor
//! draws (tab strip, breadcrumbs, document) by being *focused for the duration of
//! its pass*; afterwards focus is put back on the group the user is working in.
use super::{Workbench, widgets::surface};
use crate::app::command::Command;
use eframe::egui::{self, CursorIcon, Rect, Sense, Stroke};

/// A group never gets narrower than this, so its tabs and text stay usable.
pub(crate) const MIN_GROUP_WIDTH: f32 = 220.0;
/// When groups are stacked, each keeps room for a tab strip and a few lines.
pub(crate) const MIN_GROUP_HEIGHT: f32 = 140.0;
/// Space between the groups, and the divider's grab area.
const DIVIDER: f32 = 6.0;

/// Clamp a requested left-group share so both groups keep `MIN_GROUP_WIDTH`.
/// When the area is too narrow for two minimums the groups split it evenly.
pub(crate) fn split_widths(total: f32, ratio: f32) -> (f32, f32) {
    split_extent(total, ratio, MIN_GROUP_WIDTH)
}

/// The same split along either axis: the first group's share of `total`, both
/// groups at least `min` long, or an even split when that is impossible.
pub(crate) fn split_extent(total: f32, ratio: f32, min: f32) -> (f32, f32) {
    let usable = (total - DIVIDER).max(0.0);
    if usable < 2.0 * min {
        return (usable / 2.0, usable / 2.0);
    }
    let first = (usable * ratio).clamp(min, usable - min);
    (first, usable - first)
}

impl Workbench<'_> {
    /// Where a tab dragged out of its strip would land if dropped now: the group
    /// under the pointer (other than its own) and the position among that group's
    /// tabs. Dropping on the strip picks a position; dropping on the editor body
    /// appends.
    fn tab_drop_target(
        &self,
        pointer: egui::Pos2,
        columns: [Rect; 2],
        source_group: usize,
    ) -> Option<(usize, usize)> {
        use super::tab_drag::insertion_gap;
        let target = columns.iter().position(|column| column.contains(pointer))?;
        if target == source_group {
            return None;
        }
        let strip = self
            .view
            .tab_strips
            .iter()
            .find(|strip| strip.group == target);
        let position = strip
            .filter(|strip| strip.band.expand2(egui::vec2(0.0, 12.0)).contains(pointer))
            .map(|strip| {
                let centres: Vec<f32> =
                    strip.tabs.iter().map(|(_, rect)| rect.center().x).collect();
                insertion_gap(&centres, pointer.x)
            })
            .unwrap_or(usize::MAX);
        Some((target, position))
    }

    /// Finish a drag across groups (the drop itself) and, while one is in
    /// progress, highlight the group it would land in with an insertion marker.
    fn resolve_tab_drag(&mut self, ui: &mut egui::Ui, columns: [Rect; 2]) {
        let colors = crate::ui::theme::palette(ui.visuals());
        if let Some((id, pointer)) = self.view.tab_drop_request.take() {
            let source = self.editor.group_of(id);
            if let Some((group, position)) = self.tab_drop_target(pointer, columns, source) {
                self.command(Command::MoveTabToGroupAt(id, group, position));
            }
        }
        let (Some(drag), Some(pointer)) = (self.view.tab_drag, ui.ctx().pointer_latest_pos())
        else {
            return;
        };
        let Some((group, position)) = self.tab_drop_target(pointer, columns, drag.source_group)
        else {
            return;
        };
        ui.painter().rect_stroke(
            columns[group].shrink(1.5),
            5,
            Stroke::new(2.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        );
        if let Some(strip) = self
            .view
            .tab_strips
            .iter()
            .find(|strip| strip.group == group)
            && position != usize::MAX
        {
            let rects: Vec<Rect> = strip.tabs.iter().map(|(_, rect)| *rect).collect();
            if let Some(x) = super::tab_drag::marker_x(&rects, position) {
                ui.painter()
                    .vline(x, strip.band.y_range(), Stroke::new(2.0_f32, colors.accent));
            }
        }
    }

    pub(super) fn editor_split(&mut self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let area = ui.available_rect_before_wrap();
        let vertical = self.view.split_vertical;
        // `left`/`right` are the first and second group, whichever way they stack.
        let (left, divider, right, first_extent) = if vertical {
            let (top, bottom) =
                split_extent(area.height(), self.view.split_ratio, MIN_GROUP_HEIGHT);
            let first = Rect::from_min_size(area.min, egui::vec2(area.width(), top));
            let divider = Rect::from_min_size(
                egui::pos2(area.left(), first.bottom()),
                egui::vec2(area.width(), DIVIDER),
            );
            let second = Rect::from_min_size(
                egui::pos2(area.left(), divider.bottom()),
                egui::vec2(area.width(), bottom),
            );
            (first, divider, second, top)
        } else {
            let (left_width, right_width) = split_widths(area.width(), self.view.split_ratio);
            let first = Rect::from_min_size(area.min, egui::vec2(left_width, area.height()));
            let divider = Rect::from_min_size(
                egui::pos2(first.right(), area.top()),
                egui::vec2(DIVIDER, area.height()),
            );
            let second = Rect::from_min_size(
                egui::pos2(divider.right(), area.top()),
                egui::vec2(right_width, area.height()),
            );
            (first, divider, second, left_width)
        };
        ui.allocate_rect(area, Sense::hover());

        // Drag bookkeeping is rebuilt from scratch every frame by the tab strips.
        self.view.tab_strips.clear();
        self.view.tab_drag = None;
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

        self.resolve_tab_drag(ui, [left, right]);

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
            ui.ctx().set_cursor_icon(if vertical {
                CursorIcon::ResizeVertical
            } else {
                CursorIcon::ResizeHorizontal
            });
        }
        let length = if vertical {
            area.height()
        } else {
            area.width()
        };
        if response.dragged() && length > DIVIDER {
            let delta = if vertical {
                response.drag_delta().y
            } else {
                response.drag_delta().x
            };
            self.view.split_ratio = ((first_extent + delta) / (length - DIVIDER)).clamp(0.1, 0.9);
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
        app.view.shared_editor = true;
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

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Drag from `from` to `to` (optionally pressing Escape first), releasing at `to`.
    fn drag(
        app: &mut LumenApp,
        ctx: &egui::Context,
        from: egui::Pos2,
        to: egui::Pos2,
        escape: bool,
    ) {
        frame(
            app,
            ctx,
            1000.0,
            vec![egui::Event::PointerMoved(from), press(from, true)],
        );
        frame(
            app,
            ctx,
            1000.0,
            vec![egui::Event::PointerMoved(from + egui::vec2(24.0, 0.0))],
        );
        frame(app, ctx, 1000.0, vec![egui::Event::PointerMoved(to)]);
        frame(app, ctx, 1000.0, vec![egui::Event::PointerMoved(to)]);
        if escape {
            frame(
                app,
                ctx,
                1000.0,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        frame(
            app,
            ctx,
            1000.0,
            vec![egui::Event::PointerMoved(to), press(to, false)],
        );
        settle(app, ctx, 1000.0);
    }

    fn names(app: &LumenApp, group: usize) -> Vec<String> {
        app.editor
            .documents_in(group)
            .iter()
            .map(|id| {
                app.editor.files[app.editor.index_of(*id).unwrap()]
                    .name
                    .clone()
            })
            .collect()
    }

    fn tab_center(output: &egui::FullOutput, name: &str) -> egui::Pos2 {
        rect_of(output, name)
            .unwrap_or_else(|| panic!("no tab {name}"))
            .center()
    }

    #[test]
    fn dropping_a_tab_on_the_other_groups_editor_appends_it_there() {
        let (mut app, ctx) = split_app();
        let output = settle(&mut app, &ctx, 1000.0);
        let from = tab_center(&output, "aaa.rs");
        drag(&mut app, &ctx, from, egui::pos2(800.0, 300.0), false);
        assert_eq!(names(&app, 1), ["ccc.rs", "ddd.rs", "aaa.rs"]);
        assert_eq!(names(&app, 0), ["bbb.rs"]);
        assert_eq!(app.editor.focused_group(), 1);
        assert_eq!(app.editor.active().unwrap().name, "aaa.rs");
        assert_eq!(
            app.editor
                .files
                .iter()
                .find(|f| f.name == "aaa.rs")
                .unwrap()
                .text,
            "LEFTONE"
        );
    }

    #[test]
    fn dropping_on_the_other_groups_strip_chooses_the_position() {
        let (mut app, ctx) = split_app();
        let output = settle(&mut app, &ctx, 1000.0);
        let (ccc, ddd) = (tab_center(&output, "ccc.rs"), tab_center(&output, "ddd.rs"));
        let between = egui::pos2((ccc.x + ddd.x) / 2.0, ccc.y);
        drag(
            &mut app,
            &ctx,
            tab_center(&output, "bbb.rs"),
            between,
            false,
        );
        assert_eq!(names(&app, 1), ["ccc.rs", "bbb.rs", "ddd.rs"]);
        assert_eq!(names(&app, 0), ["aaa.rs"]);
    }

    #[test]
    fn escape_or_a_drop_on_the_tabs_own_group_does_not_move_it() {
        let (mut app, ctx) = split_app();
        let output = settle(&mut app, &ctx, 1000.0);
        let from = tab_center(&output, "aaa.rs");
        drag(&mut app, &ctx, from, egui::pos2(800.0, 300.0), true);
        assert_eq!(names(&app, 0), ["aaa.rs", "bbb.rs"]);
        drag(&mut app, &ctx, from, egui::pos2(250.0, 300.0), false);
        assert_eq!(
            names(&app, 0),
            ["aaa.rs", "bbb.rs"],
            "own body is not a drop target"
        );
        assert_eq!(names(&app, 1), ["ccc.rs", "ddd.rs"]);
    }

    #[test]
    fn dragging_a_groups_last_tab_away_closes_that_group() {
        let (mut app, ctx) = split_app();
        let output = settle(&mut app, &ctx, 1000.0);
        drag(
            &mut app,
            &ctx,
            tab_center(&output, "aaa.rs"),
            egui::pos2(800.0, 300.0),
            false,
        );
        let output = settle(&mut app, &ctx, 1000.0);
        drag(
            &mut app,
            &ctx,
            tab_center(&output, "bbb.rs"),
            egui::pos2(800.0, 300.0),
            false,
        );
        assert_eq!(app.editor.group_count(), 1);
        assert_eq!(app.editor.files.len(), 4);
        assert!(app.editor.files.iter().all(|f| !f.dirty));
    }

    #[test]
    fn a_drag_over_the_other_group_highlights_it_before_the_drop() {
        let (mut app, ctx) = split_app();
        let output = settle(&mut app, &ctx, 1000.0);
        let from = tab_center(&output, "aaa.rs");
        frame(
            &mut app,
            &ctx,
            1000.0,
            vec![egui::Event::PointerMoved(from), press(from, true)],
        );
        frame(
            &mut app,
            &ctx,
            1000.0,
            vec![egui::Event::PointerMoved(from + egui::vec2(24.0, 0.0))],
        );
        let target = egui::pos2(800.0, 300.0);
        frame(
            &mut app,
            &ctx,
            1000.0,
            vec![egui::Event::PointerMoved(target)],
        );
        let output = frame(
            &mut app,
            &ctx,
            1000.0,
            vec![egui::Event::PointerMoved(target)],
        );
        let accent = crate::ui::theme::Theme::Dark.palette().accent;
        let highlighted = output.shapes.iter().any(|shape| {
            matches!(
                &shape.shape,
                egui::Shape::Rect(rect) if rect.stroke.width == 2.0 && rect.stroke.color == accent
                    && rect.rect.contains(target) && rect.rect.left() > 400.0
            )
        });
        assert!(
            highlighted,
            "the group under the pointer is outlined while dragging"
        );
        assert_eq!(
            names(&app, 1),
            ["ccc.rs", "ddd.rs"],
            "nothing moves until release"
        );
    }

    fn frame_sized(
        app: &mut LumenApp,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.editor(ctx)),
        )
    }

    fn settle_sized(app: &mut LumenApp, ctx: &egui::Context, size: egui::Vec2) -> egui::FullOutput {
        let mut output = frame_sized(app, ctx, size, vec![]);
        for _ in 0..4 {
            output = frame_sized(app, ctx, size, vec![]);
        }
        output
    }

    #[test]
    fn extents_keep_a_minimum_along_either_axis() {
        let (top, bottom) = split_extent(600.0, 0.9, MIN_GROUP_HEIGHT);
        assert!(top >= MIN_GROUP_HEIGHT && bottom >= MIN_GROUP_HEIGHT);
        assert!((top + bottom + DIVIDER - 600.0).abs() < 0.01);
        let (a, b) = split_extent(200.0, 0.2, MIN_GROUP_HEIGHT);
        assert_eq!(a, b, "too short for two minimums: even split");
    }

    #[test]
    fn toggling_stacks_the_groups_and_refuses_when_there_is_nothing_to_stack() {
        let (mut app, ctx) = split_app();
        app.execute(Command::ToggleSplitOrientation, &ctx);
        assert!(app.view.split_vertical);
        app.execute(Command::ToggleSplitOrientation, &ctx);
        assert!(!app.view.split_vertical);
        let mut single = LumenApp::default();
        single.execute(Command::ToggleSplitOrientation, &ctx);
        assert!(!single.view.split_vertical);
        assert_eq!(single.view.status, "The editor is not split");
    }

    #[test]
    fn stacked_groups_sit_one_above_the_other_inside_the_window_at_every_height() {
        for height in [420.0, 600.0, 900.0] {
            let (mut app, ctx) = split_app();
            app.view.split_vertical = true;
            let size = egui::vec2(900.0, height);
            let output = settle_sized(&mut app, &ctx, size);
            let tab =
                |name: &str| rect_of(&output, name).unwrap_or_else(|| panic!("{name} at {height}"));
            assert!(
                tab("aaa.rs").center().y < tab("ccc.rs").center().y,
                "{height}"
            );
            assert!((tab("aaa.rs").center().x - tab("ccc.rs").center().x).abs() < 150.0);
            let window = Rect::from_min_size(egui::Pos2::ZERO, size);
            for name in ["aaa.rs", "bbb.rs", "ccc.rs", "ddd.rs"] {
                assert!(
                    window.contains_rect(tab(name)),
                    "{name} escapes at {height}"
                );
            }
            let tab_titles = output
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text.starts_with('\u{eae9}'))
                })
                .count();
            assert_eq!(tab_titles, 4, "two tabs per group at {height}");
        }
    }

    #[test]
    fn stacked_groups_route_typing_and_accept_tab_drops_like_side_by_side_ones() {
        let (mut app, ctx) = split_app();
        app.view.split_vertical = true;
        let size = egui::vec2(900.0, 700.0);
        settle_sized(&mut app, &ctx, size);
        app.execute(Command::FocusGroup(1), &ctx);
        settle_sized(&mut app, &ctx, size);
        frame_sized(&mut app, &ctx, size, vec![egui::Event::Text("Q".into())]);
        settle_sized(&mut app, &ctx, size);
        let texts: Vec<_> = app.editor.files.iter().map(|f| f.text.clone()).collect();
        assert_eq!(texts[..3], ["LEFTONE", "LEFTTWO", "RIGHTONE"]);
        assert!(texts[3].contains('Q'));
        // Drag aaa from the top group onto the lower group's editor body.
        let output = settle_sized(&mut app, &ctx, size);
        let from = tab_center(&output, "aaa.rs");
        let to = egui::pos2(450.0, 600.0);
        frame_sized(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(from), press(from, true)],
        );
        frame_sized(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(from + egui::vec2(24.0, 0.0))],
        );
        frame_sized(&mut app, &ctx, size, vec![egui::Event::PointerMoved(to)]);
        frame_sized(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(to), press(to, false)],
        );
        settle_sized(&mut app, &ctx, size);
        assert_eq!(names(&app, 1), ["ccc.rs", "ddd.rs", "aaa.rs"]);
    }

    #[test]
    fn the_stacked_divider_resizes_vertically_within_the_minimums() {
        let (mut app, ctx) = split_app();
        app.view.split_vertical = true;
        let size = egui::vec2(900.0, 700.0);
        settle_sized(&mut app, &ctx, size);
        let (top, _) = split_extent(700.0 - 4.0, 0.5, MIN_GROUP_HEIGHT);
        let divider = egui::pos2(450.0, 2.0 + top + DIVIDER / 2.0);
        let before = app.view.split_ratio;
        frame_sized(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(divider), press(divider, true)],
        );
        let target = egui::pos2(450.0, divider.y + 120.0);
        frame_sized(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(target)],
        );
        frame_sized(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(target)],
        );
        frame_sized(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(target), press(target, false)],
        );
        assert!(
            app.view.split_ratio > before,
            "dragging down grows the top group"
        );
        let (a, b) = split_extent(700.0, app.view.split_ratio, MIN_GROUP_HEIGHT);
        assert!(a >= MIN_GROUP_HEIGHT && b >= MIN_GROUP_HEIGHT);
    }
}
