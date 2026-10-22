//! CommonMark/GFM parsing for the Markdown preview. Produces a block/inline tree
//! with no UI or I/O dependencies. Raw HTML is kept as inert text and never
//! interpreted. `markdown_lite` remains the tiny streaming subset for chat.
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link {
        destination: String,
        content: Vec<Inline>,
    },
    Image {
        destination: String,
        alt: String,
    },
    SoftBreak,
    HardBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColumnAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListItem {
    /// `Some(checked)` for a GFM task list item.
    pub(crate) task: Option<bool>,
    pub(crate) blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Heading {
        level: u8,
        /// GitHub-style slug used by `#anchor` links; unique within a document.
        anchor: String,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Quote(Vec<Block>),
    Code {
        language: String,
        text: String,
    },
    List {
        /// First number of an ordered list; `None` for a bullet list.
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Rule,
    Table {
        alignments: Vec<ColumnAlign>,
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// Raw HTML, shown verbatim as text.
    Html(String),
}

/// Where a link in a preview leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkTarget {
    External(String),
    Anchor(String),
    /// A path relative to the source file, without query or fragment.
    File(String),
}

impl LinkTarget {
    pub(crate) fn classify(destination: &str) -> Self {
        let destination = destination.trim();
        if let Some(anchor) = destination.strip_prefix('#') {
            return Self::Anchor(anchor.to_owned());
        }
        if has_url_scheme(destination) {
            return Self::External(destination.to_owned());
        }
        let path = destination.split(['#', '?']).next().unwrap_or(destination);
        Self::File(percent_decode(path))
    }
}

/// `scheme:` per RFC 3986, so `C:\x` style drive paths and `a/b:c` are not URLs
/// (single-letter schemes are treated as drive letters).
fn has_url_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once(':') else {
        return false;
    };
    scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(value) = bytes
                .get(index + 1..index + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(value);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// GitHub-style heading slug: lowercase, spaces to `-`, punctuation dropped.
pub(crate) fn slug(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c.to_lowercase().collect::<String>())
            } else if c.is_whitespace() {
                Some("-".into())
            } else {
                None
            }
        })
        .collect()
}

/// Plain text of inline content, used for slugs and image alternatives.
pub(crate) fn plain_text(inlines: &[Inline]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(value) | Inline::Code(value) => text.push_str(value),
            Inline::Emphasis(inner)
            | Inline::Strong(inner)
            | Inline::Strikethrough(inner)
            | Inline::Link { content: inner, .. } => text.push_str(&plain_text(inner)),
            Inline::Image { alt, .. } => text.push_str(alt),
            Inline::SoftBreak | Inline::HardBreak => text.push(' '),
        }
    }
    text
}

pub(crate) fn parse(source: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES;
    let mut parser = Builder {
        events: Parser::new_ext(source, options).peekable(),
        slugs: HashMap::new(),
    };
    parser.blocks(None)
}

struct Builder<'a> {
    events: std::iter::Peekable<Parser<'a>>,
    slugs: HashMap<String, usize>,
}

impl Builder<'_> {
    /// Blocks up to (and consuming) the `end` tag, or to the end of input.
    fn blocks(&mut self, end: Option<TagEnd>) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut loose: Vec<Inline> = Vec::new();
        while let Some(event) = self.events.next() {
            match event {
                Event::End(tag) if Some(tag) == end => break,
                Event::End(_) => {}
                Event::Start(tag) => {
                    flush_paragraph(&mut loose, &mut blocks);
                    if let Some(block) = self.block(tag) {
                        blocks.push(block);
                    }
                }
                Event::Rule => {
                    flush_paragraph(&mut loose, &mut blocks);
                    blocks.push(Block::Rule);
                }
                Event::Html(html) | Event::InlineHtml(html) => {
                    flush_paragraph(&mut loose, &mut blocks);
                    blocks.push(Block::Html(html.trim_end().to_owned()));
                }
                other => {
                    // Tight list items carry their text without a paragraph.
                    if let Some(inline) = self.leaf_inline(other) {
                        loose.push(inline);
                    }
                }
            }
        }
        flush_paragraph(&mut loose, &mut blocks);
        blocks
    }

    fn block(&mut self, tag: Tag<'_>) -> Option<Block> {
        Some(match tag {
            Tag::Paragraph => Block::Paragraph(self.inlines(TagEnd::Paragraph)),
            Tag::Heading { level, .. } => {
                let content = self.inlines(TagEnd::Heading(level));
                let base = slug(&plain_text(&content));
                let count = self.slugs.entry(base.clone()).or_insert(0);
                let anchor = if *count == 0 {
                    base
                } else {
                    format!("{base}-{count}")
                };
                *count += 1;
                Block::Heading {
                    level: heading_level(level),
                    anchor,
                    content,
                }
            }
            Tag::BlockQuote(kind) => Block::Quote(self.blocks(Some(TagEnd::BlockQuote(kind)))),
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                let mut text = String::new();
                for event in self.events.by_ref() {
                    match event {
                        Event::End(TagEnd::CodeBlock) => break,
                        Event::Text(value) => text.push_str(&value),
                        _ => {}
                    }
                }
                if text.ends_with('\n') {
                    text.pop();
                }
                Block::Code { language, text }
            }
            Tag::List(start) => {
                let mut items = Vec::new();
                while let Some(event) = self.events.next() {
                    match event {
                        Event::End(TagEnd::List(_)) => break,
                        Event::Start(Tag::Item) => items.push(self.item()),
                        _ => {}
                    }
                }
                Block::List { start, items }
            }
            Tag::Table(alignments) => self.table(&alignments),
            Tag::HtmlBlock => {
                let mut html = String::new();
                for event in self.events.by_ref() {
                    match event {
                        Event::End(TagEnd::HtmlBlock) => break,
                        Event::Html(value) | Event::Text(value) => html.push_str(&value),
                        _ => {}
                    }
                }
                Block::Html(html.trim_end().to_owned())
            }
            other => {
                // Unsupported containers (footnotes, definition lists, metadata):
                // keep their text rather than dropping content.
                let end = other.to_end();
                let blocks = self.blocks(Some(end));
                return blocks.into_iter().next();
            }
        })
    }

    fn item(&mut self) -> ListItem {
        let task = match self.events.peek() {
            Some(Event::TaskListMarker(checked)) => Some(*checked),
            _ => None,
        };
        if task.is_some() {
            self.events.next();
        }
        ListItem {
            task,
            blocks: self.blocks(Some(TagEnd::Item)),
        }
    }

    fn table(&mut self, alignments: &[Alignment]) -> Block {
        let mut head = Vec::new();
        let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
        let mut in_head = false;
        while let Some(event) = self.events.next() {
            match event {
                Event::End(TagEnd::Table) => break,
                Event::Start(Tag::TableHead) => in_head = true,
                Event::End(TagEnd::TableHead) => in_head = false,
                Event::Start(Tag::TableRow) => rows.push(Vec::new()),
                Event::Start(Tag::TableCell) => {
                    let cell = self.inlines(TagEnd::TableCell);
                    if in_head {
                        head.push(cell);
                    } else if let Some(row) = rows.last_mut() {
                        row.push(cell);
                    }
                }
                _ => {}
            }
        }
        Block::Table {
            alignments: alignments
                .iter()
                .map(|alignment| match alignment {
                    Alignment::Center => ColumnAlign::Center,
                    Alignment::Right => ColumnAlign::Right,
                    Alignment::Left | Alignment::None => ColumnAlign::Left,
                })
                .collect(),
            head,
            rows,
        }
    }

    /// Inline content up to (and consuming) `end`.
    fn inlines(&mut self, end: TagEnd) -> Vec<Inline> {
        let mut inlines = Vec::new();
        while let Some(event) = self.events.next() {
            match event {
                Event::End(tag) if tag == end => break,
                Event::Start(tag) => {
                    if let Some(inline) = self.inline_span(tag) {
                        inlines.push(inline);
                    }
                }
                Event::End(_) => {}
                other => {
                    if let Some(inline) = self.leaf_inline(other) {
                        inlines.push(inline);
                    }
                }
            }
        }
        inlines
    }

    fn inline_span(&mut self, tag: Tag<'_>) -> Option<Inline> {
        Some(match tag {
            Tag::Emphasis => Inline::Emphasis(self.inlines(TagEnd::Emphasis)),
            Tag::Strong => Inline::Strong(self.inlines(TagEnd::Strong)),
            Tag::Strikethrough => Inline::Strikethrough(self.inlines(TagEnd::Strikethrough)),
            Tag::Link { dest_url, .. } => Inline::Link {
                destination: dest_url.into_string(),
                content: self.inlines(TagEnd::Link),
            },
            Tag::Image { dest_url, .. } => {
                let alt = plain_text(&self.inlines(TagEnd::Image));
                Inline::Image {
                    destination: dest_url.into_string(),
                    alt,
                }
            }
            other => {
                // An unexpected container inside inline content: keep its text.
                let content = self.inlines(other.to_end());
                Inline::Emphasis(content)
            }
        })
    }

    fn leaf_inline(&mut self, event: Event<'_>) -> Option<Inline> {
        Some(match event {
            Event::Text(text) => Inline::Text(text.into_string()),
            Event::Code(code) => Inline::Code(code.into_string()),
            Event::InlineHtml(html) | Event::Html(html) => Inline::Text(html.into_string()),
            Event::SoftBreak => Inline::SoftBreak,
            Event::HardBreak => Inline::HardBreak,
            Event::FootnoteReference(label) => Inline::Text(format!("[{label}]")),
            Event::InlineMath(text) | Event::DisplayMath(text) => Inline::Code(text.into_string()),
            Event::TaskListMarker(_) | Event::Rule | Event::Start(_) | Event::End(_) => {
                return None;
            }
        })
    }
}

fn flush_paragraph(loose: &mut Vec<Inline>, blocks: &mut Vec<Block>) {
    if !loose.is_empty() {
        blocks.push(Block::Paragraph(std::mem::take(loose)));
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Inline {
        Inline::Text(value.into())
    }

    #[test]
    fn headings_get_unique_github_style_anchors() {
        let blocks = parse("# Hello, World!\n\n## Hello, World!\n");
        let anchors: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::Heading { anchor, .. } => Some(anchor.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(anchors, ["hello-world", "hello-world-1"]);
    }

    #[test]
    fn paragraph_with_inline_styles_links_and_images() {
        let blocks = parse("a *b* **c** ~~d~~ `e` [f](http://x.io) ![alt](img.png)\n");
        let [Block::Paragraph(inlines)] = blocks.as_slice() else {
            panic!("{blocks:?}");
        };
        assert_eq!(inlines[0], text("a "));
        assert_eq!(inlines[1], Inline::Emphasis(vec![text("b")]));
        assert_eq!(inlines[3], Inline::Strong(vec![text("c")]));
        assert_eq!(inlines[5], Inline::Strikethrough(vec![text("d")]));
        assert_eq!(inlines[7], Inline::Code("e".into()));
        assert_eq!(
            inlines[9],
            Inline::Link {
                destination: "http://x.io".into(),
                content: vec![text("f")]
            }
        );
        assert_eq!(
            inlines[11],
            Inline::Image {
                destination: "img.png".into(),
                alt: "alt".into()
            }
        );
    }

    #[test]
    fn fenced_and_indented_code_keep_language_and_text() {
        let blocks = parse("```rust title=x\nfn main() {}\n```\n\n    plain\n");
        assert_eq!(
            blocks,
            [
                Block::Code {
                    language: "rust".into(),
                    text: "fn main() {}".into()
                },
                Block::Code {
                    language: String::new(),
                    text: "plain".into()
                }
            ]
        );
    }

    #[test]
    fn nested_lists_and_task_items() {
        let blocks = parse("- [x] done\n- [ ] todo\n  1. one\n  2. two\n");
        let [Block::List { start: None, items }] = blocks.as_slice() else {
            panic!("{blocks:?}");
        };
        assert_eq!(items[0].task, Some(true));
        assert_eq!(items[0].blocks, [Block::Paragraph(vec![text("done")])]);
        assert_eq!(items[1].task, Some(false));
        let Block::List {
            start,
            items: inner,
        } = &items[1].blocks[1]
        else {
            panic!("{:?}", items[1]);
        };
        assert_eq!(*start, Some(1));
        assert_eq!(inner.len(), 2);
    }

    #[test]
    fn block_quote_rule_and_table() {
        let blocks = parse("> quoted\n\n---\n\n| a | b |\n|:-:|--:|\n| 1 | 2 |\n");
        assert_eq!(
            blocks[0],
            Block::Quote(vec![Block::Paragraph(vec![text("quoted")])])
        );
        assert_eq!(blocks[1], Block::Rule);
        let Block::Table {
            alignments,
            head,
            rows,
        } = &blocks[2]
        else {
            panic!("{blocks:?}");
        };
        assert_eq!(alignments, &[ColumnAlign::Center, ColumnAlign::Right]);
        assert_eq!(head, &[vec![text("a")], vec![text("b")]]);
        assert_eq!(rows, &[vec![vec![text("1")], vec![text("2")]]]);
    }

    #[test]
    fn raw_html_is_kept_as_inert_text() {
        let blocks = parse("<div>hi</div>\n\nx <b>y</b>\n");
        assert_eq!(blocks[0], Block::Html("<div>hi</div>".into()));
        let Block::Paragraph(inlines) = &blocks[1] else {
            panic!("{blocks:?}");
        };
        assert!(inlines.contains(&text("<b>")));
    }

    #[test]
    fn malformed_and_unfinished_input_never_panics() {
        for source in [
            "",
            "```rust\nunterminated",
            "| a | b\n|--",
            "[broken](",
            "**open *nested",
            "- \n  - \n    >",
            "![x",
            "\u{0}\r\n\t",
        ] {
            let _ = parse(source);
        }
        assert_eq!(
            parse("```\nopen"),
            [Block::Code {
                language: String::new(),
                text: "open".into()
            }]
        );
    }

    #[test]
    fn link_targets_are_classified() {
        assert_eq!(
            LinkTarget::classify("https://a.io/x#y"),
            LinkTarget::External("https://a.io/x#y".into())
        );
        assert_eq!(
            LinkTarget::classify("mailto:a@b.c"),
            LinkTarget::External("mailto:a@b.c".into())
        );
        assert_eq!(
            LinkTarget::classify("#my-heading"),
            LinkTarget::Anchor("my-heading".into())
        );
        assert_eq!(
            LinkTarget::classify("docs/My%20File.md#top"),
            LinkTarget::File("docs/My File.md".into())
        );
        assert_eq!(
            LinkTarget::classify("C:\\notes.md"),
            LinkTarget::File("C:\\notes.md".into())
        );
    }
}
