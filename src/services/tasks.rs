//! Task configuration storage. Reuses the settings store's guarantees (symlink and
//! containment refusal, conflict check against the text the draft was loaded from,
//! staged atomic replace, no-clobber create) so task drafts never overwrite
//! concurrent edits. Parsing, validation and TOML edits live in `lumen-task-core`.
// The controller and task editor that call this arrive in later phases.
#![allow(dead_code)]
use std::path::{Path, PathBuf};

use lumen_task_core::{Issue, TaskDef, TaskFile};

use super::settings::SettingsStore;

pub(crate) fn user_path() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".lumen/tasks.toml"))
}

pub(crate) fn workspace_path(root: &Path) -> PathBuf {
    root.join(".lumen/tasks.toml")
}

/// One read of a tasks file. `text` is what a later save must still match.
pub(crate) struct LoadedTasks {
    pub(crate) text: Option<String>,
    /// A missing file is an empty list; a bad file keeps its issues so the UI can
    /// show them instead of guessing.
    pub(crate) parsed: Result<TaskFile, Vec<Issue>>,
}

/// Reading never creates files or runs anything.
pub(crate) fn load(store: &dyn SettingsStore, path: &Path) -> Result<LoadedTasks, String> {
    let text = store.read(path)?;
    let parsed = match &text {
        None => Ok(TaskFile::default()),
        Some(text) => lumen_task_core::parse(text),
    };
    Ok(LoadedTasks { text, parsed })
}

/// How `save_task` treats an ID that already exists in the file.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveMode {
    /// A new task: an existing ID is an error, never a silent replacement.
    Create,
    /// Replace the definition with this ID.
    Update,
}

/// Create or replace one task, writing only if the file still equals
/// `loaded_text`. The result must be valid for this task (dependencies, defaults,
/// duplicates); problems elsewhere in the file do not block the save. Returns the
/// new file text for the next conflict check.
pub(crate) fn save_task(
    store: &dyn SettingsStore,
    path: &Path,
    root: Option<&Path>,
    loaded_text: Option<&str>,
    task: &TaskDef,
    mode: SaveMode,
) -> Result<String, String> {
    if mode == SaveMode::Create
        && let Some(text) = loaded_text
        && lumen_task_core::existing_ids(text)
            .map_err(|error| error.to_string())?
            .contains(&task.id)
    {
        return Err(format!(
            "A task with ID '{}' already exists in this file",
            task.id
        ));
    }
    let updated = match loaded_text {
        None => lumen_task_core::new_file(task),
        Some(text) => lumen_task_core::upsert_task(text, task),
    }
    .map_err(|error| error.to_string())?;
    if let Err(issues) = lumen_task_core::parse(&updated) {
        let own: Vec<String> = issues
            .iter()
            .filter(|issue| issue.task.as_deref() == Some(task.id.as_str()))
            .map(|issue| format!("{}: {}", issue.field, issue.message))
            .collect();
        if !own.is_empty() {
            return Err(own.join("; "));
        }
    }
    store.write(path, root, loaded_text, &updated)?;
    Ok(updated)
}

/// Remove one definition. Deleting never touches files a task produced.
pub(crate) fn delete_task(
    store: &dyn SettingsStore,
    path: &Path,
    root: Option<&Path>,
    loaded_text: &str,
    id: &str,
) -> Result<String, String> {
    let updated =
        lumen_task_core::remove_task(loaded_text, id).map_err(|error| error.to_string())?;
    store.write(path, root, Some(loaded_text), &updated)?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::settings::LocalSettingsStore;
    use lumen_task_core::TaskKind;

    fn task(id: &str) -> TaskDef {
        let mut task = TaskDef::new(id, id, TaskKind::Process);
        task.command = Some("true".into());
        task
    }

    #[test]
    fn paths_follow_the_documented_locations() {
        assert_eq!(
            workspace_path(Path::new("/w")),
            Path::new("/w/.lumen/tasks.toml")
        );
        assert!(user_path().is_none_or(|path| path.ends_with(".lumen/tasks.toml")));
    }

    #[test]
    fn a_missing_file_loads_empty_and_is_not_created() {
        let root = tempfile::tempdir().unwrap();
        let path = workspace_path(root.path());
        let loaded = load(&LocalSettingsStore, &path).unwrap();
        assert!(loaded.text.is_none() && loaded.parsed.unwrap().tasks.is_empty());
        assert!(!root.path().join(".lumen").exists());
    }

    #[test]
    fn create_update_delete_round_trip_and_keep_comments() {
        let root = tempfile::tempdir().unwrap();
        let path = workspace_path(root.path());
        let store = LocalSettingsStore;
        let text = save_task(
            &store,
            &path,
            Some(root.path()),
            None,
            &task("a"),
            SaveMode::Create,
        )
        .unwrap();
        let with_comment = text.replace("schema_version = 1", "# mine\nschema_version = 1");
        std::fs::write(&path, &with_comment).unwrap();
        let loaded = load(&store, &path).unwrap();
        let text = save_task(
            &store,
            &path,
            Some(root.path()),
            loaded.text.as_deref(),
            &task("b"),
            SaveMode::Create,
        )
        .unwrap();
        assert!(text.contains("# mine"));
        assert_eq!(load(&store, &path).unwrap().parsed.unwrap().tasks.len(), 2);
        let text = delete_task(&store, &path, Some(root.path()), &text, "a").unwrap();
        let tasks = load(&store, &path).unwrap().parsed.unwrap().tasks;
        assert_eq!(tasks.len(), 1);
        assert!(text.contains("# mine"));
    }

    #[test]
    fn a_concurrent_edit_is_a_conflict_and_nothing_is_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let path = workspace_path(root.path());
        let store = LocalSettingsStore;
        let text = save_task(
            &store,
            &path,
            Some(root.path()),
            None,
            &task("a"),
            SaveMode::Create,
        )
        .unwrap();
        std::fs::write(&path, format!("{text}# external\n")).unwrap();
        let error = save_task(
            &store,
            &path,
            Some(root.path()),
            Some(&text),
            &task("b"),
            SaveMode::Create,
        )
        .unwrap_err();
        assert!(error.contains("changed on disk"), "{error}");
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# external")
        );
        // Creating a file that appeared meanwhile never clobbers it.
        let error = save_task(
            &store,
            &path,
            Some(root.path()),
            None,
            &task("c"),
            SaveMode::Create,
        )
        .unwrap_err();
        assert!(error.contains("changed on disk"), "{error}");
    }

    #[test]
    fn invalid_definitions_and_unreadable_files_write_nothing() {
        let root = tempfile::tempdir().unwrap();
        let path = workspace_path(root.path());
        let store = LocalSettingsStore;
        let mut bad = task("a");
        bad.command = None;
        assert!(
            save_task(
                &store,
                &path,
                Some(root.path()),
                None,
                &bad,
                SaveMode::Create
            )
            .is_err()
        );
        assert!(!path.exists());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not [ toml").unwrap();
        let loaded = load(&store, &path).unwrap();
        assert!(loaded.parsed.is_err());
        let error = save_task(
            &store,
            &path,
            Some(root.path()),
            loaded.text.as_deref(),
            &task("a"),
            SaveMode::Create,
        )
        .unwrap_err();
        assert!(error.contains("unreadable"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not [ toml");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_directory_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join(".lumen")).unwrap();
        let path = workspace_path(root.path());
        assert!(
            save_task(
                &LocalSettingsStore,
                &path,
                Some(root.path()),
                None,
                &task("a"),
                SaveMode::Create
            )
            .is_err()
        );
        assert!(!outside.path().join("tasks.toml").exists());
    }

    #[test]
    fn creating_never_replaces_and_updates_only_fail_for_their_own_problems() {
        let root = tempfile::tempdir().unwrap();
        let path = workspace_path(root.path());
        let store = LocalSettingsStore;
        let text = save_task(
            &store,
            &path,
            Some(root.path()),
            None,
            &task("a"),
            SaveMode::Create,
        )
        .unwrap();
        let error = save_task(
            &store,
            &path,
            Some(root.path()),
            Some(&text),
            &task("a"),
            SaveMode::Create,
        )
        .unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        // Updating replaces.
        let mut changed = task("a");
        changed.label = "Renamed".into();
        let text = save_task(
            &store,
            &path,
            Some(root.path()),
            Some(&text),
            &changed,
            SaveMode::Update,
        )
        .unwrap();
        assert!(text.contains("Renamed"));
        // A dependency that does not exist is this task's problem and blocks it.
        let mut needy = task("b");
        needy.depends_on = vec!["ghost".into()];
        let error = save_task(
            &store,
            &path,
            Some(root.path()),
            Some(&text),
            &needy,
            SaveMode::Create,
        )
        .unwrap_err();
        assert!(error.contains("unknown task 'ghost'"), "{error}");
        // A problem in another task does not block saving this one.
        let broken = format!(
            "{text}\n[[tasks]]\nid = \"bad\"\nlabel = \"Bad\"\ntype = \"process\"\ncommand = \"x\"\ndepends_on = [\"nope\"]\n"
        );
        std::fs::write(&path, &broken).unwrap();
        let saved = save_task(
            &store,
            &path,
            Some(root.path()),
            Some(&broken),
            &task("c"),
            SaveMode::Create,
        );
        assert!(saved.is_ok(), "{saved:?}");
    }
}
