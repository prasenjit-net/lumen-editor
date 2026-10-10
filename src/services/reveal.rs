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
