use eframe::egui;
use std::fs;
pub(crate) fn configure(ctx: &egui::Context) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "lumen-icons".into(),
        egui::FontData::from_static(include_bytes!("../../assets/lumen-icons.ttf")).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push("lumen-icons".into());
    // Use platform fonts when available, retaining embedded fallbacks.
    #[cfg(not(target_os = "windows"))]
    let platform_fonts = [
        (
            "system-ui",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            egui::FontFamily::Proportional,
        ),
        (
            "editor-mono",
            "/System/Library/Fonts/Menlo.ttc",
            egui::FontFamily::Monospace,
        ),
    ];
    #[cfg(target_os = "windows")]
    let platform_fonts = [
        (
            "system-ui",
            r"C:\Windows\Fonts\segoeui.ttf",
            egui::FontFamily::Proportional,
        ),
        (
            "editor-mono",
            r"C:\Windows\Fonts\consola.ttf",
            egui::FontFamily::Monospace,
        ),
    ];
    for (name, path, family) in platform_fonts {
        if let Ok(bytes) = fs::read(path) {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.into());
        }
    }
    let hack = egui::FontDefinitions::default().families[&egui::FontFamily::Monospace].clone();
    fonts
        .families
        .insert(egui::FontFamily::Name("lumen-hack".into()), hack);
    ctx.set_fonts(fonts.clone());
    fonts
}
