use std::{env, error::Error, fs, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("bundle") => {
            let mut release = false;
            let mut offline = false;
            let mut shared_editor = false;
            for arg in args {
                match arg.as_str() {
                    "--release" => release = true,
                    "--offline" => offline = true,
                    "--shared-editor" => shared_editor = true,
                    _ => return Err(format!("Unknown option: {arg}").into()),
                }
            }
            bundle(release, offline, shared_editor)
        }
        None | Some("--help" | "-h") => {
            println!(
                "Usage: cargo xtask bundle [--release] [--offline] [--shared-editor]\n\nBuild Lumen and package target/Lumen.app on macOS. The shared-editor feature builds a separate experimental bundle."
            );
            Ok(())
        }
        Some(command) => Err(format!("Unknown command: {command}. Use cargo xtask --help.").into()),
    }
}

fn bundle(release: bool, offline: bool, shared_editor: bool) -> Result<(), Box<dyn Error>> {
    if env::consts::OS != "macos" {
        return Err(
            "The .app bundler requires macOS. On Windows, use cargo build --release.".into(),
        );
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Cannot locate workspace root")?
        .to_path_buf();
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
    if shared_editor {
        build.args(["--features", "shared-editor"]);
    }
    if !build.status()?.success() {
        return Err("Editor build failed; the existing app bundle was left unchanged.".into());
    }

    let profile = if release { "release" } else { "debug" };
    let executable = target.join(&host).join(profile).join("lumen-editor");
    let app = target.join(bundle_name(shared_editor));
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    fs::create_dir_all(&macos)?;
    // Replace by rename rather than truncating an executable that may be running.
    let staged = macos.join("lumen-editor.bundle-tmp");
    fs::copy(&executable, &staged)?;
    fs::rename(&staged, macos.join("lumen-editor"))?;
    let plist = fs::read_to_string(root.join("assets/macos/Info.plist"))?;
    let plist = bundle_plist(&plist, shared_editor);
    fs::write(contents.join("Info.plist"), plist)?;
    println!("Built {} ({profile})", app.display());
    Ok(())
}

fn bundle_name(experimental: bool) -> &'static str {
    if experimental {
        "Lumen Experimental.app"
    } else {
        "Lumen.app"
    }
}

fn bundle_plist(plist: &str, experimental: bool) -> String {
    if experimental {
        plist
            .replace(
                "<string>Lumen</string>",
                "<string>Lumen Experimental</string>",
            )
            .replace(
                "<string>dev.lumen.editor</string>",
                "<string>dev.lumen.editor.experimental</string>",
            )
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
        assert_eq!(bundle_plist(original, false), original);
        assert_ne!(bundle_name(false), bundle_name(true));
        let experimental = bundle_plist(original, true);
        assert!(experimental.contains("<string>dev.lumen.editor.experimental</string>"));
        assert!(experimental.contains("<string>Lumen Experimental</string>"));
        assert!(
            experimental.contains("<key>CFBundleExecutable</key><string>lumen-editor</string>")
        );
    }
}
