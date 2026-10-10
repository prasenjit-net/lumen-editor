//! Desktop-specific integrations, isolated from document workflows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) mod native_menu;

pub(crate) mod fonts;
