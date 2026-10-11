//! Desktop-specific integrations, isolated from document workflows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) mod native_menu;

#[cfg(feature = "shared-editor")]
pub(crate) mod editor_fonts;
pub(crate) mod fonts;
