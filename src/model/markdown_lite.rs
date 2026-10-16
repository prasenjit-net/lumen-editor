//! A deliberately small Markdown subset for agent replies: paragraphs, headings,
//! bullet/numbered items, fenced code and inline `code` / **bold**. Anything else
//! is shown as plain text, so unfinished (still streaming) input never breaks.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Inline {
    Text(String),
    Code(String),
    Bold(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Paragraph(Vec<Inline>),
    Heading(Vec<Inline>),
    /// `marker` is `•` or the original number like `1.`.
    Item {
        marker: String,
        content: Vec<Inline>,
    },
    Code {
        language: String,
        text: String,
    },
}

pub(crate) fn parse(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut lines = source.lines();
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(inline(&paragraph.join("\n"))));
            paragraph.clear();
        }
    };
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            flush(&mut paragraph, &mut blocks);
            let mut code = Vec::new();
            // An unterminated fence is still streaming: show what has arrived.
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                code.push(inner);
            }
            blocks.push(Block::Code {
                language: rest.trim().to_owned(),
                text: code.join("\n"),
            });
        } else if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else if let Some(heading) = heading(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading(inline(heading)));
        } else if let Some((marker, content)) = list_item(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Item {
                marker,
                content: inline(content),
            });
        } else {
            paragraph.push(line);
        }
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

fn heading(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches('#');
    let hashes = line.len() - rest.len();
    ((1..=6).contains(&hashes) && rest.starts_with(' ')).then(|| rest.trim())
}

fn list_item(line: &str) -> Option<(String, &str)> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return Some(("•".to_owned(), rest));
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits)
        && let Some(rest) = line[digits..].strip_prefix(". ")
    {
        return Some((format!("{}.", &line[..digits]), rest));
    }
    None
}

/// `code` and **bold** spans; an unmatched marker stays literal text.
pub(crate) fn inline(text: &str) -> Vec<Inline> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let found = if let Some(after) = rest.strip_prefix('`') {
            after
                .find('`')
                .filter(|end| *end > 0)
                .map(|end| (Inline::Code(after[..end].to_owned()), 1 + end + 1))
        } else if let Some(after) = rest.strip_prefix("**") {
            after
                .find("**")
                .filter(|end| *end > 0)
                .map(|end| (Inline::Bold(after[..end].to_owned()), 2 + end + 2))
        } else {
            None
        };
        match found {
            Some((span, consumed)) => {
                if !plain.is_empty() {
                    spans.push(Inline::Text(std::mem::take(&mut plain)));
                }
                spans.push(span);
                rest = &rest[consumed..];
            }
            None => {
                let character = rest.chars().next().expect("non-empty");
                plain.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
    }
    if !plain.is_empty() {
        spans.push(Inline::Text(plain));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Inline {
        Inline::Text(value.to_owned())
    }

    #[test]
    fn paragraphs_headings_lists_and_code_split_into_blocks() {
        let blocks = parse(
            "# Title\nFirst line\nsecond line\n\n- one\n* two\n3. three\n```rust\nfn main() {}\n```\nafter",
        );
        assert_eq!(
            blocks,
            vec![
                Block::Heading(vec![text("Title")]),
                Block::Paragraph(vec![text("First line\nsecond line")]),
                Block::Item {
                    marker: "•".into(),
                    content: vec![text("one")]
                },
                Block::Item {
                    marker: "•".into(),
                    content: vec![text("two")]
                },
                Block::Item {
                    marker: "3.".into(),
                    content: vec![text("three")]
                },
                Block::Code {
                    language: "rust".into(),
                    text: "fn main() {}".into()
                },
                Block::Paragraph(vec![text("after")]),
            ]
        );
    }

    #[test]
    fn an_unfinished_fence_shows_what_has_streamed_so_far() {
        assert_eq!(
            parse("Here:\n```sh\ncargo te"),
            vec![
                Block::Paragraph(vec![text("Here:")]),
                Block::Code {
                    language: "sh".into(),
                    text: "cargo te".into()
                },
            ]
        );
        assert_eq!(
            parse("```\n"),
            vec![Block::Code {
                language: String::new(),
                text: String::new()
            }]
        );
    }

    #[test]
    fn inline_markers_pair_up_and_unmatched_ones_stay_literal() {
        assert_eq!(
            inline("use `cargo` and **care**"),
            vec![
                text("use "),
                Inline::Code("cargo".into()),
                text(" and "),
                Inline::Bold("care".into())
            ]
        );
        assert_eq!(inline("a ` b"), vec![text("a ` b")]);
        assert_eq!(inline("**open"), vec![text("**open")]);
        assert_eq!(inline("``"), vec![text("``")]);
        assert_eq!(
            inline("héllo `ü`"),
            vec![text("héllo "), Inline::Code("ü".into())]
        );
    }

    #[test]
    fn hash_without_a_space_and_long_numbers_are_not_markup() {
        assert_eq!(
            parse("#hashtag"),
            vec![Block::Paragraph(vec![text("#hashtag")])]
        );
        assert_eq!(
            parse("1234. no"),
            vec![Block::Paragraph(vec![text("1234. no")])]
        );
        assert!(parse("").is_empty());
    }
}
