use crate::{app::LumenApp, platform, ui::theme::configure_ui};
use eframe::egui;
pub(crate) fn run() -> eframe::Result<()> {
    use crate::cli::{Invocation, USAGE, parse};
    let cwd = std::env::current_dir().unwrap_or_default();
    let targets = match parse(std::env::args_os().skip(1), &cwd, |path| path.exists()) {
        Invocation::Help => {
            println!("Lumen {}\n\n{USAGE}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Invocation::Version => {
            println!("Lumen {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Invocation::Invalid(message) => {
            eprintln!("lumen: {message}\n\n{USAGE}");
            std::process::exit(2);
        }
        Invocation::Open(targets) => targets,
    };
    install_panic_log();
    crate::services::logger::info(
        "editor",
        &format!("Lumen {} started", env!("CARGO_PKG_VERSION")),
    );
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
            let repaint = cc.egui_ctx.clone();
            crate::services::logger::set_waker(std::sync::Arc::new(move || {
                repaint.request_repaint()
            }));
            let mut app = LumenApp::default();
            #[cfg(feature = "shared-editor")]
            {
                app.view.shared_surface = Some(lumen_editor_surface::EditorSurface::with_shaper(
                    lumen_editor_surface::Shaper::new(&platform::editor_fonts::definitions(_fonts)),
                ));
            }
            app.initialize_recent_files(crate::services::settings::user_path());
            let opened_something = !targets.is_empty();
            app.open_command_line_targets(targets);
            app.initialize_settings(crate::services::settings::user_path());
            app.task_user_path = crate::services::tasks::user_path();
            app.task_login_path = true;
            if let Some(path) = crate::services::task_trust::FileTrust::user_path() {
                app.task_trust = Box::new(crate::services::task_trust::FileTrust::new(path));
            }
            if !opened_something {
                app.reopen_last_folder_if_enabled();
            }
            app.restore_recovery();
            if std::env::var_os("LUMEN_NATIVE_AGENTS").is_some_and(|v| v != "0") {
                app.enable_native_agents(
                    &cc.egui_ctx,
                    crate::services::agent_sessions_store::user_path(),
                );
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                let menu = platform::native_menu::NativeMenu::new(&cc.egui_ctx)?;
                #[cfg(target_os = "windows")]
                menu.attach(cc)?;
                menu.set_context(
                    app.editor.active_index().is_some(),
                    app.editor.active_index().is_some(),
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

/// Record panics in the Output log before the default handler reports them.
fn install_panic_log() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crate::services::logger::error("panic", &info.to_string());
        previous(info);
    }));
}
