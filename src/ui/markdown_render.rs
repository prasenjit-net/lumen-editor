//! Markdown block rendering shared by the Markdown preview tab and agent chat
//! replies. The two differ only in their [`Metrics`] (sizes and spacing) and their
//! [`Host`] (what links, images and widget ids do), so a reply shows the same
//! tables, nested lists, links and highlighted code as a previewed document.
use super::theme;
use crate::{
    app::{
        command::Command,
        preview::{PreviewImage, PreviewState, resolve_relative},
    },
    icon,
    model::markdown::{self, Block, ColumnAlign, Inline, LinkTarget, ListItem},
};
use eframe::egui::{
    self, Align, Color32, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    TextFormat,
    text::LayoutJob,
    util::cache::{ComputerMut, FrameCache},
};
use std::{path::Path, sync::Arc};

/// Text sizes and spacing for one rendering surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Metrics {
    pub(super) body: f32,
    pub(super) code: f32,
    /// Heading sizes for levels 1-4; 5 and 6 use the body size.
    pub(super) headings: [f32; 4],
    /// Underline level 1-2 headings, as a document does.
    pub(super) heading_rules: bool,
    /// Space between sibling blocks.
    pub(super) gap: f32,
    /// Space between blocks inside a list item, and between list items.
    pub(super) list_gap: f32,
    /// Code blocks taller than this scroll; `None` shows them whole.
    pub(super) code_max_height: Option<f32>,
    /// A language label and copy button above each code block.
    pub(super) code_header: bool,
}

impl Metrics {
    /// The Markdown preview tab: a readable document.
    pub(super) const PREVIEW: Self = Self {
        body: 13.0,
        code: 12.0,
        headings: [24.0, 19.0, 16.0, 14.5],
        heading_rules: true,
        gap: 8.0,
        list_gap: 2.0,
        code_max_height: None,
        code_header: false,
    };

    /// Agent replies in the narrow chat column: compact, and code never takes
    /// over the conversation.
    pub(super) const CHAT: Self = Self {
        body: 12.5,
        code: 11.5,
        headings: [15.0, 13.5, 12.5, 12.5],
        heading_rules: false,
        gap: 6.0,
        list_gap: 2.0,
        code_max_height: Some(280.0),
        code_header: true,
    };

    /// An agent's working notes between steps: one step quieter than a reply.
    pub(super) const CHAT_NOTES: Self = Self {
        body: 11.5,
        code: 11.0,
        headings: [12.5, 12.0, 11.5, 11.5],
        ..Self::CHAT
    };
}

/// Where the Markdown is shown, which decides what links and images do and how
/// widget ids are made unique.
pub(super) enum Host<'a> {
    Preview {
        document: crate::model::DocumentId,
        source_path: Option<&'a Path>,
        state: &'a mut PreviewState,
        /// Heading anchor still waiting to be scrolled to.
        anchor: &'a mut Option<String>,
    },
    /// One transcript item of an agent chat. Images are not loaded; links are
    /// resolved by the app against the session's folder.
    Chat {
        session: lumen_agent_core::SessionId,
        item: &'a str,
    },
}

/// Parsed blocks, so a long transcript is not re-parsed every frame.
#[derive(Default)]
struct Parser;

impl ComputerMut<&str, Arc<Vec<Block>>> for Parser {
    fn compute(&mut self, text: &str) -> Arc<Vec<Block>> {
        Arc::new(markdown::parse(text))
    }
}

type ParsedCache = FrameCache<Arc<Vec<Block>>, Parser>;

/// Parse `text` (cached per text for as long as it is shown).
pub(super) fn parsed(ctx: &egui::Context, text: &str) -> Arc<Vec<Block>> {
    ctx.memory_mut(|memory| memory.caches.cache::<ParsedCache>().get(text))
}

/// Draw `text` as Markdown at `metrics`. `muted` dims the body text (agent
/// working notes) while keeping emphasis readable.
pub(super) fn show(
    ui: &mut egui::Ui,
    text: &str,
    metrics: Metrics,
    host: Host<'_>,
    muted: bool,
    commands: &mut Vec<Command>,
) {
    let blocks = parsed(ui.ctx(), text);
    let colors = theme::palette(ui.visuals());
    let mut renderer = Renderer::new(colors, metrics, host, commands);
    if muted {
        renderer.body = colors.muted;
    }
    ui.scope(|ui| {
        // Spacing comes from the metrics alone, not from the caller's layout.
        ui.spacing_mut().item_spacing.y = 0.0;
        renderer.blocks(ui, &blocks, metrics.body);
    });
}

pub(super) struct Renderer<'a, 'c> {
    colors: theme::Palette,
    metrics: Metrics,
    host: Host<'a>,
    commands: &'c mut Vec<Command>,
    /// Colour of ordinary text; strong text uses the full text colour.
    body: Color32,
    /// Space between sibling blocks; tighter inside list items.
    gap: f32,
    /// Code blocks and tables drawn so far, for unique ids within a chat item.
    widgets: usize,
}

#[derive(Clone, Copy)]
struct Style {
    size: f32,
    color: Color32,
    strong: bool,
    italics: bool,
    strike: bool,
}

impl<'a, 'c> Renderer<'a, 'c> {
    pub(super) fn new(
        colors: theme::Palette,
        metrics: Metrics,
        host: Host<'a>,
        commands: &'c mut Vec<Command>,
    ) -> Self {
        Self {
            colors,
            metrics,
            host,
            commands,
            body: colors.text.lerp_to_gamma(colors.muted, 0.2),
            gap: metrics.gap,
            widgets: 0,
        }
    }

    fn style(&self, size: f32) -> Style {
        Style {
            size,
            color: self.body,
            strong: false,
            italics: false,
            strike: false,
        }
    }

    pub(super) fn blocks(&mut self, ui: &mut egui::Ui, blocks: &[Block], size: f32) {
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
                            .size(self.metrics.code)
                            .color(self.colors.muted),
                    )
                    .selectable(true),
                );
            }
        }
    }

    fn heading(&mut self, ui: &mut egui::Ui, level: u8, anchor: &str, content: &[Inline]) {
        let size = match level {
            1..=4 => self.metrics.headings[usize::from(level - 1)],
            _ => self.metrics.body,
        };
        let mut style = self.style(size);
        style.strong = true;
        style.color = if level == 6 {
            self.colors.muted
        } else {
            self.colors.text
        };
        let rule = self.metrics.heading_rules && level <= 2;
        ui.add_space(if rule { 6.0 } else { 2.0 });
        let response = self.flow(ui, content, style);
        if let Host::Preview {
            anchor: requested, ..
        } = &mut self.host
            && requested.as_deref() == Some(anchor)
        {
            response.scroll_to_me(Some(Align::TOP));
            **requested = None;
        }
        if rule {
            ui.add_space(2.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, self.colors.border);
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, start: Option<u64>, items: &[ListItem], size: f32) {
        for (position, item) in items.iter().enumerate() {
            if position > 0 {
                ui.add_space(self.metrics.list_gap + 1.0);
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
                // A vertical layout wraps its labels; a horizontal one would not.
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    let outer = std::mem::replace(&mut self.gap, self.metrics.list_gap);
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
        let metrics = self.metrics;
        self.widgets += 1;
        let ordinal = self.widgets;
        let inner = if metrics.code_header {
            Margin::symmetric(8, 5)
        } else {
            Margin::symmetric(10, 8)
        };
        Frame::default()
            .fill(colors.panel)
            .stroke(Stroke::new(1.0_f32, colors.border))
            .corner_radius(3)
            .inner_margin(inner)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if metrics.code_header {
                    code_header(ui, language, text, &colors);
                }
                let job = highlighted_job(
                    ui.ctx(),
                    text,
                    language,
                    metrics.code,
                    ui.visuals(),
                    colors.text,
                );
                let label = |ui: &mut egui::Ui| {
                    ui.add(
                        egui::Label::new(job)
                            .wrap_mode(egui::TextWrapMode::Extend)
                            .selectable(true),
                    );
                };
                match &self.host {
                    Host::Preview { document, .. } => {
                        ScrollArea::horizontal()
                            .id_salt(("preview-code", *document, text.len(), language))
                            .auto_shrink([false, true])
                            .show(ui, label);
                    }
                    Host::Chat { session, item } => {
                        let area = ScrollArea::both()
                            .id_salt(("chat-code", session.0, *item, ordinal))
                            .auto_shrink([false, true]);
                        match metrics.code_max_height {
                            Some(height) => area.max_height(height),
                            None => area,
                        }
                        .show(ui, label);
                    }
                }
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
        let body = self.metrics.body;
        self.widgets += 1;
        let ordinal = self.widgets;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let grid = |renderer: &mut Self, ui: &mut egui::Ui, grid: egui::Grid| {
            grid.striped(true).spacing([16.0, 5.0]).show(ui, |ui| {
                let align = |column: usize| match alignments.get(column) {
                    Some(ColumnAlign::Center) => Align::Center,
                    Some(ColumnAlign::Right) => Align::Max,
                    _ => Align::Min,
                };
                for (column, cell) in head.iter().enumerate() {
                    let mut style = renderer.style(body);
                    style.strong = true;
                    style.color = colors.text;
                    ui.with_layout(Layout::top_down(align(column)), |ui| {
                        renderer.flow(ui, cell, style);
                    });
                }
                ui.end_row();
                for row in rows {
                    for (column, cell) in row.iter().enumerate() {
                        ui.with_layout(Layout::top_down(align(column)), |ui| {
                            renderer.flow(ui, cell, renderer.style(body));
                        });
                    }
                    ui.end_row();
                }
            });
        };
        let area = match &self.host {
            Host::Preview { document, .. } => ScrollArea::horizontal()
                .id_salt(("preview-table", *document, head.len(), rows.len()))
                .auto_shrink([false, true]),
            Host::Chat { session, item } => ScrollArea::horizontal()
                .id_salt(("chat-table", session.0, *item, ordinal))
                .auto_shrink([false, true]),
        };
        let table = match &self.host {
            Host::Preview { document, .. } => {
                egui::Grid::new(("preview-grid", *document, rows.len(), head.len()))
            }
            Host::Chat { session, item } => {
                egui::Grid::new(("chat-grid", session.0, *item, ordinal))
            }
        };
        area.show(ui, |ui| grid(self, ui, table));
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
                    self.commands.push(match &self.host {
                        Host::Preview { document, .. } => {
                            Command::OpenPreviewLink(*document, destination.clone())
                        }
                        Host::Chat { session, .. } => {
                            Command::OpenChatLink(*session, destination.clone())
                        }
                    });
                }
            }
            Inline::Image { destination, alt } => self.image(ui, destination, alt),
            _ => {}
        }
    }

    fn image(&mut self, ui: &mut egui::Ui, destination: &str, alt: &str) {
        let size = self.metrics.body;
        let fallback = |ui: &mut egui::Ui, colors: &theme::Palette| {
            ui.label(
                RichText::new(if alt.is_empty() { destination } else { alt })
                    .size(size)
                    .italics()
                    .color(colors.muted),
            )
            .on_hover_text(destination);
        };
        let Host::Preview {
            source_path, state, ..
        } = &mut self.host
        else {
            // Chats never load images: a reply cannot make Lumen read files.
            fallback(ui, &self.colors);
            return;
        };
        // Remote images stay off: no network access from a document preview.
        let LinkTarget::File(relative) = LinkTarget::classify(destination) else {
            fallback(ui, &self.colors);
            return;
        };
        let Some(path) = source_path.and_then(|source| resolve_relative(source, &relative)) else {
            fallback(ui, &self.colors);
            return;
        };
        match state.image(&path) {
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

/// The language name and a copy button above a chat code block.
fn code_header(ui: &mut egui::Ui, language: &str, text: &str, colors: &theme::Palette) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if language.is_empty() {
                "code"
            } else {
                language
            })
            .size(10.0)
            .color(colors.muted),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button(icon::COPY)
                .on_hover_text("Copy code")
                .clicked()
            {
                ui.ctx().copy_text(text.to_owned());
            }
        });
    });
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
    size: f32,
    visuals: &egui::Visuals,
    default: Color32,
) -> LayoutJob {
    use std::hash::{Hash, Hasher};
    let font = FontId::monospace(size);
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

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::SessionId;

    const WIDTH: f32 = 240.0;

    /// One frame of `messages` (item id, Markdown) drawn as chat replies in a
    /// column `WIDTH` wide. Returns the output and how wide the content became.
    fn render(
        ctx: &egui::Context,
        messages: &[(&str, &str)],
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, f32) {
        let mut used = 0.0;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(WIDTH, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(Frame::NONE)
                    .show(ctx, |ui| {
                        let mut commands = Vec::new();
                        for (item, text) in messages {
                            show(
                                ui,
                                text,
                                Metrics::CHAT,
                                Host::Chat {
                                    session: SessionId(7),
                                    item,
                                },
                                false,
                                &mut commands,
                            );
                        }
                        used = ui.min_rect().width();
                    });
            },
        );
        (output, used)
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        ctx
    }

    /// Every text galley whose text starts with `needle`, with its screen rect.
    fn galleys(output: &egui::FullOutput, needle: &str) -> Vec<(Arc<egui::Galley>, egui::Rect)> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.starts_with(needle) => Some((
                    text.galley.clone(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn long_list_items_wrap_inside_a_narrow_column() {
        let ctx = context();
        let bullet = "This list item is far too long to fit on one line of a narrow agent panel";
        let text = format!("- {bullet}\n  - nested {bullet}\n1. numbered {bullet}");
        for _ in 0..2 {
            render(&ctx, &[("a", &text)], vec![]);
        }
        let (output, used) = render(&ctx, &[("a", &text)], vec![]);
        for needle in [bullet, "nested", "numbered"] {
            let found = galleys(&output, needle);
            assert_eq!(found.len(), 1, "{needle}");
            let (galley, rect) = &found[0];
            assert!(galley.rows.len() > 1, "{needle} wraps");
            assert!(
                rect.right() <= WIDTH + 0.5,
                "{needle} ends at {}",
                rect.right()
            );
        }
        assert!(used <= WIDTH + 0.5);
    }

    #[test]
    fn a_wide_table_scrolls_instead_of_widening_the_column() {
        let ctx = context();
        let text = "| Name | Description | Another very wide column |\n|---|---|---|\n\
                    | alpha | a long cell that keeps going and going | more text in the cell |";
        let (output, used) = render(&ctx, &[("a", text)], vec![]);
        assert!(used <= WIDTH + 0.5, "content is {used} wide");
        // The first column is shown; later ones are reached by scrolling.
        assert!(!galleys(&output, "Name").is_empty());
    }

    #[test]
    fn bold_text_is_brighter_than_body_text() {
        let ctx = context();
        let (output, _) = render(&ctx, &[("a", "plain **bold** _italic_")], vec![]);
        let (galley, _) = &galleys(&output, "plain ")[0];
        let colors = theme::palette(&ctx.style().visuals);
        let color_of = |needle: &str| {
            let start = galley.job.text.find(needle).unwrap();
            galley
                .job
                .sections
                .iter()
                .find(|section| section.byte_range.contains(&start))
                .unwrap()
                .format
                .clone()
        };
        assert_eq!(color_of("bold").color, colors.text);
        assert_ne!(color_of("plain").color, colors.text);
        assert!(color_of("italic").italics);
    }

    #[test]
    fn code_blocks_in_different_messages_scroll_independently() {
        let ctx = context();
        ctx.style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
        let code = format!("```\n{}\n```", "x".repeat(200));
        let messages = [("first", code.as_str()), ("second", code.as_str())];
        let (output, _) = render(&ctx, &messages, vec![]);
        let before = galleys(&output, "xxxx");
        assert_eq!(before.len(), 2);
        let over_first = egui::pos2(before[0].1.left() + 20.0, before[0].1.center().y);
        render(&ctx, &messages, vec![egui::Event::PointerMoved(over_first)]);
        render(
            &ctx,
            &messages,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(-120.0, 0.0),
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let (output, _) = render(&ctx, &messages, vec![]);
        let after = galleys(&output, "xxxx");
        assert!(
            after[0].1.left() < before[0].1.left() - 1.0,
            "the first block scrolled"
        );
        assert_eq!(after[1].1.left(), before[1].1.left(), "the second did not");
    }

    #[test]
    fn unfinished_streaming_markdown_still_renders() {
        let ctx = context();
        for partial in [
            "```rust\nfn main() {",
            "Some **bold",
            "| a | b |\n|---",
            "1. first\n2.",
            "[a link](src/ma",
        ] {
            let (output, _) = render(&ctx, &[("a", partial)], vec![]);
            assert!(!output.shapes.is_empty(), "{partial:?}");
        }
    }

    #[test]
    fn chat_links_become_chat_commands() {
        let ctx = context();
        let text = "See [main](src/main.rs:3).";
        let click = |events| {
            let mut commands = Vec::new();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(WIDTH, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        show(
                            ui,
                            text,
                            Metrics::CHAT,
                            Host::Chat {
                                session: SessionId(3),
                                item: "a",
                            },
                            false,
                            &mut commands,
                        );
                    });
                },
            );
            (output, commands)
        };
        let (output, _) = click(vec![]);
        let link = galleys(&output, "main")[0].1.center();
        click(vec![egui::Event::PointerMoved(link)]);
        let press = |pressed| egui::Event::PointerButton {
            pos: link,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        click(vec![press(true)]);
        let (_, commands) = click(vec![press(false)]);
        assert_eq!(
            commands,
            vec![Command::OpenChatLink(SessionId(3), "src/main.rs:3".into())]
        );
    }
}
