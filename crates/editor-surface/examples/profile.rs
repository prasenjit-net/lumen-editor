//! Headless CPU frame probe. Does not measure native input or GPU presentation.
use egui::{Context, Id, RawInput, Rect, Vec2};
use lumen_editor_core::{EditorBuffer, Selection};
use lumen_editor_surface::EditorSurface;
use std::{hint::black_box, time::Instant};

fn main() {
    println!("fixture,bytes,cursors,accessibility,cold_ms,idle_mean_ms");
    for (fixture, text, cursors) in [
        (
            "100-rows",
            "let value = office_ffi + 123; // é\n".repeat(100),
            1,
        ),
        (
            "1000-rows",
            "let value = office_ffi + 123; // é\n".repeat(1_000),
            1,
        ),
        (
            "10000-rows",
            "let value = office_ffi + 123; // é\n".repeat(10_000),
            1,
        ),
        (
            "1000-cursors",
            "let value = office_ffi + 123; // é\n".repeat(1_000),
            1_000,
        ),
        ("long-line", "office é 漢👩‍💻 ".repeat(500), 1),
    ] {
        for accessibility in [false, true] {
            let ctx = Context::default();
            if accessibility {
                ctx.enable_accesskit();
            }
            let mut buffer = EditorBuffer::new(text.clone());
            if cursors > 1 {
                let selections = std::iter::once(0)
                    .chain(text.match_indices('\n').map(|(byte, _)| byte + 1))
                    .take(cursors)
                    .map(Selection::caret)
                    .collect();
                buffer.set_selections(selections, 0).unwrap();
            }
            #[cfg(feature = "shaped")]
            let mut surface = EditorSurface::with_shaper(lumen_editor_surface::Shaper::default());
            #[cfg(not(feature = "shaped"))]
            let mut surface = EditorSurface::default();
            let mut timings = Vec::new();
            for frame in 0..4 {
                let start = Instant::now();
                black_box(ctx.run(
                    RawInput {
                        time: Some(frame as f64 * 0.05),
                        screen_rect: Some(Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(900.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            black_box(surface.show(ui, Id::new("profile"), &mut buffer));
                        });
                    },
                ));
                timings.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            println!(
                "{fixture},{},{cursors},{accessibility},{:.3},{:.3}",
                text.len(),
                timings[0],
                timings[1..].iter().sum::<f64>() / 3.0
            );
        }
    }
}
