//! Headless CPU frame probe. Does not measure native input or GPU presentation.
use egui::{Context, Id, RawInput, Rect, Vec2};
use lumen_editor_core::{EditorBuffer, Selection};
use lumen_editor_surface::EditorSurface;
use std::{hint::black_box, time::Instant};

fn main() {
    tab_switch_probe();
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
            let mut surface = EditorSurface::with_shaper(lumen_editor_surface::Shaper::default());
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

fn tab_switch_probe() {
    for rows in [10_000, 50_000] {
        let ctx = Context::default();
        let mut surface = EditorSurface::with_shaper(lumen_editor_surface::Shaper::default());
        let mut large = EditorBuffer::new("let value = office_ffi + 123; // é\n".repeat(rows));
        let mut small = EditorBuffer::new("small tab\n".into());
        let mut render = |buffer: &mut EditorBuffer, tab| {
            let start = Instant::now();
            black_box(ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(900.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        black_box(surface.show(ui, Id::new(tab), buffer));
                    });
                },
            ));
            start.elapsed().as_secs_f64() * 1000.0
        };
        let cold = render(&mut large, "large");
        let mut returns = Vec::new();
        for _ in 0..5 {
            render(&mut small, "small");
            returns.push(render(&mut large, "large"));
        }
        eprintln!(
            "tabs rows={rows} cold_ms={cold:.3} return_mean_ms={:.3}",
            returns.iter().sum::<f64>() / returns.len() as f64
        );
    }
}
