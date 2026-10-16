//! Platform-correct shortcut labels shared by menus, hints and tooltips.

/// Label for a shortcut that uses the platform's primary modifier
/// (Command on macOS, Ctrl elsewhere). `keys` is the optional extra modifiers
/// followed by the key, e.g. `"Shift+A"` or `"Alt+F"`.
pub(crate) fn primary(keys: &str) -> String {
    label(cfg!(target_os = "macos"), keys)
}

fn label(mac: bool, keys: &str) -> String {
    if !mac {
        return format!("Ctrl+{keys}");
    }
    // The bundled UI font has the Command glyph but not Shift or Option, which
    // would draw as empty boxes; spell those modifiers out.
    match keys.rsplit_once('+') {
        Some((modifiers, key)) => format!("{modifiers}+\u{2318}{key}"),
        None => format!("\u{2318}{keys}"),
    }
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn macos_uses_command_glyphs_and_other_platforms_use_ctrl() {
        assert_eq!(label(true, "N"), "⌘N");
        assert_eq!(label(true, "Shift+O"), "Shift+⌘O");
        assert_eq!(label(true, "Alt+F"), "Alt+⌘F");
        assert_eq!(label(true, "Shift+Alt+F"), "Shift+Alt+⌘F");
        assert_eq!(label(false, "N"), "Ctrl+N");
        assert_eq!(label(false, "Shift+O"), "Ctrl+Shift+O");
    }
}
