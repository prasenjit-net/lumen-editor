//! Autosave, external-change checks and crash-recovery orchestration.
use super::LumenApp;
use crate::{model::DocumentId, services::recovery::RecoveryDocument};
use eframe::egui;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub(crate) struct SafetyState {
    last_edits: HashMap<DocumentId, (u64, Instant)>,
    last_backup_keys: Vec<BackupKey>,
    backup_at: Option<Instant>,
    disk_check_at: Option<Instant>,
    focused_editor: Option<DocumentId>,
    window_focused: Option<bool>,
    recovery_ready: bool,
}

#[derive(PartialEq, Eq)]
struct BackupKey {
    id: DocumentId,
    revision: u64,
    path: Option<std::path::PathBuf>,
    name: String,
}

impl Default for SafetyState {
    fn default() -> Self {
        Self {
            last_edits: HashMap::new(),
            last_backup_keys: Vec::new(),
            backup_at: None,
            disk_check_at: None,
            focused_editor: None,
            window_focused: None,
            recovery_ready: true,
        }
    }
}

impl LumenApp {
    pub(crate) fn restore_recovery(&mut self) {
        match self.recovery_store.load() {
            Ok(loaded) => {
                for warning in loaded.warnings {
                    self.view.safety_notices.push(warning);
                }
                let mut restored = 0;
                for snapshot in loaded.documents {
                    if snapshot.text == snapshot.saved_text {
                        continue;
                    }
                    let duplicate = snapshot.path.as_ref().is_some_and(|path| {
                        self.editor
                            .files
                            .iter()
                            .any(|file| file.path.as_ref() == Some(path))
                    });
                    let name = if duplicate {
                        format!("{} (Recovered)", snapshot.name)
                    } else {
                        snapshot.name
                    };
                    let document = crate::model::Document::restored(
                        if duplicate { None } else { snapshot.path },
                        name,
                        snapshot.text,
                        snapshot.saved_text,
                    );
                    if let Some(number) = document
                        .name
                        .strip_prefix("untitled-")
                        .and_then(|name| name.strip_suffix(".txt"))
                        .and_then(|name| name.parse::<usize>().ok())
                    {
                        self.editor.next_untitled =
                            self.editor.next_untitled.max(number.saturating_add(1));
                    }
                    self.editor.files.push(document);
                    self.editor
                        .set_active_index(Some(self.editor.files.len() - 1));
                    restored += 1;
                }
                if restored > 0 {
                    self.view.status =
                        format!("Restored {restored} unsaved document(s) from recovery");
                } else if !self.editor.has_unsaved_changes()
                    && let Err(error) = self.recovery_store.save(&[])
                {
                    let notice = format!("Could not retire stale recovery data: {error}");
                    self.view.status = notice.clone();
                    self.view.safety_notices.push(notice);
                }
            }
            Err(error) => {
                self.safety.recovery_ready = false;
                let notice = format!(
                    "Could not read recovery data: {error}. Existing backup was left untouched."
                );
                self.view.status = notice.clone();
                self.view.safety_notices.push(notice);
            }
        }
    }

    pub(crate) fn maintain_documents(&mut self, ctx: &egui::Context) {
        if self.view.allow_quit {
            return;
        }
        let now = Instant::now();
        self.safety
            .last_edits
            .retain(|id, _| self.editor.files.iter().any(|file| file.id == *id));
        self.view
            .fold_tabs
            .retain(|id, _| self.editor.files.iter().any(|file| file.id == *id));
        for file in &self.editor.files {
            if file.dirty {
                let entry = self
                    .safety
                    .last_edits
                    .entry(file.id)
                    .or_insert((file.revision, now));
                if entry.0 != file.revision {
                    *entry = (file.revision, now);
                }
            }
        }

        let focused_editor = self
            .editor
            .files
            .iter()
            .find(|file| {
                ctx.memory(|memory| memory.has_focus(super::editor_action::text_widget_id(file.id)))
            })
            .map(|file| file.id);
        let window_focused = ctx.input(|input| input.focused);
        let focus_lost = self
            .safety
            .focused_editor
            .filter(|id| Some(*id) != focused_editor);
        let window_lost = self.safety.window_focused == Some(true) && !window_focused;
        self.safety.focused_editor = focused_editor;
        self.safety.window_focused = Some(window_focused);

        let mut due = Vec::new();
        for (index, file) in self.editor.files.iter().enumerate() {
            if !file.dirty || file.path.is_none() || file.disk_changed {
                continue;
            }
            let policy = match self
                .view
                .settings
                .effective(
                    lumen_settings_core::setting("files.auto_save"),
                    file.path.as_deref(),
                )
                .0
            {
                lumen_settings_core::Value::Choice(policy) => policy,
                _ => unreachable!(),
            };
            let should_save = match policy.as_str() {
                "afterDelay" => self
                    .safety
                    .last_edits
                    .get(&file.id)
                    .is_some_and(|(_, edited)| {
                        now.duration_since(*edited)
                            >= Duration::from_millis(
                                self.view
                                    .settings
                                    .number("files.auto_save_delay", file.path.as_deref())
                                    as u64,
                            )
                    }),
                "onFocusChange" => focus_lost == Some(file.id),
                "onWindowChange" => window_lost,
                _ => false,
            };
            if should_save {
                due.push(index);
            }
        }
        for index in due {
            if let Some(path) = self.editor.files[index].path.clone() {
                self.save_file_at(index, path);
            }
            if let Some(file) = self.editor.files.get(index) {
                self.safety.last_edits.insert(file.id, (file.revision, now));
            }
        }

        if self
            .safety
            .disk_check_at
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(2))
        {
            self.safety.disk_check_at = Some(now);
            self.check_open_disk_changes();
        }
        self.write_recovery(now);
        if self
            .editor
            .files
            .iter()
            .any(|file| file.path.is_some() || file.dirty)
        {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    pub(crate) fn check_open_disk_changes(&mut self) {
        for file in self
            .editor
            .files
            .iter_mut()
            .filter(|file| !file.is_image() && !file.is_preview())
        {
            let Some(path) = &file.path else {
                continue;
            };
            let changed = match self.file_system.read_text(path) {
                Ok(text) => text != file.saved_text(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(_) => continue,
            };
            if changed && !file.disk_changed {
                self.view.status = format!(
                    "{} changed on disk. Saving will require confirmation.",
                    file.name
                );
            }
            file.disk_changed = changed;
        }
    }

    fn write_recovery(&mut self, now: Instant) {
        if !self.safety.recovery_ready {
            return;
        }
        let keys: Vec<_> = self
            .editor
            .files
            .iter()
            .filter(|file| file.dirty)
            .map(|file| BackupKey {
                id: file.id,
                revision: file.revision,
                path: file.path.clone(),
                name: file.name.clone(),
            })
            .collect();
        if keys == self.safety.last_backup_keys {
            return;
        }
        if !keys.is_empty()
            && self
                .safety
                .backup_at
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
        {
            return;
        }
        let snapshots: Vec<_> = self
            .editor
            .files
            .iter()
            .filter(|file| file.dirty)
            .map(|file| RecoveryDocument {
                path: file.path.clone(),
                name: file.name.clone(),
                text: file.text.clone(),
                saved_text: file.saved_text().into(),
            })
            .collect();
        match self.recovery_store.save(&snapshots) {
            Ok(()) => {
                self.safety.last_backup_keys = keys;
                self.safety.backup_at = Some(now);
                self.view
                    .safety_notices
                    .retain(|notice| !notice.starts_with("Could not back up unsaved edits:"));
            }
            Err(error) => {
                let notice = format!("Could not back up unsaved edits: {error}");
                self.view.status = notice.clone();
                if !self
                    .view
                    .safety_notices
                    .iter()
                    .any(|existing| existing.starts_with("Could not back up unsaved edits:"))
                {
                    self.view.safety_notices.push(notice);
                }
                self.safety.backup_at = Some(now);
            }
        }
    }

    pub(crate) fn clear_recovery_for_exit(&mut self) -> bool {
        if !self.safety.recovery_ready {
            return true;
        }
        match self.recovery_store.save(&[]) {
            Ok(()) => true,
            Err(error) => {
                let notice = format!("Could not clear recovery snapshot before quitting: {error}");
                self.view.status = notice.clone();
                self.view.safety_notices.push(notice);
                false
            }
        }
    }
}
