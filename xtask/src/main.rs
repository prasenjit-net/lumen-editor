use std::{env, error::Error, fs, path::PathBuf, process::Command};

mod version;

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
                "Usage: cargo xtask bundle [--release] [--offline]\n       cargo xtask bump-version <major|minor|patch>\n       cargo xtask set-version <major.minor.patch>\n\nPackage target/Lumen.app, or update the application version in Cargo.toml, Cargo.lock, and Info.plist."
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
    if !build.status()?.success() {
        return Err("Editor build failed; the existing app bundle was left unchanged.".into());
    }

    let profile = if release { "release" } else { "debug" };
    let executable = target.join(&host).join(profile).join("lumen-editor");
    let app = target.join("Lumen.app");
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    // Replace by rename rather than truncating an executable that may be running.
    let staged = macos.join("lumen-editor.bundle-tmp");
    fs::copy(&executable, &staged)?;
    fs::rename(&staged, macos.join("lumen-editor"))?;
    fs::write(
        resources.join("Lumen.icns"),
        icns_from_png(&fs::read(root.join("assets/icons/lumen.png"))?)?,
    )?;
    let plist = fs::read_to_string(root.join("assets/macos/Info.plist"))?;
    fs::write(contents.join("Info.plist"), plist)?;
    println!("Built {} ({profile})", app.display());
    Ok(())
}

/// The modern ICNS `ic10` representation contains a complete PNG image.
/// Keeping the source PNG in the repository makes one authored icon serve the
/// runtime window icon on every platform and the macOS application bundle.
fn icns_from_png(png: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !png.starts_with(PNG_SIGNATURE) {
        return Err("The bundled application icon is not a PNG.".into());
    }
    let chunk_length = u32::try_from(png.len() + 8)?;
    let total_length = chunk_length.checked_add(8).ok_or("Icon is too large.")?;
    let mut icon = Vec::with_capacity(total_length as usize);
    icon.extend_from_slice(b"icns");
    icon.extend_from_slice(&total_length.to_be_bytes());
    icon.extend_from_slice(b"ic10");
    icon.extend_from_slice(&chunk_length.to_be_bytes());
    icon.extend_from_slice(png);
    Ok(icon)
}

fn workspace_root() -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Cannot locate workspace root")?
        .to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macos_icon_wraps_the_shared_png_as_an_icns_chunk() {
        let png = include_bytes!("../../assets/icons/lumen.png");
        let icon = icns_from_png(png).unwrap();
        assert_eq!(&icon[..4], b"icns");
        assert_eq!(&icon[8..12], b"ic10");
        assert_eq!(&icon[16..], png);
        assert_eq!(
            u32::from_be_bytes(icon[4..8].try_into().unwrap()) as usize,
            icon.len()
        );
    }
}
