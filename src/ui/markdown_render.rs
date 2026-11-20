//! Markdown block rendering shared by the Markdown preview tab and agent chat
//! replies. The two differ only in their [`Metrics`] (sizes, spacing and how code
//! behaves) and their [`Host`] (what links, images and widget ids do), so a reply
//! shows the same tables, nested lists, links and highlighted code as a
//! previewed document.
//!
//! Nothing here may grow wider than the column it is given: text wraps (long
//! words break), tables wrap their cells and only scroll when even their
//! narrowest layout does not fit, and code either wraps (chat) or scrolls inside
//! its own block with a visible bar (preview). Every style change is scoped to
//! the block that needs it.
use super::{theme, widgets};
use crate::{
    app::{
        command::Command,
        preview::{PreviewImage, PreviewState, resolve_relative},
    },
    icon,
    model::markdown::{self, Block, ColumnAlign, Inline, LinkTarget, ListItem},
};
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense,
    Stroke, TextFormat, Vec2,
    text::{LayoutJob, TextWrapping},
    util::cache::{ComputerMut, FrameCache},
};
use std::{ops::Range, path::Path, sync::Arc};

/// Text sizes and spacing for one rendering surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Metrics {
    pub(super) body: f32,
    pub(super) code: f32,
    /// Heading sizes for levels 1-4; 5 and 6 use the body size.
    pub(super) headings: [f32; 4],
    /// Underline level 1-2 headings, as a document does.
    pub(super) heading_rules: bool,
    /// Line height as a multiple of the text size.
    pub(super) line_height: f32,
    /// Space between sibling blocks.
    pub(super) gap: f32,
    /// Space between blocks inside a list item, and between list items.
    pub(super) list_gap: f32,
    /// Space above a heading that follows other content.
    pub(super) heading_gap: f32,
    /// Horizontal and vertical padding inside table cells.
    pub(super) cell_padding: [i8; 2],
    /// Code wraps to the column and shows whole (chat); otherwise it keeps its
    /// lines and scrolls sideways inside its block (preview).
    pub(super) wrap_code: bool,
    /// A code block inside a list item uses the item's outer width (chat), so a
    /// narrow column does not squeeze it further.
    pub(super) dedent_list_code: bool,
}

impl Metrics {
    /// The Markdown preview tab: a readable document, laid out like VS Code's
    /// preview (14 px text on a 22 px line, 16 px between blocks).
    pub(super) const PREVIEW: Self = Self {
        body: 14.0,
        code: 13.0,
        headings: [28.0, 21.0, 17.5, 14.0],
        heading_rules: true,
        line_height: 22.0 / 14.0,
        gap: 16.0,
        list_gap: 4.0,
        heading_gap: 24.0,
        cell_padding: [10, 5],
        wrap_code: false,
        dedent_list_code: false,
    };

    /// Agent replies in the chat column: compact, like VS Code's chat (13 px text
    /// at 1.5 line height); code wraps and never takes over the conversation.
    pub(super) const CHAT: Self = Self {
        body: 13.0,
        code: 12.0,
        headings: [20.0, 16.0, 14.0, 13.0],
        heading_rules: false,
        line_height: 1.5,
        gap: 12.0,
        list_gap: 4.0,
        heading_gap: 16.0,
        cell_padding: [8, 4],
        wrap_code: true,
        dedent_list_code: true,
    };

    /// An agent's working notes between steps: one step quieter than a reply.
    pub(super) const CHAT_NOTES: Self = Self {
        body: 12.0,
        code: 11.5,
        headings: [13.5, 12.5, 12.0, 12.0],
        gap: 8.0,
        heading_gap: 10.0,
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
    ui.scope(|ui| renderer.blocks(ui, &blocks, metrics.body));
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
    /// How deeply nested the list being drawn is (0 outside lists).
    depth: usize,
}

#[derive(Clone, Copy)]
struct Style {
    size: f32,
    /// Line height as a multiple of `size`.
    line: f32,
    color: Color32,
    strong: bool,
    italics: bool,
    strike: bool,
}

/// A link inside a laid-out paragraph: its byte range in the job's text.
struct LinkSpan {
    range: Range<usize>,
    destination: String,
}

/// One part of a list item: ordinary blocks in the item's column, or a code
/// block drawn at the list's own width.
enum ItemPart<'b> {
    Indented(&'b [Block]),
    Outdented(&'b Block),
}

/// Width of a list item's marker column (the marker plus the space after it).
const MARKER_COLUMN: f32 = 26.0;
/// Narrowest a table column becomes before the table scrolls instead.
const MIN_COLUMN: f32 = 48.0;
/// How long the copy button says "Copied".
const COPIED_SECONDS: f64 = 1.2;

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
            depth: 0,
        }
    }

    fn style(&self, size: f32) -> Style {
        Style {
            size,
            line: self.metrics.line_height,
            color: self.body,
            strong: false,
            italics: false,
            strike: false,
        }
    }

    /// Height of one line of body text, in points.
    fn line_px(&self, size: f32) -> f32 {
        (size * self.metrics.line_height).round()
    }

    pub(super) fn blocks(&mut self, ui: &mut egui::Ui, blocks: &[Block], size: f32) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut previous: Option<&Block> = None;
        for block in blocks {
            if let Some(previous) = previous {
                ui.add_space(self.gap_between(previous, block));
            }
            self.block(ui, block, size);
            previous = Some(block);
        }
    }

    /// Headings get room above them and sit close to what they introduce.
    fn gap_between(&self, previous: &Block, next: &Block) -> f32 {
        let top_level = self.depth == 0;
        match (previous, next) {
            (_, Block::Heading { .. }) if top_level => self.metrics.heading_gap,
            (Block::Heading { .. }, _) => (self.gap * 0.75).round(),
            _ => self.gap,
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
            Block::Quote(inner) => self.quote(ui, inner, size),
            Block::Code { language, text } => self.code_block(ui, language, text),
            Block::List { start, items } => self.list(ui, *start, items, size),
            Block::Rule => {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 9.0), Sense::hover());
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
            } => {
                ui.scope(|ui| self.table(ui, alignments, head, rows));
            }
            Block::Html(html) => {
                let mut job = LayoutJob::default();
                job.append(
                    html,
                    0.0,
                    self.code_format(self.metrics.code, self.colors.muted),
                );
                ui.add(egui::Label::new(job).wrap().selectable(true));
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
        style.line = 1.3;
        style.color = if level == 6 {
            self.colors.muted
        } else {
            self.colors.text
        };
        let response = self.flow(ui, content, style);
        if let Host::Preview {
            anchor: requested, ..
        } = &mut self.host
            && requested.as_deref() == Some(anchor)
        {
            response.scroll_to_me(Some(Align::TOP));
            **requested = None;
        }
        if self.metrics.heading_rules && level <= 2 {
            ui.add_space((size * 0.3).round());
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, self.colors.border);
        }
    }

    /// A quoted block: an accent bar at the left over a faint tint.
    fn quote(&mut self, ui: &mut egui::Ui, inner: &[Block], size: f32) {
        let response = Frame::NONE
            .fill(self.colors.accent.linear_multiply(0.06))
            .corner_radius(2)
            .inner_margin(Margin {
                left: 14,
                right: 8,
                top: 4,
                bottom: 4,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.blocks(ui, inner, size);
            });
        let rect = response.response.rect;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 3.0, rect.bottom())),
            CornerRadius::same(1),
            self.colors.accent.linear_multiply(0.6),
        );
    }

    fn list(&mut self, ui: &mut egui::Ui, start: Option<u64>, items: &[ListItem], size: f32) {
        self.depth += 1;
        for (position, item) in items.iter().enumerate() {
            if position > 0 {
                ui.add_space(self.metrics.list_gap);
            }
            for (part_index, part) in self.item_parts(&item.blocks).into_iter().enumerate() {
                if part_index > 0 {
                    ui.add_space(self.metrics.list_gap);
                }
                match part {
                    ItemPart::Indented(blocks) => {
                        let first = part_index == 0;
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let (marker, _) = ui.allocate_exact_size(
                                egui::vec2(MARKER_COLUMN, self.line_px(size)),
                                Sense::hover(),
                            );
                            if first {
                                self.marker(
                                    ui,
                                    marker,
                                    start.map(|n| n + position as u64),
                                    item,
                                    size,
                                );
                            }
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width());
                                let outer = std::mem::replace(&mut self.gap, self.metrics.list_gap);
                                self.blocks(ui, blocks, size);
                                self.gap = outer;
                            });
                        });
                    }
                    ItemPart::Outdented(block) => self.block(ui, block, size),
                }
            }
        }
        self.depth -= 1;
    }

    /// Splits an item so its code blocks can use the list's full width. Without
    /// dedenting, the item is one indented part.
    fn item_parts<'b>(&self, blocks: &'b [Block]) -> Vec<ItemPart<'b>> {
        if !self.metrics.dedent_list_code {
            return vec![ItemPart::Indented(blocks)];
        }
        let mut parts = Vec::new();
        let mut start = 0;
        for (index, block) in blocks.iter().enumerate() {
            if matches!(block, Block::Code { .. }) {
                // The first part always exists, so the marker has a row to sit on.
                if index > start || parts.is_empty() {
                    parts.push(ItemPart::Indented(&blocks[start..index]));
                }
                parts.push(ItemPart::Outdented(block));
                start = index + 1;
            }
        }
        if start < blocks.len() || parts.is_empty() {
            parts.push(ItemPart::Indented(&blocks[start..]));
        }
        parts
    }

    /// The bullet, number or task box of a list item, centred on its first line.
    fn marker(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        number: Option<u64>,
        item: &ListItem,
        size: f32,
    ) {
        let font = FontId::proportional(size);
        let row = ui.fonts_mut(|fonts| fonts.row_height(&font));
        let center = egui::pos2(rect.left() + 10.0, rect.top() + row / 2.0);
        let painter = ui.painter();
        let muted = self.colors.muted;
        if let Some(checked) = item.task {
            let rect = egui::Rect::from_center_size(center, Vec2::splat(12.0));
            let (fill, stroke) = if checked {
                (self.colors.accent, self.colors.accent)
            } else {
                (Color32::TRANSPARENT, muted)
            };
            painter.rect(
                rect,
                3.0,
                fill,
                Stroke::new(1.0_f32, stroke),
                egui::StrokeKind::Inside,
            );
            if checked {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon::CHECK,
                    FontId::proportional(10.0),
                    self.colors.on_accent,
                );
            }
        } else if let Some(number) = number {
            painter.text(
                egui::pos2(rect.left() + 18.0, center.y),
                egui::Align2::RIGHT_CENTER,
                format!("{number}."),
                font,
                muted,
            );
        } else {
            match self.depth {
                1 => {
                    painter.circle_filled(center, 2.5, muted);
                }
                2 => {
                    painter.circle_stroke(center, 2.5, Stroke::new(1.0_f32, muted));
                }
                _ => {
                    painter.rect_filled(
                        egui::Rect::from_center_size(center, Vec2::splat(4.5)),
                        0.0,
                        muted,
                    );
                }
            }
        }
    }

    fn code_block(&mut self, ui: &mut egui::Ui, language: &str, text: &str) {
        self.widgets += 1;
        let id = match &self.host {
            Host::Preview { document, .. } => {
                egui::Id::new(("preview-code", *document, text.len(), language))
            }
            Host::Chat { session, item } => {
                egui::Id::new(("chat-code", session.0, *item, self.widgets))
            }
        };
        let colors = self.colors;
        let metrics = self.metrics;
        let code = text.strip_suffix('\n').unwrap_or(text);
        let response = Frame::default()
            .fill(colors.panel)
            .stroke(Stroke::new(1.0_f32, colors.border))
            .corner_radius(4)
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let job = highlighted_job(
                    ui.ctx(),
                    code,
                    language,
                    metrics.code,
                    (metrics.code * 1.45).round(),
                    ui.visuals(),
                    colors.text,
                );
                if metrics.wrap_code {
                    ui.add(egui::Label::new(job).wrap().selectable(true));
                } else {
                    // A visible bar says there is more to the right.
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    ScrollArea::horizontal()
                        .id_salt(id)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(job).extend().selectable(true));
                        });
                }
            });
        copy_button(ui, response.response.rect, id, code, &colors);
    }

    fn table(
        &mut self,
        ui: &mut egui::Ui,
        alignments: &[ColumnAlign],
        head: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
    ) {
        self.widgets += 1;
        let id = match &self.host {
            Host::Preview { document, .. } => {
                egui::Id::new(("preview-table", *document, head.len(), rows.len()))
            }
            Host::Chat { session, item } => {
                egui::Id::new(("chat-table", session.0, *item, self.widgets))
            }
        };
        let columns = rows
            .iter()
            .map(Vec::len)
            .chain([head.len()])
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return;
        }
        let [pad_x, _] = self.metrics.cell_padding;
        let padding = 2.0 * f32::from(pad_x);
        let mut natural = vec![0.0_f32; columns];
        let mut narrowest = vec![0.0_f32; columns];
        for (row_index, row) in std::iter::once(head)
            .chain(rows.iter().map(Vec::as_slice))
            .enumerate()
        {
            for (column, cell) in row.iter().enumerate() {
                let (wide, narrow) = self.measure_cell(ui, cell, row_index == 0);
                natural[column] = natural[column].max(wide + padding);
                narrowest[column] = narrowest[column].max(narrow + padding);
            }
        }
        let widths = column_widths(&natural, &narrowest, ui.available_width() - 1.0);
        let total: f32 = widths.iter().sum();
        if total <= ui.available_width() + 0.5 {
            self.table_grid(ui, alignments, head, rows, &widths);
        } else {
            // Even the narrowest layout does not fit: scroll inside the table, with
            // a bar that is always drawn so the hidden columns are discoverable.
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            ScrollArea::horizontal()
                .id_salt(id)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    self.table_grid(ui, alignments, head, rows, &widths)
                });
        }
    }

    /// Single-line width and longest-word width of a cell's text.
    fn measure_cell(&self, ui: &egui::Ui, cell: &[Inline], header: bool) -> (f32, f32) {
        let mut style = self.style(self.metrics.body);
        style.strong = header;
        let mut job = LayoutJob::default();
        self.append(&mut job, cell, style, &mut Vec::new());
        let text = job.text.clone();
        job.wrap = TextWrapping::no_max_width();
        let font = FontId::proportional(self.metrics.body);
        ui.fonts_mut(|fonts| {
            let wide = fonts.layout_job(job).size().x;
            let narrow = text
                .split_whitespace()
                .map(|word| {
                    fonts
                        .layout_no_wrap(word.to_owned(), font.clone(), Color32::WHITE)
                        .size()
                        .x
                })
                .fold(0.0_f32, f32::max);
            (wide, narrow.max(MIN_COLUMN).min(wide.max(MIN_COLUMN)))
        })
    }

    fn table_grid(
        &mut self,
        ui: &mut egui::Ui,
        alignments: &[ColumnAlign],
        head: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
        widths: &[f32],
    ) {
        let colors = self.colors;
        let [pad_x, pad_y] = self.metrics.cell_padding;
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let header_fill = ui.painter().add(egui::Shape::Noop);
        let left = ui.cursor().left();
        let top = ui.cursor().top();
        let mut row_bottoms = Vec::new();
        for (row_index, row) in std::iter::once(head)
            .chain(rows.iter().map(Vec::as_slice))
            .enumerate()
        {
            let response = ui.horizontal_top(|ui| {
                for (column, width) in widths.iter().enumerate() {
                    let align = match alignments.get(column) {
                        Some(ColumnAlign::Center) => Align::Center,
                        Some(ColumnAlign::Right) => Align::Max,
                        _ => Align::Min,
                    };
                    ui.allocate_ui_with_layout(
                        egui::vec2(*width, 0.0),
                        Layout::top_down(align),
                        |ui| {
                            ui.set_width(*width);
                            Frame::NONE
                                .inner_margin(Margin::symmetric(pad_x, pad_y))
                                .show(ui, |ui| {
                                    ui.set_width(width - 2.0 * f32::from(pad_x));
                                    if let Some(cell) = row.get(column) {
                                        let mut style = self.style(self.metrics.body);
                                        if row_index == 0 {
                                            style.strong = true;
                                            style.color = colors.text;
                                        }
                                        self.flow(ui, cell, style);
                                    }
                                });
                        },
                    );
                }
            });
            row_bottoms.push(response.response.rect.bottom());
        }
        let right = left + widths.iter().sum::<f32>();
        let bottom = row_bottoms.last().copied().unwrap_or(top);
        let outline = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom));
        let radius = CornerRadius::same(4);
        let painter = ui.painter();
        painter.set(
            header_fill,
            egui::Shape::rect_filled(
                egui::Rect::from_min_max(outline.min, egui::pos2(right, row_bottoms[0])),
                CornerRadius {
                    nw: 4,
                    ne: 4,
                    sw: 0,
                    se: 0,
                },
                colors.panel,
            ),
        );
        let line = Stroke::new(1.0_f32, colors.border);
        for y in row_bottoms.iter().take(row_bottoms.len().saturating_sub(1)) {
            painter.hline(left..=right, *y, line);
        }
        let mut x = left;
        for width in widths.iter().take(widths.len().saturating_sub(1)) {
            x += width;
            painter.vline(x, top..=bottom, line);
        }
        painter.rect_stroke(outline, radius, line, egui::StrokeKind::Inside);
    }

    /// Inline content as one wrapped label. Links are sections of the same
    /// text, so they wrap and select with it; they are hit-tested on hover and
    /// click. A paragraph with an image falls back to a row of widgets.
    fn flow(&mut self, ui: &mut egui::Ui, inlines: &[Inline], style: Style) -> egui::Response {
        if inlines.iter().any(has_image) {
            return self.flow_with_images(ui, inlines, style);
        }
        let mut job = LayoutJob::default();
        let mut links = Vec::new();
        self.append(&mut job, inlines, style, &mut links);
        let width = ui.available_width();
        let sense = if links.is_empty() {
            Sense::hover()
        } else {
            Sense::click()
        };
        let response = ui.add(
            egui::Label::new(job.clone())
                .wrap()
                .selectable(true)
                .sense(sense),
        );
        if !links.is_empty() {
            self.link_interaction(ui, &response, job, width, &links);
        }
        response
    }

    /// Hover feedback and clicks for the links inside a laid-out paragraph.
    fn link_interaction(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        mut job: LayoutJob,
        width: f32,
        links: &[LinkSpan],
    ) {
        let Some(pointer) = response.hover_pos() else {
            return;
        };
        job.wrap = TextWrapping::wrap_at_width(width);
        let text = job.text.clone();
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let local = pointer - response.rect.min;
        // Only a pointer actually over a glyph counts, not one past the line end.
        if !galley.rect.contains(local.to_pos2()) {
            return;
        }
        let cursor = galley.cursor_from_pos(local);
        let glyph = galley.pos_from_cursor(cursor);
        let byte = text
            .char_indices()
            .nth(cursor.index)
            .map_or(text.len(), |(byte, _)| byte);
        // A cursor sits between characters: test the character on the pointer's side.
        let byte = if local.x < glyph.left() && byte > 0 {
            text[..byte]
                .char_indices()
                .last()
                .map_or(0, |(start, _)| start)
        } else {
            byte
        };
        let Some(link) = links.iter().find(|link| link.range.contains(&byte)) else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        response.clone().on_hover_text_at_pointer(&link.destination);
        if response.clicked() {
            self.commands.push(match &self.host {
                Host::Preview { document, .. } => {
                    Command::OpenPreviewLink(*document, link.destination.clone())
                }
                Host::Chat { session, .. } => {
                    Command::OpenChatLink(*session, link.destination.clone())
                }
            });
        }
    }

    /// A paragraph with images: text runs, links and images as a wrapping row.
    fn flow_with_images(
        &mut self,
        ui: &mut egui::Ui,
        inlines: &[Inline],
        style: Style,
    ) -> egui::Response {
        let line = (style.size * style.line).round();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Rows as tall as a line of text, so wrapped rows keep their spacing.
            ui.spacing_mut().interact_size.y = line;
            let mut run: Vec<&Inline> = Vec::new();
            for inline in inlines {
                if matches!(inline, Inline::Link { .. } | Inline::Image { .. }) {
                    self.flush_run(ui, &mut run, style);
                    match inline {
                        Inline::Image { destination, alt } => self.image(ui, destination, alt),
                        other => {
                            self.flow(ui, std::slice::from_ref(other), style);
                        }
                    }
                } else {
                    run.push(inline);
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
            self.append(
                &mut job,
                std::slice::from_ref(inline),
                style,
                &mut Vec::new(),
            );
        }
        ui.add(egui::Label::new(job).selectable(true));
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

    fn append(
        &self,
        job: &mut LayoutJob,
        inlines: &[Inline],
        style: Style,
        links: &mut Vec<LinkSpan>,
    ) {
        for inline in inlines {
            match inline {
                Inline::Text(text) => job.append(text, 0.0, self.format(style)),
                Inline::Code(code) => {
                    let mut format = self.format(style);
                    format.font_id = FontId::monospace((style.size - 1.0).max(10.0));
                    format.background = self.colors.control_hover;
                    job.append(code, 0.0, format);
                }
                Inline::Emphasis(inner) => {
                    let mut style = style;
                    style.italics = true;
                    self.append(job, inner, style, links);
                }
                Inline::Strong(inner) => {
                    let mut style = style;
                    style.strong = true;
                    self.append(job, inner, style, links);
                }
                Inline::Strikethrough(inner) => {
                    let mut style = style;
                    style.strike = true;
                    self.append(job, inner, style, links);
                }
                Inline::Link {
                    destination,
                    content,
                } => {
                    let start = job.text.len();
                    let mut link = style;
                    link.color = self.colors.accent;
                    let first = job.sections.len();
                    self.append(job, content, link, links);
                    // Accent colour and a pointing hand mark a link, as in VS Code; an
                    // underline would sit at the bottom of the taller line, below the text.
                    for section in &mut job.sections[first..] {
                        section.format.color = self.colors.accent;
                    }
                    links.push(LinkSpan {
                        range: start..job.text.len(),
                        destination: destination.clone(),
                    });
                }
                Inline::Image { alt, .. } => job.append(alt, 0.0, self.format(style)),
                Inline::SoftBreak => job.append(" ", 0.0, self.format(style)),
                Inline::HardBreak => job.append("\n", 0.0, self.format(style)),
            }
        }
    }

    fn format(&self, style: Style) -> TextFormat {
        TextFormat {
            font_id: FontId::proportional(style.size),
            line_height: Some((style.size * style.line).round()),
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

    fn code_format(&self, size: f32, color: Color32) -> TextFormat {
        TextFormat {
            font_id: FontId::monospace(size),
            line_height: Some((size * 1.45).round()),
            color,
            ..Default::default()
        }
    }
}

/// Column widths for a table that must fit `available`: natural widths when
/// they fit, otherwise each column shrinks toward its narrowest in proportion to
/// how much it can give. Never narrower than the narrowest layout; the caller
/// scrolls when even that is too wide.
fn column_widths(natural: &[f32], narrowest: &[f32], available: f32) -> Vec<f32> {
    let wide: f32 = natural.iter().sum();
    if wide <= available {
        return natural.to_vec();
    }
    let narrow: f32 = narrowest.iter().sum();
    if narrow >= available {
        return narrowest.to_vec();
    }
    let slack = wide - narrow;
    let share = (available - narrow) / slack;
    natural
        .iter()
        .zip(narrowest)
        .map(|(wide, narrow)| narrow + (wide - narrow) * share)
        .collect()
}

/// Copy button at the top right of a code block. Shown on hover or keyboard
/// focus (it stays in the tab order while hidden), and says "Copied" for a
/// moment after a click.
fn copy_button(
    ui: &mut egui::Ui,
    block: egui::Rect,
    id: egui::Id,
    text: &str,
    colors: &theme::Palette,
) {
    let now = ui.input(|input| input.time);
    let copied_key = id.with("copied");
    let focus_key = id.with("copy_focus");
    let copied = ui
        .data(|data| data.get_temp::<f64>(copied_key))
        .is_some_and(|at| now - at < COPIED_SECONDS);
    if copied {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    let focused = ui
        .data(|data| data.get_temp::<bool>(focus_key))
        .unwrap_or(false);
    let visible = copied || focused || ui.rect_contains_pointer(block);
    let (label, color, width) = if copied {
        (format!("{} Copied", icon::CHECK), colors.success, 72.0)
    } else {
        (icon::COPY.to_owned(), colors.text, 24.0)
    };
    let rect = egui::Rect::from_min_size(
        egui::pos2(block.right() - 6.0 - width, block.top() + 6.0),
        egui::vec2(width, 22.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .id_salt(id.with("copy")),
    );
    child.set_opacity(if visible { 1.0 } else { 0.0 });
    child.painter().rect_filled(rect, 3.0, colors.panel);
    let response = widgets::tool_button(&mut child, false, 0.0, |ui| {
        ui.put(
            rect,
            egui::Button::new(RichText::new(label).size(12.0).color(color)).frame(false),
        )
    })
    .on_hover_text("Copy code");
    let response = widgets::named(response, true, "Copy code");
    if response.clicked() {
        ui.ctx().copy_text(text.to_owned());
        ui.data_mut(|data| data.insert_temp(copied_key, now));
    }
    let has_focus = response.has_focus();
    ui.data_mut(|data| data.insert_temp(focus_key, has_focus));
}

fn has_image(inline: &Inline) -> bool {
    match inline {
        Inline::Image { .. } => true,
        Inline::Emphasis(inner)
        | Inline::Strong(inner)
        | Inline::Strikethrough(inner)
        | Inline::Link { content: inner, .. } => inner.iter().any(has_image),
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
    line_height: f32,
    visuals: &egui::Visuals,
    default: Color32,
) -> LayoutJob {
    use std::hash::{Hash, Hasher};
    let font = FontId::monospace(size);
    let format = |color: Color32| TextFormat {
        font_id: font.clone(),
        line_height: Some(line_height),
        color,
        ..Default::default()
    };
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
            job.append(&text[cursor..range.start], 0.0, format(default));
        }
        job.append(&text[range.clone()], 0.0, format(*color));
        cursor = range.end;
    }
    job.append(&text[cursor..], 0.0, format(default));
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
    /// column `width` wide. Returns the output and how wide the content became.
    fn render_at(
        ctx: &egui::Context,
        width: f32,
        messages: &[(&str, &str)],
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, f32) {
        let mut used = 0.0;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 1200.0),
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

    fn render(
        ctx: &egui::Context,
        messages: &[(&str, &str)],
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, f32) {
        render_at(ctx, WIDTH, messages, events)
    }

    /// One frame of `text` as a previewed document `width` wide.
    fn render_preview(
        ctx: &egui::Context,
        document: crate::model::DocumentId,
        state: &mut PreviewState,
        text: &str,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(Frame::NONE)
                    .show(ctx, |ui| {
                        let mut commands = Vec::new();
                        let mut anchor = None;
                        let blocks = parsed(ui.ctx(), text);
                        let mut renderer = Renderer::new(
                            theme::palette(ui.visuals()),
                            Metrics::PREVIEW,
                            Host::Preview {
                                document,
                                source_path: None,
                                state,
                                anchor: &mut anchor,
                            },
                            &mut commands,
                        );
                        renderer.blocks(ui, &blocks, Metrics::PREVIEW.body);
                    });
            },
        )
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

    /// The right edge of the widest text drawn, wherever it is clipped.
    fn rightmost_text(output: &egui::FullOutput) -> f32 {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.pos.x + text.galley.rect.right()),
                _ => None,
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn nothing_is_drawn_past_the_chat_column() {
        let cases = [
            "See /Users/me/projects/lumen-editor/src/ui/markdown_render_with_a_long_name.rs for details.",
            "Docs: [https://code.visualstudio.com/docs/copilot/chat/chat-ask-mode-and-more](https://x.y)",
            "Read https://github.com/prasenjit-net/lumen-editor/blob/main/src/ui/markdown_render.rs now",
            "Call `crate::app::source_control::refresh_source_control_in_background` here.",
            "| Name | Description |\n|---|---|\n| refresh | Reads the repository status in the background every thirty seconds |\n",
            "1. First step\n2. Run this:\n\n   ```sh\n   cargo test --workspace -- --nocapture --test-threads=1\n   ```\n3. Done",
            "## A heading that is much too long for a narrow chat column to show",
            "> A quoted paragraph that is long enough to need wrapping in a narrow column.",
            "```\nfn a_function_with_a_long_signature(first: usize, second: usize) -> usize { first + second }\n```",
        ];
        for width in [WIDTH, 400.0] {
            for case in cases {
                let ctx = context();
                render_at(&ctx, width, &[("a", case)], vec![]);
                let (output, used) = render_at(&ctx, width, &[("a", case)], vec![]);
                assert!(used <= width + 0.5, "{width}: {case:?} is {used} wide");
                let right = rightmost_text(&output);
                assert!(right <= width + 0.5, "{width}: {case:?} draws to {right}");
            }
        }
    }

    #[test]
    fn text_after_a_table_still_wraps() {
        let ctx = context();
        let text = "| a | b |\n|---|---|\n| 1 | 2 |\n\nThis paragraph comes after a table and should still wrap inside the narrow column like any other.";
        render(&ctx, &[("a", text)], vec![]);
        let (output, used) = render(&ctx, &[("a", text)], vec![]);
        assert!(used <= WIDTH + 0.5, "content is {used} wide");
        let (galley, rect) = &galleys(&output, "This paragraph")[0];
        assert!(galley.rows.len() > 1);
        assert!(rect.right() <= WIDTH + 0.5);
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
    fn a_table_that_can_fit_wraps_its_cells_instead_of_scrolling() {
        let ctx = context();
        let text = "| Name | Description | Another very wide column |\n|---|---|---|\n\
                    | alpha | a long cell that keeps going and going | more text in the cell |";
        render(&ctx, &[("a", text)], vec![]);
        let (output, used) = render(&ctx, &[("a", text)], vec![]);
        assert!(used <= WIDTH + 0.5, "content is {used} wide");
        assert!(rightmost_text(&output) <= WIDTH + 0.5, "nothing hidden");
        let (galley, _) = &galleys(&output, "a long cell")[0];
        assert!(galley.rows.len() > 1, "the long cell wraps");
    }

    #[test]
    fn a_table_too_wide_to_wrap_scrolls_inside_itself() {
        let ctx = context();
        let word = "an_identifier_without_any_spaces";
        let text =
            format!("| {word} | {word} | {word} |\n|---|---|---|\n| {word} | {word} | {word} |");
        render(&ctx, &[("a", &text)], vec![]);
        let (output, used) = render(&ctx, &[("a", &text)], vec![]);
        assert!(used <= WIDTH + 0.5, "the column keeps its width: {used}");
        assert!(
            rightmost_text(&output) > WIDTH,
            "the rest is reached by scrolling"
        );
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
    fn chat_code_wraps_and_is_shown_whole() {
        let ctx = context();
        let long = format!("```\n{}\n```", "word ".repeat(80));
        render(&ctx, &[("a", &long)], vec![]);
        let (output, _) = render(&ctx, &[("a", &long)], vec![]);
        let (galley, rect) = &galleys(&output, "word")[0];
        assert!(galley.rows.len() > 1, "long code wraps");
        assert!(rect.right() <= WIDTH + 0.5);
        let tall: String = (0..40).map(|n| format!("line {n}\n")).collect();
        let tall = format!("```\n{tall}```");
        render(&ctx, &[("b", &tall)], vec![]);
        let (output, _) = render(&ctx, &[("b", &tall)], vec![]);
        let (galley, rect) = &galleys(&output, "line 0")[0];
        assert_eq!(galley.rows.len(), 40);
        assert!(rect.height() > 280.0, "no height cap: {}", rect.height());
    }

    #[test]
    fn preview_code_blocks_scroll_sideways_one_block_at_a_time() {
        let ctx = context();
        ctx.style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
        let document = crate::model::Document::from_path("/w/a.md".into(), String::new()).id;
        let mut state = PreviewState::default();
        let text = format!(
            "```\n{}\n```\n\n```\n{}\n```",
            "x".repeat(200),
            "x".repeat(201)
        );
        let mut frame = |events| render_preview(&ctx, document, &mut state, &text, 400.0, events);
        frame(vec![]);
        let output = frame(vec![]);
        let before = galleys(&output, "xxxx");
        assert_eq!(before.len(), 2);
        let over_first = egui::pos2(before[0].1.left() + 20.0, before[0].1.center().y);
        frame(vec![egui::Event::PointerMoved(over_first)]);
        frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(-120.0, 0.0),
            modifiers: egui::Modifiers::NONE,
        }]);
        let after = galleys(&frame(vec![]), "xxxx");
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
    fn links_inside_a_paragraph_point_and_click_through() {
        let ctx = context();
        let text = "See [main](src/main.rs:3) for details.";
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
        let (galley, rect) = &galleys(&output, "See ")[0];
        assert!(
            galley.job.text.starts_with("See main"),
            "one galley for the paragraph"
        );
        let start = galley.pos_from_cursor(egui::text::CCursor::new(4));
        let link = rect.min + start.center().to_vec2() + egui::vec2(6.0, 0.0);
        let plain = rect.min
            + galley
                .pos_from_cursor(egui::text::CCursor::new(1))
                .center()
                .to_vec2();
        let (hover, _) = click(vec![egui::Event::PointerMoved(link)]);
        assert_eq!(
            hover.platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        let press = |at, pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        click(vec![press(link, true)]);
        let (_, commands) = click(vec![press(link, false)]);
        assert_eq!(
            commands,
            vec![Command::OpenChatLink(SessionId(3), "src/main.rs:3".into())]
        );
        // Plain text in the same paragraph is not a link.
        click(vec![egui::Event::PointerMoved(plain)]);
        click(vec![press(plain, true)]);
        let (_, commands) = click(vec![press(plain, false)]);
        assert!(commands.is_empty(), "{commands:?}");
    }

    #[test]
    fn list_markers_sit_on_the_first_line_of_their_item() {
        let ctx = context();
        let text = "- plain item\n- [a link](x) item";
        render(&ctx, &[("a", text)], vec![]);
        let (output, _) = render(&ctx, &[("a", text)], vec![]);
        let markers: Vec<f32> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) => Some(circle.center.y),
                _ => None,
            })
            .collect();
        assert_eq!(markers.len(), 2);
        let row =
            ctx.fonts_mut(|fonts| fonts.row_height(&FontId::proportional(Metrics::CHAT.body)));
        for (marker, needle) in markers.iter().zip(["plain item", "a link"]) {
            let (_, rect) = &galleys(&output, needle)[0];
            let middle = rect.top() + row / 2.0;
            assert!(
                (marker - middle).abs() <= 1.5,
                "{needle}: marker {marker}, text {middle}"
            );
        }
    }

    #[test]
    fn the_copy_button_copies_the_code_and_says_so() {
        let ctx = context();
        let text = "```\nlet x = 1;\n```";
        render(&ctx, &[("a", text)], vec![]);
        let (output, _) = render(&ctx, &[("a", text)], vec![]);
        let colors = theme::palette(&ctx.style().visuals);
        let block = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == colors.panel && rect.stroke.width > 0.0 => {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("code block frame");
        let button = egui::pos2(block.right() - 18.0, block.top() + 17.0);
        render(
            &ctx,
            &[("a", text)],
            vec![egui::Event::PointerMoved(button)],
        );
        let press = |pressed| egui::Event::PointerButton {
            pos: button,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        render(&ctx, &[("a", text)], vec![press(true)]);
        let (output, _) = render(&ctx, &[("a", text)], vec![press(false)]);
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(
                    command,
                    egui::OutputCommand::CopyText(copied) if copied == "let x = 1;"
                ))
        );
        let (output, _) = render(&ctx, &[("a", text)], vec![]);
        assert!(!galleys(&output, &format!("{} Copied", icon::CHECK)).is_empty());
    }
}
