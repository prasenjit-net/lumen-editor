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
    format!("{}{}", mac_modifiers(keys), "\u{2318}") + key_of(keys)
}

/// Label for a fixed shortcut spelled with explicit modifiers (`Ctrl+Tab`,
/// `Shift+F12`). macOS shows modifier glyphs; other platforms keep the text.
pub(crate) fn explicit(keys: &str) -> String {
    explicit_label(cfg!(target_os = "macos"), keys)
}

fn explicit_label(mac: bool, keys: &str) -> String {
    if !mac {
        return keys.to_owned();
    }
    format!("{}{}", mac_modifiers(keys), key_of(keys))
}

fn key_of(keys: &str) -> &str {
    keys.rsplit_once('+').map_or(keys, |(_, key)| key)
}

/// Modifier glyphs in the macOS order (Control, Option, Shift). Command is
/// appended by the caller since `primary` always includes it.
fn mac_modifiers(keys: &str) -> String {
    let Some((modifiers, _)) = keys.rsplit_once('+') else {
        return String::new();
    };
    let has = |name: &str| modifiers.split('+').any(|part| part == name);
    let mut result = String::new();
    if has("Ctrl") {
        result.push('\u{2303}');
    }
    if has("Alt") {
        result.push('\u{2325}');
    }
    if has("Shift") {
        result.push('\u{21E7}');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn macos_uses_modifier_glyphs_and_other_platforms_use_ctrl() {
        assert_eq!(label(true, "N"), "⌘N");
        assert_eq!(label(true, "Shift+O"), "⇧⌘O");
        assert_eq!(label(true, "Alt+F"), "⌥⌘F");
        assert_eq!(label(true, "Shift+Alt+F"), "⌥⇧⌘F");
        assert_eq!(label(false, "N"), "Ctrl+N");
        assert_eq!(label(false, "Shift+O"), "Ctrl+Shift+O");
    }

    #[test]
    fn explicit_shortcuts_gain_glyphs_only_on_macos() {
        assert_eq!(super::explicit_label(true, "Ctrl+Shift+Tab"), "⌃⇧Tab");
        assert_eq!(super::explicit_label(true, "F12"), "F12");
        assert_eq!(
            super::explicit_label(false, "Ctrl+Shift+Tab"),
            "Ctrl+Shift+Tab"
        );
    }
}
