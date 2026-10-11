//! Supply additional shaping fallbacks during platform startup, not UI rendering.
use eframe::egui::{FontData, FontDefinitions};

pub(crate) fn definitions(mut fonts: FontDefinitions) -> FontDefinitions {
    #[cfg(target_os = "macos")]
    let fallbacks = [
        ("editor-arabic", "/System/Library/Fonts/SFArabic.ttf"),
        (
            "editor-cjk",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ),
        (
            "editor-emoji",
            "/System/Library/Fonts/Apple Color Emoji.ttc",
        ),
    ];
    #[cfg(target_os = "windows")]
    let fallbacks = [
        ("editor-arabic", r"C:\Windows\Fonts\arial.ttf"),
        ("editor-cjk", r"C:\Windows\Fonts\msyh.ttc"),
        ("editor-emoji", r"C:\Windows\Fonts\seguiemj.ttf"),
    ];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let fallbacks: [(&str, &str); 0] = [];
    for (name, path) in fallbacks {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.into(), FontData::from_owned(bytes).into());
        }
    }
    fonts
}
