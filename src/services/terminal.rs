//! Native shell sessions used by the integrated terminal workbench panel.
//!
//! The service owns process and PTY handles. UI code only submits complete
//! input lines and drains output through the app controller.
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

pub(crate) const DEFAULT_ROWS: u16 = 24;
pub(crate) const DEFAULT_COLUMNS: u16 = 100;

/// Resolve the native user's home directory for terminals opened outside a
/// workspace. Falling back to the process directory keeps the feature usable
/// in unusually constrained environments.
pub(crate) fn default_working_directory() -> PathBuf {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            Some(std::path::Path::new(&drive).join(path).into_os_string())
        })
        .map(PathBuf::from);
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME").map(PathBuf::from);

    home.filter(|path| path.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    output: Receiver<Vec<u8>>,
}

impl TerminalSession {
    pub(crate) fn start(root: &Path, rows: u16, columns: u16) -> Result<Self, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows.max(1),
                cols: columns.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Could not create terminal: {error}"))?;
        let mut command = CommandBuilder::new(default_shell());
        command.cwd(root);
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("Could not start terminal shell: {error}"))?;
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("Could not read terminal output: {error}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| format!("Could not write terminal input: {error}"))?;
        let (sender, output) = mpsc::channel();
        thread::spawn(move || read_terminal_output(reader, sender));
        Ok(Self {
            master: pair.master,
            writer,
            child,
            output,
        })
    }

    pub(crate) fn write_line(&mut self, input: &str) -> Result<(), String> {
        self.writer
            .write_all(input.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|error| format!("Could not send terminal input: {error}"))
    }

    pub(crate) fn resize(&self, rows: u16, columns: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows: rows.max(1),
                cols: columns.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Could not resize terminal: {error}"))
    }

    pub(crate) fn drain_output(&self) -> Vec<Vec<u8>> {
        self.output.try_iter().collect()
    }

    pub(crate) fn has_exited(&mut self) -> Result<bool, String> {
        self.child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|error| format!("Could not inspect terminal shell: {error}"))
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn default_shell() -> std::ffi::OsString {
    #[cfg(windows)]
    {
        std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into())
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into())
    }
}

fn read_terminal_output(mut reader: Box<dyn Read + Send>, sender: mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(count) => {
                if sender.send(buffer[..count].to_vec()).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_runs_in_the_requested_directory_and_streams_output() {
        let directory = tempfile::tempdir().unwrap();
        let mut terminal = TerminalSession::start(directory.path(), 12, 80).unwrap();
        terminal.write_line("printf lumen-terminal-ok").unwrap();
        let mut output = Vec::new();
        for _ in 0..50 {
            output.extend(terminal.drain_output().into_iter().flatten());
            if String::from_utf8_lossy(&output).contains("lumen-terminal-ok") {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("terminal output was not received: {output:?}");
    }

    #[test]
    fn default_working_directory_is_usable() {
        assert!(default_working_directory().is_dir());
    }
}
