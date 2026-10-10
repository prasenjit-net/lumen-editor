/// Accept one portable file name, not a path, so creation stays in the target directory.
pub(crate) fn validate_file_name(input: &str) -> Result<&str, &'static str> {
    let name = input.trim();
    if name.is_empty() {
        return Err("Enter a file name.");
    }
    if name == "." || name == ".." || name.ends_with('.') {
        return Err("Choose a file name that does not end with a dot.");
    }
    if name
        .chars()
        .any(|character| character.is_control() || "/\\:*?\"<>|".contains(character))
    {
        return Err("Use a file name without path separators or reserved characters.");
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
    {
        return Err("Choose a name that is not reserved by Windows.");
    }
    Ok(name)
}

pub(crate) fn validate_relative_name(input: &str) -> Result<std::path::PathBuf, &'static str> {
    let normalized = input.trim().replace('\\', "/");
    let normalized = normalized.strip_suffix('/').unwrap_or(&normalized);
    let mut path = std::path::PathBuf::new();
    for component in normalized.split('/') {
        path.push(validate_file_name(component)?);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::validate_file_name;

    #[test]
    fn portable_names_support_extensions_dotfiles_unicode_and_spaces() {
        for name in [
            "main.rs",
            ".gitignore",
            "notes with spaces.md",
            "résumé.txt",
            "文件.txt",
        ] {
            assert_eq!(validate_file_name(name), Ok(name));
        }
        assert_eq!(validate_file_name("  main.rs  "), Ok("main.rs"));
    }

    #[test]
    fn reject_paths_invalid_characters_and_windows_reserved_names() {
        for name in [
            "",
            "   ",
            ".",
            "..",
            "a/../b",
            "a\\b",
            "C:notes.txt",
            "bad?name",
            "bad*name",
            "name.",
            "a\nb",
            "a\0b",
            "NUL",
            "con.txt",
            "COM1.rs",
            "lpt9.txt",
        ] {
            assert!(
                validate_file_name(name).is_err(),
                "invalid name accepted: {name:?}"
            );
        }
    }
}
