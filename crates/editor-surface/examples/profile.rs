//! Headless CPU frame probe. Does not measure native input or GPU presentation.
use egui::{Context, Id, RawInput, Rect, Vec2};
use lumen_editor_core::EditorBuffer;
use lumen_editor_surface::EditorSurface;
use std::{hint::black_box, time::Instant};

fn main() {
    println!("rows,accessibility,cold_ms,idle_mean_ms");
    for rows in [100, 1_000] {
        for accessibility in [false, true] {
            let ctx = Context::default();
            if accessibility {
                ctx.enable_accesskit();
            }
            let mut buffer = EditorBuffer::new("let value = office_ffi + 123; // é\n".repeat(rows));
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
                "{rows},{accessibility},{:.3},{:.3}",
                timings[0],
                timings[1..].iter().sum::<f64>() / 3.0
            );
        }
    }
}
