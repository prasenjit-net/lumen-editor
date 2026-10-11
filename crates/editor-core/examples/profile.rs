//! Reproducible core-only timing and retained history payload; no UI or I/O.
use lumen_editor_core::EditorBuffer;
use std::{hint::black_box, time::Instant};

fn main() {
    println!("bytes,mode,edits,edit_ms,undo_ms,history_text_bytes");
    for bytes in [10_000, 100_000, 1_000_000] {
        for grouped in [false, true] {
            let mut buffer = EditorBuffer::new("a".repeat(bytes));
            let start = Instant::now();
            for index in 0..30 {
                if grouped {
                    buffer.type_text("x", index as f64 * 0.05).unwrap();
                } else {
                    buffer.replace_selections(&["x"]).unwrap();
                }
                black_box(buffer.revision());
            }
            let edit_ms = start.elapsed().as_secs_f64() * 1000.0;
            let history = buffer.history_text_bytes();
            let start = Instant::now();
            while buffer.undo() {
                black_box(buffer.text());
            }
            let undo_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(buffer.text().len(), bytes);
            println!(
                "{bytes},{},{},{edit_ms:.3},{undo_ms:.3},{history}",
                if grouped { "typing" } else { "atomic" },
                30
            );
        }
    }
}
