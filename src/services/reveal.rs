use std::{io, path::Path, process::Command};

pub(crate) fn reveal_in_file_manager(path: &Path) -> io::Result<()> {
    let path = std::fs::canonicalize(path)?;
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg("-R").arg(&path);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        command.arg(format!("/select,{}", path.display()));
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(if path.is_dir() {
            &path
        } else {
            path.parent().unwrap_or(&path)
        });
        command
    };
    command.spawn().map(|_| ())
}

/// Open a web or mail link with the platform's default handler. Only
/// `http`, `https` and `mailto` are accepted so a document cannot launch
/// arbitrary schemes or programs.
pub(crate) fn open_external(url: &str) -> io::Result<()> {
    let lower = url.to_ascii_lowercase();
    if !["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only http, https and mailto links can be opened",
        ));
    }
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler").arg(url);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    command.spawn().map(|_| ())
}
