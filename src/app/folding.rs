//! Per-document fold view state. Document text/history remain in editor-core.
use lumen_editor_core::{FoldRange, discover_folds, syntax::Language};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct FoldTabState {
    revision: Option<u64>,
    language: Option<Language>,
    pub(crate) ranges: Vec<FoldRange>,
    collapsed: BTreeMap<usize, String>,
}

impl FoldTabState {
    pub(crate) fn refresh(&mut self, text: &str, language: Language, revision: u64) {
        if self.revision == Some(revision) && self.language == Some(language) {
            return;
        }
        let ranges = discover_folds(text, language);
        let mut claimed = BTreeSet::new();
        let mut rebased = BTreeMap::new();
        for (&old_line, signature) in &self.collapsed {
            if let Some(range) = ranges
                .iter()
                .filter(|range| {
                    !claimed.contains(&range.header)
                        && line_signature(text, range.header) == *signature
                })
                .min_by_key(|range| range.header.abs_diff(old_line))
            {
                claimed.insert(range.header);
                rebased.insert(range.header, signature.clone());
            }
        }
        self.collapsed = rebased;
        self.ranges = ranges;
        self.revision = Some(revision);
        self.language = Some(language);
    }

    pub(crate) fn collapsed(&self) -> BTreeSet<usize> {
        self.collapsed.keys().copied().collect()
    }

    pub(crate) fn set_collapsed(&mut self, headers: &BTreeSet<usize>, text: &str) {
        self.collapsed = headers
            .iter()
            .filter(|header| self.ranges.iter().any(|range| range.header == **header))
            .map(|header| (*header, line_signature(text, *header)))
            .collect();
    }
}

fn line_signature(text: &str, line: usize) -> String {
    text.lines().nth(line).unwrap_or_default().trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_rebase_after_lines_are_inserted_above_and_drop_changed_headers() {
        let text = "fn a() {\n  work();\n}\n";
        let mut state = FoldTabState::default();
        state.refresh(text, Language::Rust, 0);
        state.set_collapsed(&BTreeSet::from([0]), text);
        assert_eq!(state.collapsed(), BTreeSet::from([0]));
        state.refresh(&format!("// heading\n{text}"), Language::Rust, 1);
        assert_eq!(state.collapsed(), BTreeSet::from([1]));
        state.refresh("fn renamed() {\n  work();\n}\n", Language::Rust, 2);
        assert!(state.collapsed().is_empty());
    }
}
