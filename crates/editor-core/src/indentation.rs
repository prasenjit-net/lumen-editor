//! UI-independent indentation rules for editor key commands.
use crate::SelectionSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indentation {
    pub width: usize,
    pub insert_spaces: bool,
    pub detect: bool,
    pub auto_indent: bool,
}

impl Default for Indentation {
    fn default() -> Self {
        Self {
            width: 4,
            insert_spaces: true,
            detect: true,
            auto_indent: true,
        }
    }
}

impl Indentation {
    pub fn effective(self, text: &str) -> Self {
        if !self.detect {
            return self;
        }
        let mut tabs = 0;
        let mut spaces = 0;
        let mut smallest_spaces = usize::MAX;
        for line in text.lines().take(200) {
            let prefix: String = line
                .chars()
                .take_while(|character| matches!(character, ' ' | '\t'))
                .collect();
            if prefix.is_empty() || prefix.len() == line.len() {
                continue;
            }
            if prefix.starts_with('\t') {
                tabs += 1;
            } else if prefix.chars().all(|character| character == ' ') {
                spaces += 1;
                if (2..=8).contains(&prefix.len()) {
                    smallest_spaces = smallest_spaces.min(prefix.len());
                }
            }
        }
        Self {
            insert_spaces: if tabs > spaces {
                false
            } else if spaces > 0 {
                true
            } else {
                self.insert_spaces
            },
            width: if smallest_spaces < usize::MAX {
                smallest_spaces
            } else {
                self.width
            },
            ..self
        }
    }

    pub fn unit(self) -> String {
        if self.insert_spaces {
            " ".repeat(self.width.clamp(1, 8))
        } else {
            "\t".into()
        }
    }

    pub fn newline_fragments(self, text: &str, selections: &SelectionSet) -> Vec<String> {
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let unit = self.unit();
        selections
            .selections()
            .iter()
            .map(|selection| {
                if !self.auto_indent {
                    return newline.to_owned();
                }
                let start = selection.range().start;
                let line_start = text[..start].rfind('\n').map_or(0, |index| index + 1);
                let before_caret = &text[line_start..start];
                let leading = before_caret
                    .bytes()
                    .take_while(|byte| matches!(byte, b' ' | b'\t'))
                    .count();
                let mut fragment = format!("{newline}{}", &before_caret[..leading]);
                if matches!(
                    before_caret.trim_end().chars().last(),
                    Some('{' | '[' | '(')
                ) {
                    fragment.push_str(&unit);
                }
                fragment
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Selection;

    #[test]
    fn detects_tabs_and_space_width_but_respects_disabled_detection() {
        let settings = Indentation::default();
        assert_eq!(settings.effective("\talpha\n\tbeta").unit(), "\t");
        assert_eq!(settings.effective("  alpha\n    beta").unit(), "  ");
        assert_eq!(
            Indentation {
                detect: false,
                ..settings
            }
            .effective("\talpha")
            .unit(),
            "    "
        );
    }

    #[test]
    fn enter_preserves_crlf_and_indents_each_caret_from_its_own_line() {
        let text = "  first {\r\n\tsecond";
        let selections = SelectionSet::new(
            text,
            vec![Selection::caret(9), Selection::caret(text.len())],
            0,
        )
        .unwrap();
        assert_eq!(
            Indentation::default().newline_fragments(text, &selections),
            ["\r\n      ", "\r\n\t"]
        );
        assert_eq!(
            Indentation {
                auto_indent: false,
                ..Default::default()
            }
            .newline_fragments(text, &selections),
            ["\r\n", "\r\n"]
        );
    }
}
