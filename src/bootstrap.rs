use crate::{app::LumenApp, platform, ui::theme::configure_ui};
use eframe::egui;
use std::path::PathBuf;
pub(crate) fn run() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 760.0])
        .with_min_inner_size([900.0, 560.0]);
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
            platform::fonts::configure(&cc.egui_ctx);
            configure_ui(&cc.egui_ctx);
            let mut app = LumenApp::default();
            for argument in std::env::args_os().skip(1) {
                let path = PathBuf::from(argument);
                if path.is_dir() {
                    app.editor.workspace = Some(path.clone());
                    app.view.expanded_dirs.insert(path);
                } else {
                    app.open_file(path);
                }
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                let menu = platform::native_menu::NativeMenu::new(&cc.egui_ctx)?;
                #[cfg(target_os = "windows")]
                menu.attach(cc)?;
                menu.set_document_open(app.editor.active_file.is_some());
                app.native_menu = Some(menu);
            }
            Ok(Box::new(app))
        }),
    )
}
