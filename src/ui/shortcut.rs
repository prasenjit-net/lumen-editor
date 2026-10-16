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
    let mut glyphs = String::new();
    let mut rest = keys;
    for (name, glyph) in [("Alt+", "⌥"), ("Shift+", "⇧")] {
        if let Some(stripped) = rest.strip_prefix(name) {
            glyphs.push_str(glyph);
            rest = stripped;
        }
    }
    // Alt may follow Shift in a spec; keep macOS ordering (⌥⇧⌘) either way.
    if let Some(stripped) = rest.strip_prefix("Alt+") {
        glyphs.insert(0, '⌥');
        rest = stripped;
    }
    format!("{glyphs}⌘{rest}")
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn macos_uses_command_glyphs_and_other_platforms_use_ctrl() {
        assert_eq!(label(true, "N"), "⌘N");
        assert_eq!(label(true, "Shift+O"), "⇧⌘O");
        assert_eq!(label(true, "Alt+F"), "⌥⌘F");
        assert_eq!(label(true, "Shift+Alt+F"), "⌥⇧⌘F");
        assert_eq!(label(false, "N"), "Ctrl+N");
        assert_eq!(label(false, "Shift+O"), "Ctrl+Shift+O");
    }
}
