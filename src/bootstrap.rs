use crate::{app::LumenApp, platform, ui::theme::configure_ui};
use eframe::egui;
use std::path::PathBuf;
pub(crate) fn run() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 760.0])
        .with_min_inner_size([900.0, 560.0])
        .with_icon(
            eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/lumen.png"))
                .expect("bundled Lumen application icon must be a valid PNG"),
        );
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(true);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Lumen",
        options,
        Box::new(|cc| {
            let _fonts = platform::fonts::configure(&cc.egui_ctx);
            configure_ui(&cc.egui_ctx);
            let mut app = LumenApp::default();
            #[cfg(feature = "shared-editor")]
            {
                app.view.shared_surface = Some(lumen_editor_surface::EditorSurface::with_shaper(
                    lumen_editor_surface::Shaper::new(&platform::editor_fonts::definitions(_fonts)),
                ));
            }
            app.initialize_recent_files(crate::services::settings::user_path());
            for argument in std::env::args_os().skip(1) {
                let path = PathBuf::from(argument);
                let path = std::path::absolute(&path).unwrap_or(path);
                if path.is_dir() {
                    app.editor.workspace = Some(path.clone());
                    app.view.expanded_dirs.insert(path);
                } else {
                    app.open_file(path);
                }
            }
            app.initialize_settings(crate::services::settings::user_path());
            app.restore_recovery();
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                let menu = platform::native_menu::NativeMenu::new(&cc.egui_ctx)?;
                #[cfg(target_os = "windows")]
                menu.attach(cc)?;
                menu.set_context(
                    app.editor.active_file.is_some(),
                    app.editor.active_file.is_some(),
                    false,
                    false,
                    false,
                    app.editor.workspace.is_some(),
                );
                app.native_menu = Some(menu);
            }
            Ok(Box::new(app))
        }),
    )
}
