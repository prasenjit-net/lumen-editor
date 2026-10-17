//! The lazy layout must agree with the original whole-document layout on every
//! observable: carets, bidirectional arrow stepping, hit testing, selection and
//! row geometry. Texts are generated deterministically from a seed.
use super::*;
use lumen_editor_core::CaretAffinity;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "fn ",
    "let ",
    "value",
    " = ",
    "42",
    ";",
    "  ",
    "\t",
    "(",
    ")",
    "{",
    "}",
    "->",
    "office",
    "é",
    "ñ",
    "e\u{301}",
    "漢字",
    "👩\u{200d}💻",
    "🎉",
    "אבג",
    "שלום",
    "العربية",
    "مرحبا",
    "abc אבג xyz",
    " ",
    "ffi",
    "\u{2028}",
];

fn generate(seed: u64, lines: usize) -> String {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut text = String::new();
    for line in 0..lines {
        for _ in 0..rng.below(7) {
            text.push_str(PIECES[rng.below(PIECES.len())]);
        }
        if line + 1 < lines || rng.below(2) == 0 {
            text.push('\n');
        }
    }
    text
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn same_rect(a: Rect, b: Rect) -> bool {
    near(a.min.x, b.min.x)
        && near(a.min.y, b.min.y)
        && near(a.max.x, b.max.x)
        && near(a.max.y, b.max.y)
}

fn typography() -> crate::Typography {
    crate::Typography::default()
}

fn boundaries(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(byte, _)| byte)
        .chain([text.len()])
        .collect()
}

fn check_carets_and_steps(text: &str, lazy: &ShapedLayout, oracle: &oracle::OracleLayout) {
    for byte in boundaries(text) {
        assert!(
            same_rect(lazy.caret(byte, Pos2::ZERO), oracle.caret(byte, Pos2::ZERO)),
            "caret at {byte} in {text:?}"
        );
        for affinity in [CaretAffinity::Before, CaretAffinity::After] {
            assert!(
                same_rect(
                    lazy.caret_with_affinity(byte, affinity, Pos2::ZERO),
                    oracle.caret_with_affinity(byte, affinity, Pos2::ZERO)
                ),
                "affinity caret at {byte} {affinity:?} in {text:?}"
            );
        }
    }
    for (byte, _) in text.grapheme_indices(true) {
        for affinity in [CaretAffinity::Before, CaretAffinity::After] {
            for right in [true, false] {
                assert_eq!(
                    lazy.visual_step(text, byte, affinity, right),
                    oracle.visual_step(text, byte, affinity, right),
                    "step from {byte} {affinity:?} right={right} in {text:?}"
                );
            }
        }
    }
    let end = text.len();
    for affinity in [CaretAffinity::Before, CaretAffinity::After] {
        for right in [true, false] {
            assert_eq!(
                lazy.visual_step(text, end, affinity, right),
                oracle.visual_step(text, end, affinity, right),
                "step from the end {affinity:?} right={right} in {text:?}"
            );
        }
    }
}

fn check_hits(text: &str, lazy: &ShapedLayout, oracle: &oracle::OracleLayout) {
    let height = typography().line_height;
    let rows = lazy.starts.len();
    let mut ys = vec![-10.0, -0.5];
    for row in 0..rows {
        ys.push(row as f32 * height + height / 2.0);
        ys.push(row as f32 * height + 0.25);
    }
    ys.push(rows as f32 * height + 3.0);
    ys.push(rows as f32 * height + 40.0);
    for y in ys {
        for x in [-8.0, 0.0, 3.0, 11.0, 29.0, 57.0, 120.0, 400.0] {
            assert_eq!(
                lazy.hit(text, vec2(x, y)),
                oracle.hit(text, vec2(x, y)),
                "hit ({x}, {y}) in {text:?}"
            );
        }
    }
}

fn check_selection(text: &str, lazy: &ShapedLayout, oracle: &oracle::OracleLayout, seed: u64) {
    let marks = boundaries(text);
    let mut rng = Rng(seed | 1);
    lazy.prepare(
        0.0,
        0.0,
        lazy.starts.len() as f32 * typography().line_height,
    );
    for _ in 0..12 {
        let (a, b) = (marks[rng.below(marks.len())], marks[rng.below(marks.len())]);
        let range = a.min(b)..a.max(b);
        let (got, want) = (
            lazy.selection(range.clone(), Pos2::ZERO),
            oracle.selection(range.clone(), Pos2::ZERO),
        );
        assert_eq!(got.len(), want.len(), "selection {range:?} in {text:?}");
        for (g, w) in got.iter().zip(&want) {
            assert!(same_rect(*g, *w), "selection {range:?}: {g:?} vs {w:?}");
        }
    }
}

#[test]
fn eager_layout_matches_the_whole_document_layout() {
    let shaper = Shaper::default();
    for seed in 1..=8 {
        let text = generate(seed, 6);
        let lazy = shaper.layout(&text, typography());
        let oracle = shaper.oracle_layout(&text, typography());
        check_carets_and_steps(&text, &lazy, &oracle);
        check_hits(&text, &lazy, &oracle);
        check_selection(&text, &lazy, &oracle, seed);
        let (a, b) = (lazy.rows(Pos2::ZERO), oracle.rows(Pos2::ZERO));
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert!(same_rect(*x, *y), "{x:?} vs {y:?} in {text:?}");
        }
        for (x, y) in lazy
            .character_carets(&text)
            .iter()
            .zip(oracle.character_carets(&text))
        {
            assert!(near(x.x, y.x) && near(x.y, y.y), "{x:?} vs {y:?}");
        }
    }
}

#[test]
fn lazily_shaped_layout_matches_the_whole_document_layout() {
    let shaper = Shaper::default();
    for seed in 20..=27 {
        let text = generate(seed, 7);
        let lazy = shaper.layout_lazy(&text, typography());
        let oracle = shaper.oracle_layout(&text, typography());
        assert_eq!(
            lazy.lines.lock().unwrap().len(),
            0,
            "nothing is shaped up front"
        );
        check_carets_and_steps(&text, &lazy, &oracle);
        check_hits(&text, &lazy, &oracle);
        check_selection(&text, &lazy, &oracle, seed);
        for (x, y) in lazy
            .character_carets(&text)
            .iter()
            .zip(oracle.character_carets(&text))
        {
            assert!(near(x.x, y.x) && near(x.y, y.y));
        }
    }
}

#[test]
fn opening_shapes_only_what_is_asked_for_not_the_document() {
    let shaper = Shaper::default();
    let text = "let value = 1; // some code\n".repeat(20_000);
    let layout = shaper.layout_lazy(&text, typography());
    assert_eq!(layout.lines.lock().unwrap().len(), 0);
    let height = typography().line_height;
    layout.prepare(0.0, 0.0, height * 40.0);
    let shaped = layout.lines.lock().unwrap().len();
    assert!(
        (40..=50).contains(&shaped),
        "{shaped} lines shaped for one screen"
    );
    // A caret far down shapes just its own line, and a click shapes just its row.
    layout.caret(text.len() / 2, Pos2::ZERO);
    layout.hit(&text, vec2(30.0, 15_000.0 * height + 3.0));
    assert!(layout.lines.lock().unwrap().len() < 60);
    // Row geometry exists for every line without shaping them.
    let rows = layout.rows(Pos2::ZERO);
    assert_eq!(rows.len(), 20_001);
    assert!(layout.lines.lock().unwrap().len() < 60);
}

#[test]
fn estimated_widths_match_shaped_widths_for_plain_monospace_lines() {
    let shaper = Shaper::default();
    let text = "plain ascii text\n\twith a tab\n\nshort\n";
    let lazy = shaper.layout_lazy(text, typography());
    let estimated: Vec<f32> = (0..lazy.starts.len())
        .map(|line| lazy.width_of(line))
        .collect();
    let exact = shaper.layout(text, typography());
    for (line, estimate) in estimated.iter().enumerate() {
        assert!(
            (estimate - exact.width_of(line)).abs() < 0.5,
            "line {line}: estimated {estimate}, shaped {}",
            exact.width_of(line)
        );
    }
}
