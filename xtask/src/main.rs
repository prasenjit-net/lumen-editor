use std::{env, error::Error, fs, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("bundle") => {
            let mut release = false;
            let mut offline = false;
            for arg in args {
                match arg.as_str() {
                    "--release" => release = true,
                    "--offline" => offline = true,
                    _ => return Err(format!("Unknown option: {arg}").into()),
                }
            }
            bundle(release, offline)
        }
        None | Some("--help" | "-h") => {
            println!(
                "Usage: cargo xtask bundle [--release] [--offline]\n\nBuild Lumen and package target/Lumen.app on macOS."
            );
            Ok(())
        }
        Some(command) => Err(format!("Unknown command: {command}. Use cargo xtask --help.").into()),
    }
}

fn bundle(release: bool, offline: bool) -> Result<(), Box<dyn Error>> {
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
    if !build.status()?.success() {
        return Err("Editor build failed; the existing app bundle was left unchanged.".into());
    }

    let profile = if release { "release" } else { "debug" };
    let executable = target.join(&host).join(profile).join("lumen-editor");
    let app = target.join("Lumen.app");
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    fs::create_dir_all(&macos)?;
    // Replace by rename rather than truncating an executable that may be running.
    let staged = macos.join("lumen-editor.bundle-tmp");
    fs::copy(&executable, &staged)?;
    fs::rename(&staged, macos.join("lumen-editor"))?;
    fs::copy(
        root.join("assets/macos/Info.plist"),
        contents.join("Info.plist"),
    )?;
    println!("Built {} ({profile})", app.display());
    Ok(())
}
