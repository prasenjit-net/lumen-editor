use std::{env, error::Error, fs, path::PathBuf, process::Command};

mod version;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum BundleEditor {
    #[default]
    Standard,
    Experimental,
    Legacy,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some(command @ ("bump-version" | "set-version")) => {
            let value = args.next().ok_or("Missing version or bump level")?;
            if args.next().is_some() {
                return Err("Expected exactly one version or bump level".into());
            }
            let root = workspace_root()?;
            let new_version = if command == "bump-version" {
                version::bump(&root, &value)?
            } else {
                version::set(&root, &value)?
            };
            println!("{new_version}");
            Ok(())
        }
        Some("bundle") => {
            let mut release = false;
            let mut offline = false;
            let mut editor = BundleEditor::Standard;
            for arg in args {
                match arg.as_str() {
                    "--release" => release = true,
                    "--offline" => offline = true,
                    "--shared-editor" | "--legacy-editor" => {
                        if editor != BundleEditor::Standard {
                            return Err("Select only one editor bundle option.".into());
                        }
                        editor = if arg == "--shared-editor" {
                            BundleEditor::Experimental
                        } else {
                            BundleEditor::Legacy
                        };
                    }
                    _ => return Err(format!("Unknown option: {arg}").into()),
                }
            }
            bundle(release, offline, editor)
        }
        None | Some("--help" | "-h") => {
            println!(
                "Usage: cargo xtask bundle [--release] [--offline] [--shared-editor | --legacy-editor]\n       cargo xtask bump-version <major|minor|patch>\n       cargo xtask set-version <major.minor.patch>\n\nPackage target/Lumen.app, or update the application version in Cargo.toml, Cargo.lock, and Info.plist."
            );
            Ok(())
        }
        Some(command) => Err(format!("Unknown command: {command}. Use cargo xtask --help.").into()),
    }
}

fn bundle(release: bool, offline: bool, editor: BundleEditor) -> Result<(), Box<dyn Error>> {
    if env::consts::OS != "macos" {
        return Err(
            "The .app bundler requires macOS. On Windows, use cargo build --release.".into(),
        );
    }
    let root = workspace_root()?;
    let target = match env::var_os("CARGO_TARGET_DIR") {
        Some(path) => {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        }
        None => root.join("target"),
    };
    // Select the host explicitly so a cross-compilation default cannot package
    // an incompatible executable. Cargo builds the editor, never this helper.
    let host = format!("{}-apple-darwin", env::consts::ARCH);
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut build = Command::new(cargo);
    build
        .current_dir(&root)
        .args([
            "build",
            "--package",
            "lumen-editor",
            "--bin",
            "lumen-editor",
            "--target",
            &host,
        ])
        .arg("--target-dir")
        .arg(&target);
    if release {
        build.arg("--release");
    }
    if offline {
        build.arg("--offline");
    }
    build.args(editor_feature_args(editor));
    if !build.status()?.success() {
        return Err("Editor build failed; the existing app bundle was left unchanged.".into());
    }

    let profile = if release { "release" } else { "debug" };
    let executable = target.join(&host).join(profile).join("lumen-editor");
    let app = target.join(bundle_name(editor));
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    fs::create_dir_all(&macos)?;
    // Replace by rename rather than truncating an executable that may be running.
    let staged = macos.join("lumen-editor.bundle-tmp");
    fs::copy(&executable, &staged)?;
    fs::rename(&staged, macos.join("lumen-editor"))?;
    let plist = fs::read_to_string(root.join("assets/macos/Info.plist"))?;
    let plist = bundle_plist(&plist, editor);
    fs::write(contents.join("Info.plist"), plist)?;
    println!("Built {} ({profile})", app.display());
    Ok(())
}

fn workspace_root() -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Cannot locate workspace root")?
        .to_path_buf())
}

fn editor_feature_args(editor: BundleEditor) -> &'static [&'static str] {
    match editor {
        BundleEditor::Standard => &[],
        BundleEditor::Experimental => &["--features", "shared-editor"],
        BundleEditor::Legacy => &["--no-default-features"],
    }
}

fn bundle_name(editor: BundleEditor) -> &'static str {
    match editor {
        BundleEditor::Standard => "Lumen.app",
        BundleEditor::Experimental => "Lumen Experimental.app",
        BundleEditor::Legacy => "Lumen Legacy.app",
    }
}

fn bundle_plist(plist: &str, editor: BundleEditor) -> String {
    if editor != BundleEditor::Standard {
        let (name, identifier) = match editor {
            BundleEditor::Experimental => (
                "<string>Lumen Experimental</string>",
                "<string>dev.lumen.editor.experimental</string>",
            ),
            BundleEditor::Legacy => (
                "<string>Lumen Legacy</string>",
                "<string>dev.lumen.editor.legacy</string>",
            ),
            BundleEditor::Standard => unreachable!(),
        };
        plist
            .replace("<string>Lumen</string>", name)
            .replace("<string>dev.lumen.editor</string>", identifier)
    } else {
        plist.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn experimental_bundle_has_separate_identity_and_preserves_executable_metadata() {
        let original = include_str!("../../assets/macos/Info.plist");
        assert_eq!(bundle_plist(original, BundleEditor::Standard), original);
        assert_ne!(
            bundle_name(BundleEditor::Standard),
            bundle_name(BundleEditor::Experimental)
        );
        let experimental = bundle_plist(original, BundleEditor::Experimental);
        assert!(experimental.contains("<string>dev.lumen.editor.experimental</string>"));
        assert!(experimental.contains("<string>Lumen Experimental</string>"));
        assert!(
            experimental.contains("<key>CFBundleExecutable</key><string>lumen-editor</string>")
        );
    }
    #[test]
    fn legacy_bundle_disables_default_features_and_has_separate_identity() {
        assert!(editor_feature_args(BundleEditor::Standard).is_empty());
        assert_eq!(
            editor_feature_args(BundleEditor::Legacy),
            &["--no-default-features"]
        );
        let original = include_str!("../../assets/macos/Info.plist");
        let legacy = bundle_plist(original, BundleEditor::Legacy);
        assert_eq!(bundle_name(BundleEditor::Legacy), "Lumen Legacy.app");
        assert!(legacy.contains("<string>dev.lumen.editor.legacy</string>"));
        assert!(legacy.contains("<key>CFBundleExecutable</key><string>lumen-editor</string>"));
    }
}
