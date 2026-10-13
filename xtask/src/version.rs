//! Version-only release edits, shared by the prepare, build, and publish jobs.
use std::{error::Error, fs, path::Path};

pub(super) fn bump(root: &Path, level: &str) -> Result<String, Box<dyn Error>> {
    let manifest = read_text(&root.join("Cargo.toml"))?;
    let current = current_version(&manifest)?;
    let next = bumped_version(&current, level)?;
    set(root, &next)
}

pub(super) fn set(root: &Path, target: &str) -> Result<String, Box<dyn Error>> {
    parse_version(target)?;
    let manifest_path = root.join("Cargo.toml");
    let lock_path = root.join("Cargo.lock");
    let plist_path = root.join("assets/macos/Info.plist");
    let manifest = read_text(&manifest_path)?;
    let lock = read_text(&lock_path)?;
    let plist = read_text(&plist_path)?;
    let current = current_version(&manifest)?;
    let updated = updated_contents(&manifest, &lock, &plist, &current, target)?;
    fs::write(manifest_path, updated.0)?;
    fs::write(lock_path, updated.1)?;
    fs::write(plist_path, updated.2)?;
    Ok(target.to_owned())
}

fn read_text(path: &Path) -> Result<String, Box<dyn Error>> {
    Ok(normalize_line_endings(&fs::read_to_string(path)?))
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn current_version(manifest: &str) -> Result<String, Box<dyn Error>> {
    let section = manifest
        .strip_prefix("[package]\n")
        .ok_or("Cargo.toml must start with [package]")?
        .split("\n[")
        .next()
        .ok_or("Missing package section")?;
    if !section
        .lines()
        .any(|line| line == "name = \"lumen-editor\"")
    {
        return Err("Unexpected root package name".into());
    }
    let versions: Vec<_> = section
        .lines()
        .filter_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
        .collect();
    let [version] = versions.as_slice() else {
        return Err("Expected one root package version".into());
    };
    parse_version(version)?;
    Ok((*version).to_owned())
}

fn parse_version(version: &str) -> Result<[u64; 3], Box<dyn Error>> {
    let parts: Vec<_> = version.split('.').collect();
    let [major, minor, patch] = parts.as_slice() else {
        return Err("Expected a numeric major.minor.patch version".into());
    };
    let parse = |part: &str| -> Result<u64, Box<dyn Error>> {
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.starts_with('0') && part.len() > 1)
        {
            return Err("Version components must be decimal without leading zeroes".into());
        }
        Ok(part.parse()?)
    };
    Ok([parse(major)?, parse(minor)?, parse(patch)?])
}

fn bumped_version(current: &str, level: &str) -> Result<String, Box<dyn Error>> {
    let [major, minor, patch] = parse_version(current)?;
    let next = match level {
        "major" => [major.checked_add(1).ok_or("Major version overflow")?, 0, 0],
        "minor" => [
            major,
            minor.checked_add(1).ok_or("Minor version overflow")?,
            0,
        ],
        "patch" => [
            major,
            minor,
            patch.checked_add(1).ok_or("Patch version overflow")?,
        ],
        _ => return Err("Bump level must be major, minor, or patch".into()),
    };
    Ok(format!("{}.{}.{}", next[0], next[1], next[2]))
}

fn updated_contents(
    manifest: &str,
    lock: &str,
    plist: &str,
    current: &str,
    target: &str,
) -> Result<(String, String, String), Box<dyn Error>> {
    let manifest = replace_once(
        manifest,
        &format!("[package]\nname = \"lumen-editor\"\nversion = \"{current}\""),
        &format!("[package]\nname = \"lumen-editor\"\nversion = \"{target}\""),
    )?;
    let lock = replace_once(
        lock,
        &format!("[[package]]\nname = \"lumen-editor\"\nversion = \"{current}\""),
        &format!("[[package]]\nname = \"lumen-editor\"\nversion = \"{target}\""),
    )?;
    let existing_bundle_version = plist_value(plist, "CFBundleShortVersionString")?;
    if existing_bundle_version != current {
        return Err("macOS bundle version does not match Cargo.toml".into());
    }
    let plist = replace_plist_value(plist, "CFBundleShortVersionString", target)?;
    let plist = replace_plist_value(&plist, "CFBundleVersion", target)?;
    Ok((manifest, lock, plist))
}

fn replace_once(source: &str, old: &str, new: &str) -> Result<String, Box<dyn Error>> {
    if source.matches(old).count() != 1 {
        return Err(format!("Expected one version entry matching {old:?}").into());
    }
    Ok(source.replacen(old, new, 1))
}

fn plist_value<'a>(plist: &'a str, key: &str) -> Result<&'a str, Box<dyn Error>> {
    let marker = format!("<key>{key}</key><string>");
    if plist.matches(&marker).count() != 1 {
        return Err(format!("Expected one {key} entry in Info.plist").into());
    }
    plist
        .split_once(&marker)
        .and_then(|(_, tail)| tail.split_once("</string>"))
        .map(|(value, _)| value)
        .ok_or_else(|| format!("Malformed {key} entry in Info.plist").into())
}

fn replace_plist_value(plist: &str, key: &str, target: &str) -> Result<String, Box<dyn Error>> {
    let current = plist_value(plist, key)?;
    replace_once(
        plist,
        &format!("<key>{key}</key><string>{current}</string>"),
        &format!("<key>{key}</key><string>{target}</string>"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> (String, String, String) {
        (
            normalize_line_endings(include_str!("../../Cargo.toml")),
            normalize_line_endings(include_str!("../../Cargo.lock")),
            normalize_line_endings(include_str!("../../assets/macos/Info.plist")),
        )
    }

    #[test]
    fn bump_resets_lower_components_and_rejects_invalid_versions() {
        assert_eq!(bumped_version("0.1.9", "patch").unwrap(), "0.1.10");
        assert_eq!(bumped_version("0.1.9", "minor").unwrap(), "0.2.0");
        assert_eq!(bumped_version("0.1.9", "major").unwrap(), "1.0.0");
        assert!(bumped_version("0.1.9", "other").is_err());
        assert!(parse_version("01.2.3").is_err());
        assert!(parse_version("+1.2.3").is_err());
        assert!(parse_version("1.2.3-beta").is_err());
    }

    #[test]
    fn windows_line_endings_are_normalized_before_version_parsing() {
        let manifest = "[package]\r\nname = \"lumen-editor\"\r\nversion = \"0.1.0\"\r\n";
        assert_eq!(
            current_version(&normalize_line_endings(manifest)).unwrap(),
            "0.1.0"
        );
    }

    #[test]
    fn updates_only_editor_and_bundle_versions() {
        let (manifest, lock, plist) = fixtures();
        let current = current_version(&manifest).unwrap();
        let (manifest, lock, plist) =
            updated_contents(&manifest, &lock, &plist, &current, "0.2.0").unwrap();
        assert_eq!(current_version(&manifest).unwrap(), "0.2.0");
        assert!(lock.contains("name = \"lumen-editor\"\nversion = \"0.2.0\""));
        assert_eq!(
            plist_value(&plist, "CFBundleShortVersionString").unwrap(),
            "0.2.0"
        );
        assert_eq!(plist_value(&plist, "CFBundleVersion").unwrap(), "0.2.0");
        assert!(updated_contents(&manifest, &lock, &plist, "0.2.0", "0.3.0").is_ok());
    }

    #[test]
    fn rejects_inconsistent_metadata_before_writing() {
        let (manifest, lock, plist) = fixtures();
        let current = current_version(&manifest).unwrap();
        assert!(updated_contents(&manifest, &lock, &plist, "9.9.9", "10.0.0").is_err());
        assert!(updated_contents(&manifest, &lock, &plist, &current, "0.1.1").is_ok());
        let wrong_lock = lock.replacen(
            &format!("name = \"lumen-editor\"\nversion = \"{current}\""),
            "name = \"lumen-editor\"\nversion = \"9.9.9\"",
            1,
        );
        assert!(updated_contents(&manifest, &wrong_lock, &plist, &current, "0.1.1").is_err());
        let wrong_plist = plist.replacen(
            &format!("<key>CFBundleShortVersionString</key><string>{current}</string>"),
            "<key>CFBundleShortVersionString</key><string>9.9.9</string>",
            1,
        );
        assert!(updated_contents(&manifest, &lock, &wrong_plist, &current, "0.1.1").is_err());
    }
}
