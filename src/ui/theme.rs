use eframe::egui::{self, Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle, Vec2};
/// Resolved colors are passed through the UI's visuals, including scoped editor overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Theme {
    Dark,
    Light,
    HighContrast,
}

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) bg: Color32,
    pub(crate) panel: Color32,
    pub(crate) sidebar: Color32,
    pub(crate) tab: Color32,
    #[cfg(not(target_os = "windows"))]
    pub(crate) title: Color32,
    pub(crate) status: Color32,
    pub(crate) active: Color32,
    pub(crate) text: Color32,
    pub(crate) muted: Color32,
    pub(crate) border: Color32,
    /// Edge of surfaces that float above the workbench (popups, dialogs, menus).
    pub(crate) float_border: Color32,
    pub(crate) accent: Color32,
    pub(crate) hover: Color32,
    pub(crate) input: Color32,
    pub(crate) diff_removed_bg: Color32,
    pub(crate) diff_added_bg: Color32,
    pub(crate) diff_removed_text: Color32,
    pub(crate) diff_added_text: Color32,
    pub(crate) diff_removed_highlight: Color32,
    pub(crate) diff_added_highlight: Color32,
    pub(crate) canvas: Color32,
    pub(crate) error: Color32,
    pub(crate) warning: Color32,
    pub(crate) success: Color32,
    pub(crate) info: Color32,
    /// Subtle per-provider identity colours for session rows and chat tabs.
    pub(crate) codex: Color32,
    pub(crate) claude: Color32,
}
impl Theme {
    pub(crate) fn resolve(choice: &str, system: Option<egui::Theme>) -> Self {
        match choice {
            "light" | "lumen-light" => Self::Light,
            "high-contrast" | "lumen-high-contrast" => Self::HighContrast,
            "system" if system == Some(egui::Theme::Light) => Self::Light,
            _ => Self::Dark,
        }
    }
    #[cfg(feature = "shared-editor")]
    pub(crate) fn scheme(self) -> &'static str {
        match self {
            Self::Dark => "lumen-dark",
            Self::Light => "lumen-light",
            Self::HighContrast => "lumen-high-contrast",
        }
    }
    pub(crate) fn palette(self) -> Palette {
        let dark = Palette {
            bg: Color32::from_rgb(30, 30, 30),
            panel: Color32::from_rgb(37, 37, 38),
            sidebar: Color32::from_rgb(51, 51, 51),
            tab: Color32::from_rgb(45, 45, 45),
            #[cfg(not(target_os = "windows"))]
            title: Color32::from_rgb(60, 60, 60),
            status: Color32::from_rgb(0, 122, 204),
            active: Color32::from_rgb(4, 57, 94),
            text: Color32::from_rgb(204, 204, 204),
            muted: Color32::from_rgb(157, 157, 157),
            border: Color32::from_rgb(43, 43, 43),
            float_border: Color32::from_rgb(69, 69, 69),
            accent: Color32::from_rgb(0, 122, 204),
            hover: Color32::from_rgb(43, 43, 43),
            input: Color32::from_rgb(49, 49, 49),
            diff_removed_bg: Color32::from_rgb(64, 37, 37),
            diff_added_bg: Color32::from_rgb(34, 67, 44),
            diff_removed_text: Color32::from_rgb(244, 135, 113),
            diff_added_text: Color32::from_rgb(137, 209, 133),
            diff_removed_highlight: Color32::from_rgb(105, 45, 45),
            diff_added_highlight: Color32::from_rgb(46, 91, 57),
            canvas: Color32::from_gray(24),
            error: Color32::from_rgb(244, 135, 113),
            warning: Color32::from_rgb(204, 167, 0),
            success: Color32::from_rgb(35, 209, 139),
            info: Color32::from_rgb(117, 190, 255),
            codex: Color32::from_rgb(78, 190, 160),
            claude: Color32::from_rgb(217, 140, 98),
        };
        match self {
            Self::Dark => dark,
            Self::Light => Palette {
                bg: Color32::WHITE,
                panel: Color32::from_gray(243),
                sidebar: Color32::from_gray(230),
                tab: Color32::from_gray(236),
                #[cfg(not(target_os = "windows"))]
                title: Color32::from_gray(221),
                canvas: Color32::from_gray(218),
                text: Color32::from_gray(40),
                muted: Color32::from_gray(96),
                input: Color32::WHITE,
                border: Color32::from_gray(190),
                float_border: Color32::from_gray(160),
                hover: Color32::from_gray(224),
                active: Color32::from_rgb(190, 220, 245),
                accent: Color32::from_rgb(0, 95, 160),
                error: Color32::from_rgb(165, 25, 25),
                warning: Color32::from_rgb(125, 88, 0),
                success: Color32::from_rgb(0, 112, 64),
                info: Color32::from_rgb(0, 95, 160),
                codex: Color32::from_rgb(0, 120, 100),
                claude: Color32::from_rgb(168, 84, 40),
                diff_removed_bg: Color32::from_rgb(255, 225, 225),
                diff_added_bg: Color32::from_rgb(219, 242, 219),
                diff_removed_text: Color32::from_rgb(165, 25, 25),
                diff_added_text: Color32::from_rgb(20, 105, 35),
                diff_removed_highlight: Color32::from_rgb(245, 180, 180),
                diff_added_highlight: Color32::from_rgb(170, 220, 170),
                ..dark
            },
            Self::HighContrast => Palette {
                bg: Color32::BLACK,
                panel: Color32::BLACK,
                sidebar: Color32::BLACK,
                tab: Color32::BLACK,
                #[cfg(not(target_os = "windows"))]
                title: Color32::BLACK,
                canvas: Color32::WHITE,
                text: Color32::WHITE,
                muted: Color32::from_gray(220),
                input: Color32::BLACK,
                border: Color32::WHITE,
                float_border: Color32::WHITE,
                hover: Color32::from_gray(55),
                active: Color32::from_rgb(0, 65, 100),
                accent: Color32::YELLOW,
                error: Color32::from_rgb(255, 120, 120),
                warning: Color32::YELLOW,
                success: Color32::GREEN,
                info: Color32::from_rgb(0, 255, 255),
                codex: Color32::from_rgb(0, 255, 190),
                claude: Color32::from_rgb(255, 170, 90),
                ..dark
            },
        }
    }
}

pub(crate) fn current(visuals: &egui::Visuals) -> Theme {
    if !visuals.dark_mode {
        Theme::Light
    } else if visuals.panel_fill == Color32::BLACK {
        Theme::HighContrast
    } else {
        Theme::Dark
    }
}
/// Color of a Git status letter in change rows; reuses the semantic palette.
pub(crate) fn change_color(colors: &Palette, marker: char) -> Color32 {
    match marker {
        'A' | 'U' | '?' => colors.success,
        'D' => colors.error,
        '!' => colors.warning,
        _ => colors.info,
    }
}

pub(crate) fn palette(visuals: &egui::Visuals) -> Palette {
    current(visuals).palette()
}

pub(crate) fn editor_theme(scheme: &str, visuals: &egui::Visuals) -> Theme {
    if scheme == "workbench" {
        current(visuals)
    } else {
        Theme::resolve(scheme, None)
    }
}

#[cfg(feature = "shared-editor")]
pub(crate) fn syntax_palette(scheme: &str) -> lumen_editor_surface::SyntaxPalette {
    match scheme {
        "lumen-light" => lumen_editor_surface::SyntaxPalette {
            comment: Color32::from_rgb(0, 128, 0),
            string: Color32::from_rgb(163, 21, 21),
            number: Color32::from_rgb(9, 134, 88),
            keyword: Color32::from_rgb(0, 0, 255),
            type_name: Color32::from_rgb(38, 127, 153),
            function: Color32::from_rgb(121, 94, 38),
            property: Color32::from_rgb(0, 0, 0),
            tag: Color32::from_rgb(128, 0, 0),
        },
        "lumen-high-contrast" => lumen_editor_surface::SyntaxPalette {
            comment: Color32::from_rgb(127, 255, 0),
            string: Color32::from_rgb(255, 255, 0),
            number: Color32::from_rgb(0, 255, 255),
            keyword: Color32::from_rgb(255, 128, 255),
            type_name: Color32::from_rgb(0, 255, 255),
            function: Color32::from_rgb(255, 255, 255),
            property: Color32::from_rgb(255, 255, 0),
            tag: Color32::from_rgb(255, 128, 0),
        },
        _ => lumen_editor_surface::SyntaxPalette {
            comment: Color32::from_rgb(106, 153, 85),
            string: Color32::from_rgb(206, 145, 120),
            number: Color32::from_rgb(181, 206, 168),
            keyword: Color32::from_rgb(197, 134, 192),
            type_name: Color32::from_rgb(78, 201, 176),
            function: Color32::from_rgb(220, 220, 170),
            property: Color32::from_rgb(156, 220, 254),
            tag: Color32::from_rgb(86, 156, 214),
        },
    }
}

#[cfg(all(test, feature = "shared-editor"))]
mod tests {
    use super::*;

    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |color: Color32| {
            let channel = |c: u8| {
                let c = c as f32 / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn schemes_pair_readable_syntax_and_surfaces() {
        for theme in [Theme::Dark, Theme::Light, Theme::HighContrast] {
            let colors = theme.palette();
            let syntax = syntax_palette(theme.scheme());
            for foreground in [
                colors.text,
                colors.muted,
                syntax.comment,
                syntax.string,
                syntax.number,
                syntax.keyword,
                syntax.type_name,
                syntax.function,
                syntax.property,
                syntax.tag,
            ] {
                assert!(
                    contrast(foreground, colors.bg) >= 4.5,
                    "{theme:?}: {foreground:?}"
                );
            }
            assert!(contrast(colors.text, colors.active) >= 4.5);
            for status in [colors.error, colors.warning] {
                assert!(
                    contrast(status, colors.panel) >= 4.5,
                    "{theme:?}: {status:?}"
                );
            }
            assert_eq!(current(&visuals(theme)), theme);
        }
    }

    #[test]
    fn explicit_editor_override_does_not_change_context_or_widget_identity() {
        use crate::app::LumenApp;
        let ctx = egui::Context::default();
        configure_theme(&ctx, Theme::HighContrast, false);
        let mut app = LumenApp::default();
        app.editor.new_file();
        app.view.focus_editor = true;
        app.view.settings.user.persisted.insert(
            "editor.color_scheme".into(),
            lumen_settings_core::Value::Choice("lumen-light".into()),
        );
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    app.render_component(ctx, |workbench| workbench.editor(ctx));
                },
            ));
        }
        assert_eq!(current(&ctx.style().visuals), Theme::HighContrast);
        assert!(output.unwrap().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == Color32::WHITE && rect.rect.height() > 100.0)));
        assert!(!app.editor.files[0].dirty);
        assert!(ctx.memory(
            |memory| memory.has_focus(crate::app::editor_action::text_widget_id(
                app.editor.files[0].id
            ))
        ));
    }

    #[test]
    fn named_code_schemes_have_distinct_keyword_colors_and_safe_default() {
        assert_ne!(
            syntax_palette("lumen-dark").keyword,
            syntax_palette("lumen-light").keyword
        );
        assert_ne!(
            syntax_palette("lumen-light").keyword,
            syntax_palette("lumen-high-contrast").keyword
        );
        assert_eq!(
            syntax_palette("unknown").keyword,
            syntax_palette("lumen-dark").keyword
        );
    }
}

pub(crate) fn configure_ui(ctx: &egui::Context) {
    configure_theme(ctx, Theme::Dark, false);
}

pub(crate) fn configure_theme(ctx: &egui::Context, theme: Theme, follow_system: bool) {
    ctx.set_theme(if theme == Theme::Light {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(if follow_system {
        egui::SystemTheme::SystemDefault
    } else if theme == Theme::Light {
        egui::SystemTheme::Light
    } else {
        egui::SystemTheme::Dark
    }));
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.spacing.item_spacing = Vec2::new(6.0, 0.0);
    style.spacing.button_padding = Vec2::new(6.0, 2.0);
    style.spacing.interact_size = Vec2::new(20.0, 22.0);
    style.interaction.resize_grab_radius_side = 5.0;
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(12.5));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(18.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.5));

    style.visuals = visuals(theme);
    ctx.set_style(style);
}

pub(crate) fn visuals(theme: Theme) -> egui::Visuals {
    let colors = theme.palette();
    let mut result = if theme == Theme::Light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let visuals = &mut result;
    visuals.panel_fill = colors.bg;
    visuals.window_fill = colors.panel;
    visuals.window_stroke = Stroke::new(1.0_f32, colors.float_border);
    visuals.window_corner_radius = CornerRadius::same(6);
    visuals.menu_corner_radius = CornerRadius::same(5);
    // Floating surfaces read as raised above the workbench through a soft drop
    // shadow; the light theme needs a gentler one.
    let float_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if theme == Theme::Light { 70 } else { 140 }),
    };
    visuals.window_shadow = float_shadow;
    visuals.popup_shadow = float_shadow;
    visuals.selection.bg_fill = colors.active;
    visuals.selection.stroke = Stroke::new(1.0_f32, colors.text);
    visuals.text_edit_bg_color = Some(colors.input);
    visuals.extreme_bg_color = colors.bg;
    visuals.weak_text_color = Some(colors.muted);
    visuals.hyperlink_color = colors.accent;
    // Text fields use noninteractive visuals when idle. Define all five states
    // so buttons, inputs, menus and selections share the same palette and radius.
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = colors.input;
        widget.weak_bg_fill = Color32::TRANSPARENT;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0_f32, colors.text);
        widget.corner_radius = CornerRadius::same(3);
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.fg_stroke.color = colors.muted;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, colors.border);
    visuals.widgets.hovered.bg_fill = colors.hover;
    visuals.widgets.hovered.weak_bg_fill = colors.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.active.bg_fill = colors.active;
    visuals.widgets.active.weak_bg_fill = colors.active;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, colors.accent);
    visuals.widgets.open.bg_fill = colors.active;
    visuals.widgets.open.weak_bg_fill = colors.active;
    visuals.widgets.open.bg_stroke.color = colors.accent;
    result
}

pub(crate) fn terminal_indexed_color(index: u8, color_scheme: &str, colors: Palette) -> Color32 {
    if color_scheme == "light" && index < 16 {
        const LIGHT: [Color32; 16] = [
            Color32::BLACK,
            Color32::from_rgb(170, 0, 0),
            Color32::from_rgb(0, 110, 0),
            Color32::from_rgb(120, 90, 0),
            Color32::from_rgb(0, 0, 170),
            Color32::from_rgb(150, 0, 150),
            Color32::from_rgb(0, 105, 115),
            Color32::from_gray(100),
            Color32::from_gray(80),
            Color32::from_rgb(190, 0, 0),
            Color32::from_rgb(0, 125, 0),
            Color32::from_rgb(135, 95, 0),
            Color32::from_rgb(0, 70, 190),
            Color32::from_rgb(155, 0, 155),
            Color32::from_rgb(0, 110, 125),
            Color32::from_gray(55),
        ];
        return LIGHT[index as usize];
    }
    if color_scheme == "high_contrast" {
        return HIGH_CONTRAST
            .get(index as usize)
            .copied()
            .unwrap_or(colors.text);
    }
    const ANSI: [Color32; 16] = [
        Color32::from_rgb(0, 0, 0),
        Color32::from_rgb(205, 49, 49),
        Color32::from_rgb(13, 188, 121),
        Color32::from_rgb(229, 229, 16),
        Color32::from_rgb(36, 114, 200),
        Color32::from_rgb(188, 63, 188),
        Color32::from_rgb(17, 168, 205),
        Color32::from_rgb(229, 229, 229),
        Color32::from_rgb(102, 102, 102),
        Color32::from_rgb(241, 76, 76),
        Color32::from_rgb(35, 209, 139),
        Color32::from_rgb(245, 245, 67),
        Color32::from_rgb(59, 142, 234),
        Color32::from_rgb(214, 112, 214),
        Color32::from_rgb(41, 184, 219),
        Color32::from_rgb(255, 255, 255),
    ];
    ANSI.get(index as usize).copied().unwrap_or(colors.text)
}

const HIGH_CONTRAST: [Color32; 16] = [
    Color32::BLACK,
    Color32::from_rgb(255, 85, 85),
    Color32::from_rgb(80, 255, 128),
    Color32::from_rgb(255, 255, 85),
    Color32::from_rgb(100, 170, 255),
    Color32::from_rgb(255, 110, 255),
    Color32::from_rgb(85, 255, 255),
    Color32::from_rgb(235, 235, 235),
    Color32::from_rgb(125, 125, 125),
    Color32::from_rgb(255, 120, 120),
    Color32::from_rgb(120, 255, 160),
    Color32::from_rgb(255, 255, 120),
    Color32::from_rgb(130, 190, 255),
    Color32::from_rgb(255, 150, 255),
    Color32::from_rgb(120, 255, 255),
    Color32::WHITE,
];
