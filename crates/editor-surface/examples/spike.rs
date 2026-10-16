use eframe::egui;
use lumen_editor_core::{CaretAffinity, EditorBuffer, Selection, SelectionAffinity};
use lumen_editor_surface::EditorSurface;

// Reuse the workbench palette verbatim; unused workbench-only constants are
// expected in this isolated diagnostic example, not a second theme.
#[allow(dead_code)]
#[allow(unexpected_cfgs)]
#[path = "../../../src/ui/theme.rs"]
mod theme;

#[cfg(feature = "shaped")]
#[path = "../../../src/platform/editor_fonts.rs"]
mod editor_fonts;
#[path = "../../../src/platform/fonts.rs"]
mod fonts;

struct Spike {
    surface: EditorSurface,
    buffer: EditorBuffer,
    diagnostic_preedit: bool,
    preedit_started: bool,
}

impl eframe::App for Spike {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        // Explicit synthetic preview fixture for screenshots, not native IME validation.
        if self.diagnostic_preedit {
            ctx.input_mut(|input| input.focused = true);
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("spike_fixture")));
            if !self.preedit_started {
                ctx.input_mut(|input| {
                    input.events.extend([
                        egui::Event::Ime(egui::ImeEvent::Enabled),
                        egui::Event::Ime(egui::ImeEvent::Preedit("compose".into())),
                    ])
                });
                self.preedit_started = true;
            }
        }
        egui::TopBottomPanel::top("spike_help").exact_height(38.0).show(ctx, |ui| {
            ui.add(egui::Label::new("Multi-cursor prototype · Option/Alt-click adds a cursor").truncate())
                .on_hover_text("Cmd/Ctrl+D selects next occurrence; Shift+Cmd/Ctrl+L selects all. Option+Cmd+Up/Down (macOS), Ctrl+Alt+Up/Down (Windows), Shift+Alt+Up/Down (Linux) adds a cursor. Tab/Shift+Tab indent/outdent. Copy/Cut selected ranges; matching session paste distributes them. Cmd/Ctrl+Z undoes; Escape removes secondary cursors.");
            ui.add(egui::Label::new("Not production · clipboard, IME and accessibility gates pending").truncate())
                .on_hover_text("This isolated in-memory fixture cannot change production documents. Native clipboard transport, IME composition, accessibility and shaping are not approved.");
        });
        egui::TopBottomPanel::bottom("spike_status")
            .frame(egui::Frame::new().fill(theme::palette(&ctx.style().visuals).status))
            .exact_height(
                if self.surface.notice.is_some() || self.diagnostic_preedit {
                    38.0
                } else {
                    20.0
                },
            )
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} cursors · revision {} · {}",
                    self.buffer.selections().selections().len(),
                    self.buffer.revision(),
                    if self.buffer.is_dirty() {
                        "Modified"
                    } else {
                        "Unmodified"
                    }
                ));
                let notice = self
                    .surface
                    .notice
                    .as_deref()
                    .or(self.diagnostic_preedit.then_some(
                    "Synthetic IME preview at primary cursor; commit edits all captured cursors",
                ));
                if let Some(notice) = notice {
                    ui.add(egui::Label::new(notice).truncate())
                        .on_hover_text(notice);
                }
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.surface
                .show(ui, egui::Id::new("spike_fixture"), &mut self.buffer);
        });
    }
}

fn main() -> eframe::Result {
    let width = std::env::args()
        .nth(1)
        .and_then(|width| width.parse::<f32>().ok())
        .unwrap_or(900.0);
    eframe::run_native(
        "Lumen — multi-cursor backend spike",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([width, 550.0]),
            ..Default::default()
        },
        Box::new(|creation| {
            let _fonts = fonts::configure(&creation.egui_ctx);
            theme::configure_ui(&creation.egui_ctx);
            let bidi = std::env::args().any(|argument| argument == "--bidi");
            let text = if bidi {
                "abc אבג xyz\r\nabc العربية xyz\r\n// office ffi é 漢 👩‍💻\r\n\r\n".to_owned()
                    + &"// Mixed direction: hello שלום world · hello العربية world. ".repeat(6)
            } else {
                "let first = 1;\r\nlet second = 2;\r\n// Unicode: e\u{301} 🌍 👩‍💻\r\n\r\n".to_owned()
                    + &"// A long line demonstrating shared horizontal scrolling. ".repeat(6)
            };
            let mut buffer = EditorBuffer::new(text);
            if bidi {
                buffer
                    .set_selections_with_affinity(
                        vec![(
                            Selection { anchor: 2, head: 6 },
                            SelectionAffinity {
                                anchor: CaretAffinity::After,
                                head: CaretAffinity::Before,
                            },
                        )],
                        0,
                    )
                    .unwrap();
            } else {
                buffer
                    .set_selections(
                        vec![
                            Selection { anchor: 4, head: 9 },
                            Selection {
                                anchor: 20,
                                head: 26,
                            },
                        ],
                        0,
                    )
                    .unwrap();
            }
            #[cfg(not(feature = "shaped"))]
            let surface = EditorSurface::default();
            #[cfg(feature = "shaped")]
            let surface = EditorSurface::with_shaper(lumen_editor_surface::Shaper::new(
                &editor_fonts::definitions(_fonts),
            ));
            Ok(Box::new(Spike {
                surface,
                buffer,
                diagnostic_preedit: std::env::args().any(|argument| argument == "--ime-preview"),
                preedit_started: false,
            }))
        }),
    )
}
