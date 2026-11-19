//! "Shell Command: Install/Uninstall 'lumen' command in PATH". The change runs on
//! a worker thread because macOS may show an administrator password prompt.
use super::LumenApp;
use crate::services::{
    cli_launcher::{self, LauncherAction, LauncherOutcome, LauncherTarget},
    logger,
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

/// A launcher change in progress.
pub(crate) struct LauncherJob {
    receiver: Receiver<LauncherOutcome>,
}

impl LumenApp {
    pub(crate) fn change_shell_command(&mut self, action: LauncherAction, ctx: &egui::Context) {
        let Some(target) = cli_launcher::default_target() else {
            self.finish_launcher_job(LauncherOutcome::Failed(
                "cannot determine where to install it on this system".into(),
            ));
            return;
        };
        let exe = match action {
            LauncherAction::Install => match cli_launcher::current_executable() {
                Ok(exe) => exe,
                Err(reason) => {
                    self.finish_launcher_job(LauncherOutcome::Failed(reason));
                    return;
                }
            },
            // Removal does not need to know where Lumen is.
            LauncherAction::Uninstall => PathBuf::new(),
        };
        self.start_launcher_job(action, exe, target, ctx);
    }

    /// Start the change unless one is already running.
    pub(crate) fn start_launcher_job(
        &mut self,
        action: LauncherAction,
        exe: PathBuf,
        target: LauncherTarget,
        ctx: &egui::Context,
    ) {
        if self.view.launcher_job.is_some() {
            self.view.status = "The 'lumen' shell command is already being changed".into();
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(cli_launcher::run(action, &exe, &target));
            worker_ctx.request_repaint();
        });
        self.view.launcher_job = Some(LauncherJob { receiver });
        self.view.status = match action {
            LauncherAction::Install => "Installing the 'lumen' shell command…",
            LauncherAction::Uninstall => "Removing the 'lumen' shell command…",
        }
        .into();
    }

    pub(crate) fn poll_launcher_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.view.launcher_job else {
            return;
        };
        let outcome = match job.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(200));
                return;
            }
            Err(TryRecvError::Disconnected) => {
                LauncherOutcome::Failed("the installer stopped unexpectedly".into())
            }
        };
        self.view.launcher_job = None;
        self.finish_launcher_job(outcome);
    }

    fn finish_launcher_job(&mut self, outcome: LauncherOutcome) {
        let message = outcome.message();
        match outcome {
            LauncherOutcome::Failed(_) | LauncherOutcome::Foreign { .. } => {
                logger::warn("shell-command", &message)
            }
            _ => logger::info("shell-command", &message),
        }
        self.view.status = message;
    }
}
