use crate::{Direction, EditError, EditorBuffer, Selection};
use unicode_segmentation::UnicodeSegmentation;

impl EditorBuffer {
    pub fn move_word(&mut self, direction: Direction, extend: bool) {
        let selections = self
            .selections()
            .selections()
            .iter()
            .map(|selection| {
                let head = word_neighbor(self.text(), selection.head, direction);
                Selection {
                    anchor: if extend { selection.anchor } else { head },
                    head,
                }
            })
            .collect();
        self.set_selections(selections, self.selections().primary_index())
            .unwrap();
    }

    pub fn delete_word(&mut self, direction: Direction) -> Result<bool, EditError> {
        let selections = self
            .selections()
            .selections()
            .iter()
            .map(|selection| {
                if selection.range().is_empty() {
                    Selection {
                        anchor: selection.anchor,
                        head: word_neighbor(self.text(), selection.head, direction),
                    }
                } else {
                    *selection
                }
            })
            .collect();
        self.replace_ranges(selections, self.selections().primary_index(), &[""])
    }
}

fn word_neighbor(text: &str, byte: usize, direction: Direction) -> usize {
    let graphemes: Vec<_> = text.grapheme_indices(true).collect();
    let class = |grapheme: &str| {
        let c = grapheme.chars().next().unwrap();
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    };
    let mut index = graphemes.partition_point(|(start, _)| *start < byte);
    match direction {
        Direction::Forward => {
            if let Some((_, grapheme)) = graphemes.get(index) {
                let current = class(grapheme);
                while index < graphemes.len() && class(graphemes[index].1) == current {
                    index += 1;
                }
                while index < graphemes.len() && class(graphemes[index].1) == 0 {
                    index += 1;
                }
            }
        }
        Direction::Backward => {
            while index > 0 && class(graphemes[index - 1].1) == 0 {
                index -= 1;
            }
            if index > 0 {
                let current = class(graphemes[index - 1].1);
                while index > 0 && class(graphemes[index - 1].1) == current {
                    index -= 1;
                }
            }
        }
    }
    graphemes.get(index).map_or(text.len(), |(start, _)| *start)
}
