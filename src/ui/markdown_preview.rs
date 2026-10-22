//! Markdown preview tab: renders the parsed block tree of the source document.
//! Pure drawing; links become typed commands and images are requested from the
//! app, which loads them off the UI thread.
use super::{Workbench, theme};
use crate::{
    app::{
        command::Command,
        preview::{PreviewImage, PreviewState, resolve_relative},
    },
    icon,
    model::{
        markdown::{Block, ColumnAlign, Inline, LinkTarget, ListItem},
        previewable::PreviewKind,
    },
};
use eframe::egui::{
    self, Align, Color32, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    TextFormat, text::LayoutJob,
};
use std::path::Path;

const BODY_SIZE: f32 = 13.0;
const CODE_SIZE: f32 = 12.0;
/// Readable line length for prose; wider windows center the column.
const MAX_WIDTH: f32 = 860.0;
const SIDE_PADDING: f32 = 20.0;
const BLOCK_GAP: f32 = 8.0;

impl Workbench<'_> {
    pub(super) fn preview_page(&mut self, ui: &mut egui::Ui, index: usize) {
        let colors = theme::palette(ui.visuals());
        let preview = self.editor.files[index].id;
        let Some(source) = self.editor.files[index]
            .preview_of
            .and_then(|id| self.editor.index_of(id))
        else {
            ui.label(RichText::new("The previewed file was closed").color(colors.muted));
            return;
        };
        // The source may not have rendered this frame; bring its text up to date.
        self.editor.files[source].sync_shared_editing();
        if self.editor.files[source].preview_kind() == Some(PreviewKind::Svg) {
            self.svg_preview_page(ui, preview, source);
            return;
        }
        let blocks = self.view.preview.blocks(&self.editor.files[source]);
        let source_path = self.editor.files[source].path.clone();
        let mut anchor = match self.view.preview.anchor_request.take() {
            Some((target, anchor)) if target == preview => Some(anchor),
            other => {
                self.view.preview.anchor_request = other;
                None
            }
        };
        let mut commands = Vec::new();
        ScrollArea::vertical()
            .id_salt(("markdown-preview", preview))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = (ui.available_width() - 2.0 * SIDE_PADDING).clamp(0.0, MAX_WIDTH);
                let pad = ((ui.available_width() - width) / 2.0).max(0.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.add_space(12.0);
                        let mut renderer = Renderer {
                            colors,
                            preview,
                            source_path: source_path.as_deref(),
                            state: &mut self.view.preview,
                            commands: &mut commands,
                            anchor: &mut anchor,
                            gap: BLOCK_GAP,
                        };
                        renderer.blocks(ui, &blocks, BODY_SIZE);
                        ui.add_space(24.0);
                    });
                });
            });
        if anchor.is_some() {
            self.view.status = "No heading matches that link".into();
        }
        for command in commands {
            self.command(command);
        }
    }
}

struct Renderer<'a> {
    colors: theme::Palette,
    preview: crate::model::DocumentId,
    source_path: Option<&'a Path>,
    state: &'a mut PreviewState,
    commands: &'a mut Vec<Command>,
    /// Heading anchor still waiting to be scrolled to.
    anchor: &'a mut Option<String>,
    /// Space between sibling blocks; tighter inside list items.
    gap: f32,
}

#[derive(Clone, Copy)]
struct Style {
    size: f32,
    color: Color32,
    strong: bool,
    italics: bool,
    strike: bool,
}

impl<'a> Renderer<'a> {
    fn body_color(&self) -> Color32 {
        self.colors.text.lerp_to_gamma(self.colors.muted, 0.2)
    }

    fn style(&self, size: f32) -> Style {
        Style {
            size,
            color: self.body_color(),
            strong: false,
            italics: false,
            strike: false,
        }
    }

    fn blocks(&mut self, ui: &mut egui::Ui, blocks: &[Block], size: f32) {
        for (position, block) in blocks.iter().enumerate() {
            if position > 0 {
                ui.add_space(self.gap);
            }
            self.block(ui, block, size);
        }
    }

    fn block(&mut self, ui: &mut egui::Ui, block: &Block, size: f32) {
        match block {
            Block::Heading {
                level,
                anchor,
                content,
            } => self.heading(ui, *level, anchor, content),
            Block::Paragraph(inlines) => {
                self.flow(ui, inlines, self.style(size));
            }
            Block::Quote(inner) => {
                let frame = Frame::NONE.inner_margin(Margin {
                    left: 14,
                    right: 0,
                    top: 0,
                    bottom: 0,
                });
                let response = frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.blocks(ui, inner, size);
                });
                let rect = response.response.rect;
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.left() + 3.0, rect.top()),
                        egui::pos2(rect.left() + 3.0, rect.bottom()),
                    ],
                    Stroke::new(3.0_f32, self.colors.muted),
                );
            }
            Block::Code { language, text } => self.code_block(ui, language, text),
            Block::List { start, items } => self.list(ui, *start, items, size),
            Block::Rule => {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::hover());
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    Stroke::new(1.0_f32, self.colors.border),
                );
            }
            Block::Table {
                alignments,
                head,
                rows,
            } => self.table(ui, alignments, head, rows),
            Block::Html(html) => {
                ui.add(
                    egui::Label::new(
                        RichText::new(html)
                            .monospace()
                            .size(CODE_SIZE)
                            .color(self.colors.muted),
                    )
                    .selectable(true),
                );
            }
        }
    }

    fn heading(&mut self, ui: &mut egui::Ui, level: u8, anchor: &str, content: &[Inline]) {
        let size = match level {
            1 => 24.0,
            2 => 19.0,
            3 => 16.0,
            4 => 14.5,
            _ => BODY_SIZE,
        };
        let mut style = self.style(size);
        style.strong = true;
        style.color = if level == 6 {
            self.colors.muted
        } else {
            self.colors.text
        };
        ui.add_space(if level <= 2 { 6.0 } else { 2.0 });
        let response = self.flow(ui, content, style);
        if self.anchor.as_deref() == Some(anchor) {
            response.scroll_to_me(Some(Align::TOP));
            *self.anchor = None;
        }
        if level <= 2 {
            ui.add_space(2.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, self.colors.border);
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, start: Option<u64>, items: &[ListItem], size: f32) {
        for (position, item) in items.iter().enumerate() {
            if position > 0 {
                ui.add_space(3.0);
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let marker_width = 22.0;
                if let Some(checked) = item.task {
                    self.task_box(ui, checked);
                } else {
                    let marker = match start {
                        Some(first) => format!("{}.", first + position as u64),
                        None => "•".to_owned(),
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(marker_width, 0.0),
                        Layout::top_down(Align::Max),
                        |ui| {
                            ui.label(RichText::new(marker).size(size).color(self.colors.muted));
                        },
                    );
                }
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    let outer = std::mem::replace(&mut self.gap, 2.0);
                    self.blocks(ui, &item.blocks, size);
                    self.gap = outer;
                });
            });
        }
    }

    fn task_box(&self, ui: &mut egui::Ui, checked: bool) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
        let rect = rect.shrink(1.5);
        ui.painter().rect(
            rect,
            3.0,
            if checked {
                self.colors.accent
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(
                1.0_f32,
                if checked {
                    self.colors.accent
                } else {
                    self.colors.muted
                },
            ),
            egui::StrokeKind::Inside,
        );
        if checked {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon::CHECK,
                FontId::proportional(11.0),
                self.colors.on_accent,
            );
        }
    }

    fn code_block(&mut self, ui: &mut egui::Ui, language: &str, text: &str) {
        let colors = self.colors;
        Frame::default()
            .fill(colors.panel)
            .stroke(Stroke::new(1.0_f32, colors.border))
            .corner_radius(3)
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let job = highlighted_job(ui.ctx(), text, language, ui.visuals(), colors.text);
                ScrollArea::horizontal()
                    .id_salt(("preview-code", self.preview, text.len(), language))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(job)
                                .wrap_mode(egui::TextWrapMode::Extend)
                                .selectable(true),
                        );
                    });
            });
    }

    fn table(
        &mut self,
        ui: &mut egui::Ui,
        alignments: &[ColumnAlign],
        head: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
    ) {
        let colors = self.colors;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ScrollArea::horizontal()
            .id_salt(("preview-table", self.preview, head.len(), rows.len()))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new(("preview-grid", self.preview, rows.len(), head.len()))
                    .striped(true)
                    .spacing([16.0, 5.0])
                    .show(ui, |ui| {
                        let align = |column: usize| match alignments.get(column) {
                            Some(ColumnAlign::Center) => Align::Center,
                            Some(ColumnAlign::Right) => Align::Max,
                            _ => Align::Min,
                        };
                        for (column, cell) in head.iter().enumerate() {
                            let mut style = self.style(BODY_SIZE);
                            style.strong = true;
                            style.color = colors.text;
                            ui.with_layout(Layout::top_down(align(column)), |ui| {
                                self.flow(ui, cell, style);
                            });
                        }
                        ui.end_row();
                        for row in rows {
                            for (column, cell) in row.iter().enumerate() {
                                ui.with_layout(Layout::top_down(align(column)), |ui| {
                                    self.flow(ui, cell, self.style(BODY_SIZE));
                                });
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    /// Inline content, wrapped. Plain runs share one label; links and images
    /// are their own widgets so they can be clicked.
    fn flow(&mut self, ui: &mut egui::Ui, inlines: &[Inline], style: Style) -> egui::Response {
        let interactive = inlines.iter().any(has_widget);
        if !interactive {
            let mut job = LayoutJob::default();
            self.append(&mut job, inlines, style);
            return ui.add(egui::Label::new(job).selectable(true));
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut run: Vec<&Inline> = Vec::new();
            for inline in inlines {
                match inline {
                    Inline::Link { .. } | Inline::Image { .. } => {
                        self.flush_run(ui, &mut run, style);
                        self.widget(ui, inline, style);
                    }
                    other => run.push(other),
                }
            }
            self.flush_run(ui, &mut run, style);
        })
        .response
    }

    fn flush_run(&mut self, ui: &mut egui::Ui, run: &mut Vec<&Inline>, style: Style) {
        if run.is_empty() {
            return;
        }
        let mut job = LayoutJob::default();
        for inline in run.drain(..) {
            self.append(&mut job, std::slice::from_ref(inline), style);
        }
        ui.add(egui::Label::new(job).selectable(true));
    }

    fn widget(&mut self, ui: &mut egui::Ui, inline: &Inline, style: Style) {
        match inline {
            Inline::Link {
                destination,
                content,
            } => {
                let mut job = LayoutJob::default();
                let mut link = style;
                link.color = self.colors.accent;
                self.append(&mut job, content, link);
                for section in &mut job.sections {
                    section.format.underline = Stroke::new(1.0_f32, self.colors.accent);
                }
                let response = ui
                    .add(egui::Label::new(job).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(destination);
                if response.clicked() {
                    self.commands
                        .push(Command::OpenPreviewLink(self.preview, destination.clone()));
                }
            }
            Inline::Image { destination, alt } => self.image(ui, destination, alt),
            _ => {}
        }
    }

    fn image(&mut self, ui: &mut egui::Ui, destination: &str, alt: &str) {
        let fallback = |ui: &mut egui::Ui, colors: &theme::Palette| {
            ui.label(
                RichText::new(if alt.is_empty() { destination } else { alt })
                    .size(BODY_SIZE)
                    .italics()
                    .color(colors.muted),
            )
            .on_hover_text(destination);
        };
        // Remote images stay off: no network access from a document preview.
        let LinkTarget::File(relative) = LinkTarget::classify(destination) else {
            fallback(ui, &self.colors);
            return;
        };
        let Some(path) = self
            .source_path
            .and_then(|source| resolve_relative(source, &relative))
        else {
            fallback(ui, &self.colors);
            return;
        };
        match self.state.image(&path) {
            Some(PreviewImage::Ready { image, texture }) => {
                let handle = texture.get_or_insert_with(|| {
                    ui.ctx().load_texture(
                        format!("preview-image:{}", path.display()),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [image.texture_width as usize, image.texture_height as usize],
                            &image.rgba,
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                });
                let natural = egui::vec2(image.width as f32, image.height as f32);
                let scale = (ui.available_width() / natural.x).min(1.0);
                ui.add(egui::Image::new((handle.id(), natural * scale)))
                    .on_hover_text(alt);
            }
            Some(PreviewImage::Failed) => fallback(ui, &self.colors),
            Some(PreviewImage::Loading) | None => {
                ui.spinner();
            }
        }
    }

    fn append(&self, job: &mut LayoutJob, inlines: &[Inline], style: Style) {
        for inline in inlines {
            match inline {
                Inline::Text(text) => job.append(text, 0.0, self.format(style)),
                Inline::Code(code) => {
                    let mut format = self.format(style);
                    format.font_id = FontId::monospace((style.size - 1.0).max(10.0));
                    format.background = self.colors.input;
                    job.append(code, 0.0, format);
                }
                Inline::Emphasis(inner) => {
                    let mut style = style;
                    style.italics = true;
                    self.append(job, inner, style);
                }
                Inline::Strong(inner) => {
                    let mut style = style;
                    style.strong = true;
                    self.append(job, inner, style);
                }
                Inline::Strikethrough(inner) => {
                    let mut style = style;
                    style.strike = true;
                    self.append(job, inner, style);
                }
                // Links and images nested in a plain run (such as a table cell
                // inside a heading) fall back to their text.
                Inline::Link { content, .. } => self.append(job, content, style),
                Inline::Image { alt, .. } => job.append(alt, 0.0, self.format(style)),
                Inline::SoftBreak => job.append(" ", 0.0, self.format(style)),
                Inline::HardBreak => job.append("\n", 0.0, self.format(style)),
            }
        }
    }

    fn format(&self, style: Style) -> TextFormat {
        TextFormat {
            font_id: FontId::proportional(style.size),
            color: if style.strong {
                self.colors.text
            } else {
                style.color
            },
            italics: style.italics,
            strikethrough: if style.strike {
                Stroke::new(1.0_f32, style.color)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        }
    }
}

fn has_widget(inline: &Inline) -> bool {
    match inline {
        Inline::Link { .. } | Inline::Image { .. } => true,
        Inline::Emphasis(inner) | Inline::Strong(inner) | Inline::Strikethrough(inner) => {
            inner.iter().any(has_widget)
        }
        _ => false,
    }
}

/// Monospace layout of `text`, colored by the editor's highlighter when the
/// fence names a known language. Colors are cached per text and palette.
fn highlighted_job(
    ctx: &egui::Context,
    text: &str,
    language: &str,
    visuals: &egui::Visuals,
    default: Color32,
) -> LayoutJob {
    use std::hash::{Hash, Hasher};
    let font = FontId::monospace(CODE_SIZE);
    let mut job = LayoutJob::default();
    let spans = fence_language(language).map(|language| {
        let palette = theme::syntax_palette(theme::current(visuals).scheme());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        language.label().hash(&mut hasher);
        palette.keyword.hash(&mut hasher);
        palette.comment.hash(&mut hasher);
        let key = egui::Id::new(("preview-syntax", hasher.finish(), text.len()));
        type Spans = std::sync::Arc<Vec<(std::ops::Range<usize>, Color32)>>;
        if let Some(spans) = ctx.data(|data| data.get_temp::<Spans>(key)) {
            return spans;
        }
        let spans: Spans = Spans::new(lumen_editor_surface::syntax_colors(text, language, palette));
        ctx.data_mut(|data| data.insert_temp(key, spans.clone()));
        spans
    });
    let mut cursor = 0;
    for (range, color) in spans.iter().flat_map(|spans| spans.iter()) {
        if range.start < cursor
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            continue;
        }
        if range.start > cursor {
            job.append(
                &text[cursor..range.start],
                0.0,
                TextFormat::simple(font.clone(), default),
            );
        }
        job.append(
            &text[range.clone()],
            0.0,
            TextFormat::simple(font.clone(), *color),
        );
        cursor = range.end;
    }
    job.append(&text[cursor..], 0.0, TextFormat::simple(font, default));
    job
}

/// Language named by a code fence (`rs`, `rust`, `Python`, ...).
pub(crate) fn fence_language(name: &str) -> Option<lumen_editor_core::syntax::Language> {
    use lumen_editor_core::syntax::Language;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let lower = name.to_lowercase();
    let extension = match lower.as_str() {
        "rust" => "rs",
        "python" => "py",
        "javascript" | "node" => "js",
        "shell" | "bash" | "zsh" | "console" => "sh",
        "golang" => "go",
        "yaml" => "yml",
        "markdown" => "md",
        "c++" => "cpp",
        "kotlin" => "kt",
        other => other,
    };
    let by_extension = Language::from_path(Path::new(&format!("code.{extension}")));
    if by_extension != Language::Plain {
        return Some(by_extension);
    }
    let by_name = Language::from_path(Path::new(&lower));
    (by_name != Language::Plain).then_some(by_name)
}
