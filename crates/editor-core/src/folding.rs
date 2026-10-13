//! Read-only fold discovery. Collapsing rows requires a shared source/display
//! projection and is deliberately not performed here.
use crate::syntax::{self, Kind, Language};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FoldRange {
    /// Zero-based header line, kept visible when this range is collapsed.
    pub header: usize,
    /// Zero-based final included line.
    pub end: usize,
}

pub fn discover_folds(text: &str, language: Language) -> Vec<FoldRange> {
    let mut ranges = match language {
        Language::Python | Language::Yaml => indentation_folds(text),
        Language::Plain => Vec::new(),
        _ => bracket_folds(text, language),
    };
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

fn bracket_folds(text: &str, language: Language) -> Vec<FoldRange> {
    let spans = syntax::highlight(text, language);
    let mut span_index = 0;
    let mut line = 0;
    let mut stack = Vec::new();
    let mut ranges = Vec::new();
    for (byte, ch) in text.char_indices() {
        while spans
            .get(span_index)
            .is_some_and(|span| span.range.end <= byte)
        {
            span_index += 1;
        }
        let ignored = spans.get(span_index).is_some_and(|span| {
            span.range.contains(&byte) && matches!(span.kind, Kind::String | Kind::Comment)
        });
        if !ignored {
            match ch {
                '{' => stack.push(('{', line)),
                '[' => stack.push(('[', line)),
                '}' | ']' => {
                    let open = if ch == '}' { '{' } else { '[' };
                    if stack.last().is_some_and(|(kind, _)| *kind == open)
                        && let Some((_, header)) = stack.pop()
                        && line > header
                    {
                        ranges.push(FoldRange { header, end: line });
                    }
                }
                _ => {}
            }
        }
        if ch == '\n' {
            line += 1;
        }
    }
    ranges
}

fn indentation_folds(text: &str) -> Vec<FoldRange> {
    let mut ranges = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut previous = None;
    for (line, content) in text.lines().enumerate() {
        if content.trim().is_empty() {
            continue;
        }
        let width = content
            .chars()
            .take_while(|ch| matches!(ch, ' ' | '\t'))
            .map(|ch| if ch == '\t' { 4 } else { 1 })
            .sum::<usize>();
        if let Some((previous_line, previous_width)) = previous {
            while stack
                .last()
                .is_some_and(|(_, header_width)| width <= *header_width)
            {
                let (header, _) = stack.pop().unwrap();
                if previous_line > header {
                    ranges.push(FoldRange {
                        header,
                        end: previous_line,
                    });
                }
            }
            if width > previous_width {
                stack.push((previous_line, previous_width));
            }
        }
        previous = Some((line, width));
    }
    if let Some((last_line, _)) = previous {
        for (header, _) in stack {
            if last_line > header {
                ranges.push(FoldRange {
                    header,
                    end: last_line,
                });
            }
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braces_ignore_comment_and_string_tokens_and_keep_nested_lines() {
        let text = "fn main() {\r\n  let x = \"{ }\"; // {\r\n  if x {\r\n    run();\r\n  }\r\n}";
        assert_eq!(
            discover_folds(text, Language::Rust),
            [
                FoldRange { header: 0, end: 5 },
                FoldRange { header: 2, end: 4 }
            ]
        );
    }

    #[test]
    fn indentation_folds_skip_blank_lines_and_close_at_dedent() {
        let text = "def one():\n  if x:\n    pass\n\n  done()\nnext()\n";
        assert_eq!(
            discover_folds(text, Language::Python),
            [
                FoldRange { header: 0, end: 4 },
                FoldRange { header: 1, end: 2 }
            ]
        );
    }

    #[test]
    fn plain_text_has_no_implicit_folds() {
        assert!(discover_folds("a {\n b\n}", Language::Plain).is_empty());
    }
}
