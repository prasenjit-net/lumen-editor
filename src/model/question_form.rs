//! The answers being put together for an agent's questions, before they are
//! sent. One question shows at a time; each needs a chosen option or typed
//! words before the form can move on or be sent.
use lumen_agent_core::{
    Question,
    api::{QuestionAnswer, QuestionReply},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct QuestionForm {
    step: usize,
    entries: Vec<Entry>,
    /// The option the keyboard is on, in the current question.
    cursor: usize,
    /// The reply went out; the form only waits for the request to resolve.
    sent: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Entry {
    chosen: Vec<String>,
    typed: String,
}

impl QuestionForm {
    pub(crate) fn new(questions: &[Question]) -> Self {
        Self {
            entries: vec![Entry::default(); questions.len()],
            ..Self::default()
        }
    }

    /// The question being shown.
    pub(crate) fn step(&self) -> usize {
        self.step
    }

    pub(crate) fn is_last(&self) -> bool {
        self.step + 1 >= self.entries.len()
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    /// Put the keyboard on option `option` of the current question without
    /// choosing it (focus reached it some other way, such as Tab).
    pub(crate) fn point_at(&mut self, option: usize) {
        self.cursor = option;
    }

    pub(crate) fn is_chosen(&self, question: usize, label: &str) -> bool {
        self.entries
            .get(question)
            .is_some_and(|entry| entry.chosen.iter().any(|chosen| chosen == label))
    }

    /// Choose option `option` of the current question. A single choice
    /// replaces the earlier one and any typed words; a multiple choice toggles.
    pub(crate) fn choose(&mut self, questions: &[Question], option: usize) {
        let Some(question) = questions.get(self.step) else {
            return;
        };
        let Some(label) = question.options.get(option).map(|o| o.label.clone()) else {
            return;
        };
        self.cursor = option;
        let entry = &mut self.entries[self.step];
        if question.multiple {
            if let Some(at) = entry.chosen.iter().position(|chosen| *chosen == label) {
                entry.chosen.remove(at);
            } else {
                entry.chosen.push(label);
            }
        } else {
            entry.chosen = vec![label];
            entry.typed.clear();
        }
    }

    /// Move the keyboard by `delta` options. For a single choice that also
    /// chooses, the way a radio group behaves.
    pub(crate) fn move_cursor(&mut self, questions: &[Question], delta: isize) {
        let Some(question) = questions.get(self.step) else {
            return;
        };
        let count = question.options.len();
        if count == 0 {
            return;
        }
        let next = self.cursor.saturating_add_signed(delta).min(count - 1);
        if question.multiple {
            self.cursor = next;
        } else {
            self.choose(questions, next);
        }
    }

    /// The typed answer of the current question, for editing in place.
    pub(crate) fn typed_mut(&mut self) -> &mut String {
        &mut self.entries[self.step].typed
    }

    /// After the typed answer changed: words typed for a single choice stand
    /// in for the options, so a chosen one is let go.
    pub(crate) fn typed_changed(&mut self, questions: &[Question]) {
        let single = questions.get(self.step).is_some_and(|q| !q.multiple);
        let entry = &mut self.entries[self.step];
        if single && !entry.typed.trim().is_empty() {
            entry.chosen.clear();
        }
    }

    fn is_answered(&self, question: usize) -> bool {
        self.entries
            .get(question)
            .is_some_and(|entry| !entry.chosen.is_empty() || !entry.typed.trim().is_empty())
    }

    pub(crate) fn can_advance(&self) -> bool {
        !self.sent && self.is_answered(self.step)
    }

    pub(crate) fn can_submit(&self) -> bool {
        !self.sent && (0..self.entries.len()).all(|question| self.is_answered(question))
    }

    pub(crate) fn next(&mut self, questions: &[Question]) {
        if self.can_advance() && !self.is_last() {
            self.go(questions, self.step + 1);
        }
    }

    pub(crate) fn back(&mut self, questions: &[Question]) {
        if self.step > 0 {
            self.go(questions, self.step - 1);
        }
    }

    /// Show question `step`, with the keyboard on its chosen option.
    fn go(&mut self, questions: &[Question], step: usize) {
        self.step = step.min(self.entries.len().saturating_sub(1));
        self.cursor = questions
            .get(self.step)
            .and_then(|question| {
                let entry = &self.entries[self.step];
                question
                    .options
                    .iter()
                    .position(|option| entry.chosen.contains(&option.label))
            })
            .unwrap_or(0);
    }

    pub(crate) fn is_sent(&self) -> bool {
        self.sent
    }

    /// The answers, once every question has one; the form is then sent.
    pub(crate) fn submit(&mut self) -> Option<QuestionReply> {
        if !self.can_submit() {
            return None;
        }
        self.sent = true;
        Some(QuestionReply::Answered(
            self.entries
                .iter()
                .map(|entry| QuestionAnswer {
                    chosen: entry.chosen.clone(),
                    typed: Some(entry.typed.trim().to_string()).filter(|typed| !typed.is_empty()),
                })
                .collect(),
        ))
    }

    /// Skip every question; the form is then sent.
    pub(crate) fn skip(&mut self) -> Option<QuestionReply> {
        if self.sent {
            return None;
        }
        self.sent = true;
        Some(QuestionReply::Skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{QuestionOption, TypedAnswer};

    fn question(multiple: bool, labels: &[&str]) -> Question {
        Question {
            id: "q".into(),
            header: String::new(),
            prompt: "?".into(),
            options: labels
                .iter()
                .map(|label| QuestionOption {
                    label: (*label).into(),
                    description: String::new(),
                })
                .collect(),
            multiple,
            typed: TypedAnswer::Text,
        }
    }

    fn answers(reply: QuestionReply) -> Vec<(Vec<String>, Option<String>)> {
        match reply {
            QuestionReply::Answered(answers) => answers
                .into_iter()
                .map(|answer| (answer.chosen, answer.typed))
                .collect(),
            QuestionReply::Skipped => panic!("skipped"),
        }
    }

    #[test]
    fn a_single_choice_replaces_and_typed_words_stand_in_for_it() {
        let questions = [question(false, &["a", "b"])];
        let mut form = QuestionForm::new(&questions);
        assert!(!form.can_submit(), "nothing answered yet");
        form.choose(&questions, 0);
        form.choose(&questions, 1);
        assert!(form.is_chosen(0, "b") && !form.is_chosen(0, "a"));
        form.typed_mut().push_str("  my own  ");
        form.typed_changed(&questions);
        assert!(!form.is_chosen(0, "b"), "typing lets the choice go");
        form.choose(&questions, 0);
        assert_eq!(
            form.clone().submit().map(answers),
            Some(vec![(vec!["a".to_string()], None)]),
            "choosing clears the typed words"
        );
        form.typed_mut().push_str("mine");
        form.typed_changed(&questions);
        assert_eq!(
            form.submit().map(answers),
            Some(vec![(Vec::new(), Some("mine".to_string()))])
        );
        assert!(
            form.is_sent() && !form.can_submit() && form.skip().is_none(),
            "sent once"
        );
    }

    #[test]
    fn a_multiple_choice_toggles_and_keeps_typed_words_too() {
        let questions = [question(true, &["x", "y", "z"])];
        let mut form = QuestionForm::new(&questions);
        form.choose(&questions, 0);
        form.choose(&questions, 2);
        form.choose(&questions, 0);
        form.typed_mut().push('w');
        form.typed_changed(&questions);
        assert_eq!(
            form.submit().map(answers),
            Some(vec![(vec!["z".to_string()], Some("w".to_string()))])
        );
    }

    #[test]
    fn each_question_needs_an_answer_before_moving_on_and_sending() {
        let questions = [question(false, &["a", "b"]), question(true, &["x", "y"])];
        let mut form = QuestionForm::new(&questions);
        form.next(&questions);
        assert_eq!(form.step(), 0, "the first question is not answered");
        form.choose(&questions, 1);
        assert!(form.can_advance() && !form.can_submit());
        form.next(&questions);
        assert!(form.is_last() && form.step() == 1);
        assert!(!form.can_advance(), "the second is not answered yet");
        form.back(&questions);
        assert_eq!(
            (form.step(), form.cursor()),
            (0, 1),
            "back on the chosen option"
        );
        form.next(&questions);
        form.choose(&questions, 0);
        assert_eq!(
            form.submit().map(answers),
            Some(vec![
                (vec!["b".to_string()], None),
                (vec!["x".to_string()], None)
            ])
        );
    }

    #[test]
    fn the_keyboard_chooses_a_single_answer_and_moves_over_multiple_ones() {
        let questions = [
            question(false, &["a", "b", "c"]),
            question(true, &["x", "y"]),
        ];
        let mut form = QuestionForm::new(&questions);
        form.move_cursor(&questions, 1);
        assert!(form.is_chosen(0, "b"), "a radio group chooses as it moves");
        form.move_cursor(&questions, 5);
        assert!(
            form.is_chosen(0, "c") && form.cursor() == 2,
            "stops at the end"
        );
        form.move_cursor(&questions, -9);
        assert_eq!(form.cursor(), 0);
        form.next(&questions);
        form.move_cursor(&questions, 1);
        assert!(
            !form.is_chosen(1, "y"),
            "moving over check boxes leaves them"
        );
        assert_eq!(form.cursor(), 1);
    }

    #[test]
    fn skipping_sends_once() {
        let questions = [question(false, &["a"])];
        let mut form = QuestionForm::new(&questions);
        assert_eq!(form.skip(), Some(QuestionReply::Skipped));
        assert!(form.skip().is_none() && form.submit().is_none());
    }
}
