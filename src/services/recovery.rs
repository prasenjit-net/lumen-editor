//! Durable per-session snapshots. Locked live sessions are never imported.
use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

// Clock resolution varies by host; two stores in one process must never share
// a snapshot name even when SystemTime returns the same instant.
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryDocument {
    pub(crate) path: Option<PathBuf>,
    pub(crate) name: String,
    pub(crate) text: String,
    pub(crate) saved_text: String,
}

#[derive(Default)]
pub(crate) struct RecoveryLoad {
    pub(crate) documents: Vec<RecoveryDocument>,
    pub(crate) warnings: Vec<String>,
}

pub(crate) trait RecoveryStore {
    fn load(&self) -> io::Result<RecoveryLoad>;
    fn save(&self, documents: &[RecoveryDocument]) -> io::Result<()>;
}

struct ImportedSnapshot {
    path: PathBuf,
    lock_path: PathBuf,
    _lock: File,
}

#[derive(Default)]
struct RecoveryState {
    own_lock: Option<File>,
    imported: Vec<ImportedSnapshot>,
}

pub(crate) struct LocalRecoveryStore {
    directory: Option<PathBuf>,
    session: String,
    state: RefCell<RecoveryState>,
}

impl LocalRecoveryStore {
    pub(crate) fn new() -> Self {
        let directory = super::settings::user_path()
            .and_then(|path| path.parent().map(|parent| parent.join("recovery")));
        Self::with_directory(directory)
    }

    fn with_directory(directory: Option<PathBuf>) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            directory,
            session: format!(
                "session-{}-{nonce}-{}",
                std::process::id(),
                NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
            ),
            state: RefCell::new(RecoveryState::default()),
        }
    }

    fn own_paths(&self, directory: &Path) -> (PathBuf, PathBuf) {
        (
            directory.join(format!("{}.bin", self.session)),
            directory.join(format!("{}.lock", self.session)),
        )
    }

    fn ensure_own_lock(&self, directory: &Path, state: &mut RecoveryState) -> io::Result<()> {
        if state.own_lock.is_some() {
            return Ok(());
        }
        fs::create_dir_all(directory)?;
        if fs::symlink_metadata(directory)?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Recovery directory is a symbolic link",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        let (_, lock_path) = self.own_paths(directory);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)?;
        if let Err(error) = lock.try_lock() {
            drop(lock);
            let _ = fs::remove_file(lock_path);
            return Err(error.into());
        }
        state.own_lock = Some(lock);
        Ok(())
    }

    fn import(
        &self,
        path: PathBuf,
        lock_path: PathBuf,
        result: &mut RecoveryLoad,
        state: &mut RecoveryState,
    ) {
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            result.warnings.push(format!(
                "Recovery snapshot {} is a symbolic link; it was not opened",
                path.display()
            ));
            return;
        }
        if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            result.warnings.push(format!(
                "Recovery lock {} is a symbolic link; snapshot was retained",
                lock_path.display()
            ));
            return;
        }
        let lock = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
        {
            Ok(lock) => lock,
            Err(error) => {
                result.warnings.push(format!(
                    "Could not inspect recovery lock {}: {error}",
                    lock_path.display()
                ));
                return;
            }
        };
        match lock.try_lock() {
            Ok(()) => (),
            Err(TryLockError::WouldBlock) => return, // Another process owns this snapshot.
            Err(TryLockError::Error(error)) => {
                result.warnings.push(format!(
                    "Could not lock recovery snapshot {}: {error}",
                    path.display()
                ));
                return;
            }
        }
        match fs::read(&path).and_then(|bytes| decode(&bytes)) {
            Ok(mut documents) => {
                result.documents.append(&mut documents);
                state.imported.push(ImportedSnapshot {
                    path,
                    lock_path,
                    _lock: lock,
                });
            }
            Err(error) => result.warnings.push(format!(
                "Recovery snapshot {} could not be read: {error}; it was retained",
                path.display()
            )),
        }
    }

    fn retire_imports(state: &mut RecoveryState) -> io::Result<()> {
        let mut remaining = Vec::new();
        let mut first_error = None;
        for imported in state.imported.drain(..) {
            match fs::remove_file(&imported.path) {
                Ok(()) => {
                    let lock_path = imported.lock_path.clone();
                    drop(imported); // Windows cannot remove an open lock file.
                    let _ = fs::remove_file(lock_path);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let lock_path = imported.lock_path.clone();
                    drop(imported);
                    let _ = fs::remove_file(lock_path);
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    remaining.push(imported);
                }
            }
        }
        state.imported = remaining;
        first_error.map_or(Ok(()), Err)
    }
}

impl RecoveryStore for LocalRecoveryStore {
    fn load(&self) -> io::Result<RecoveryLoad> {
        let Some(directory) = &self.directory else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "User recovery directory is unavailable",
            ));
        };
        if fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Recovery directory is a symbolic link",
            ));
        }
        let mut result = RecoveryLoad::default();
        let mut state = self.state.borrow_mut();
        let entries = match fs::read_dir(directory) {
            Ok(entries) => Some(entries),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let mut paths = Vec::new();
        for entry in entries.into_iter().flatten() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    result.warnings.push(error.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let Some(stem) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if !stem.starts_with("session-")
                || path.extension().and_then(|ext| ext.to_str()) != Some("bin")
                || stem == self.session
            {
                continue;
            }
            paths.push((path, directory.join(format!("{stem}.lock"))));
        }
        paths.sort_by(|left, right| left.0.cmp(&right.0));
        for (path, lock_path) in paths {
            self.import(path, lock_path, &mut result, &mut state);
        }
        // Import the original single-file format without deleting it until a
        // replacement snapshot has been written successfully.
        let legacy = directory.parent().unwrap_or(directory).join("recovery.bin");
        if legacy.exists() {
            self.import(
                legacy.clone(),
                legacy.with_extension("lock"),
                &mut result,
                &mut state,
            );
        }
        Ok(result)
    }

    fn save(&self, documents: &[RecoveryDocument]) -> io::Result<()> {
        let Some(directory) = &self.directory else {
            return if documents.is_empty() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "User recovery directory is unavailable",
                ))
            };
        };
        let mut state = self.state.borrow_mut();
        let (path, _) = self.own_paths(directory);
        if documents.is_empty() {
            match fs::remove_file(&path) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
            return Self::retire_imports(&mut state);
        }
        self.ensure_own_lock(directory, &mut state)?;
        let mut staging = tempfile::NamedTempFile::new_in(directory)?;
        staging.write_all(&encode(documents))?;
        staging.as_file().sync_all()?;
        staging.persist(path).map_err(|error| error.error)?;
        Self::retire_imports(&mut state)
    }
}

impl Drop for LocalRecoveryStore {
    fn drop(&mut self) {
        let Some(directory) = &self.directory else {
            return;
        };
        let (path, lock_path) = self.own_paths(directory);
        if path.exists() {
            return;
        }
        self.state.get_mut().own_lock.take();
        let _ = fs::remove_file(lock_path);
    }
}

const MAGIC: &[u8] = b"LUMEN-RECOVERY-1\n";
fn encode(documents: &[RecoveryDocument]) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&(documents.len() as u64).to_le_bytes());
    for document in documents {
        for field in [
            document
                .path
                .as_ref()
                .and_then(|path| path.to_str())
                .unwrap_or(""),
            &document.name,
            &document.text,
            &document.saved_text,
        ] {
            bytes.extend_from_slice(&(field.len() as u64).to_le_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
    }
    bytes
}

fn decode(bytes: &[u8]) -> io::Result<Vec<RecoveryDocument>> {
    fn invalid() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Lumen recovery snapshot",
        )
    }
    let Some(mut rest) = bytes.strip_prefix(MAGIC) else {
        return Err(invalid());
    };
    fn take_number(rest: &mut &[u8]) -> io::Result<usize> {
        let number = rest.get(..8).ok_or_else(invalid)?;
        *rest = &rest[8..];
        usize::try_from(u64::from_le_bytes(
            number.try_into().map_err(|_| invalid())?,
        ))
        .map_err(|_| invalid())
    }
    fn take_text(rest: &mut &[u8]) -> io::Result<String> {
        let len = take_number(rest)?;
        let value = rest.get(..len).ok_or_else(invalid)?;
        *rest = &rest[len..];
        String::from_utf8(value.to_vec()).map_err(|_| invalid())
    }
    let count = take_number(&mut rest)?;
    if count > 100_000 {
        return Err(invalid());
    }
    let mut documents = Vec::new();
    for _ in 0..count {
        let path = take_text(&mut rest)?;
        documents.push(RecoveryDocument {
            path: if path.is_empty() {
                None
            } else {
                Some(path.into())
            },
            name: take_text(&mut rest)?,
            text: take_text(&mut rest)?,
            saved_text: take_text(&mut rest)?,
        });
    }
    if !rest.is_empty() {
        return Err(invalid());
    }
    Ok(documents)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(text: &str) -> RecoveryDocument {
        RecoveryDocument {
            path: Some("a.rs".into()),
            name: "a.rs".into(),
            text: text.into(),
            saved_text: "saved".into(),
        }
    }
    #[test]
    fn recovery_child_process_helper() {
        let Some(directory) = std::env::var_os("LUMEN_RECOVERY_CHILD_TEST_DIRECTORY") else {
            return;
        };
        let store = LocalRecoveryStore::with_directory(Some(directory.into()));
        store
            .save(&[
                doc("child disk edit"),
                RecoveryDocument {
                    path: None,
                    name: "untitled-2.txt".into(),
                    text: "child untitled edit".into(),
                    saved_text: String::new(),
                },
            ])
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    #[test]
    fn interrupted_process_backups_are_imported_but_live_ones_are_skipped() {
        use std::{
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("recovery");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services::recovery::tests::recovery_child_process_helper",
            ])
            .env("LUMEN_RECOVERY_CHILD_TEST_DIRECTORY", &directory)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while fs::read_dir(&directory).ok().is_none_or(|entries| {
            entries.filter_map(Result::ok).all(|entry| {
                entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "bin")
            })
        }) {
            if start.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child did not create a recovery snapshot");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let observer = LocalRecoveryStore::with_directory(Some(directory));
        assert!(observer.load().unwrap().documents.is_empty());
        child.kill().unwrap();
        child.wait().unwrap();
        let recovered = observer.load().unwrap().documents;
        assert_eq!(recovered.len(), 2);
        assert!(
            recovered
                .iter()
                .any(|item| item.path.is_none() && item.text == "child untitled edit")
        );
        assert!(
            recovered
                .iter()
                .any(|item| item.path.is_some() && item.text == "child disk edit")
        );
    }
    #[test]
    fn roundtrip_and_reject_truncated_snapshots() {
        let documents = vec![doc("edited\n🦀")];
        let bytes = encode(&documents);
        assert_eq!(decode(&bytes).unwrap(), documents);
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
    }
    #[test]
    fn live_session_is_skipped_then_imported_after_owner_exits() {
        let root = tempfile::tempdir().unwrap();
        let first = LocalRecoveryStore::with_directory(Some(root.path().join("recovery")));
        first.save(&[doc("first")]).unwrap();
        let second = LocalRecoveryStore::with_directory(Some(root.path().join("recovery")));
        assert!(second.load().unwrap().documents.is_empty());
        drop(first); // Simulated interrupted process: snapshot remains, lock releases.
        assert_eq!(second.load().unwrap().documents, vec![doc("first")]);
        second.save(&[doc("second")]).unwrap();
        assert_eq!(
            fs::read_dir(root.path().join("recovery"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "bin"))
                .count(),
            1
        );
    }
    #[test]
    fn corrupt_snapshot_is_retained_and_legacy_file_migrates() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("recovery");
        fs::create_dir(&directory).unwrap();
        let corrupt = directory.join("session-corrupt.bin");
        fs::write(&corrupt, b"bad").unwrap();
        let legacy = root.path().join("recovery.bin");
        fs::write(&legacy, encode(&[doc("legacy")])).unwrap();
        let store = LocalRecoveryStore::with_directory(Some(directory));
        let loaded = store.load().unwrap();
        assert_eq!(loaded.documents, vec![doc("legacy")]);
        assert_eq!(loaded.warnings.len(), 1);
        store.save(&loaded.documents).unwrap();
        assert!(!legacy.exists());
        assert!(corrupt.exists());
    }

    #[test]
    fn failed_new_snapshot_does_not_delete_imported_backup() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("recovery");
        let first = LocalRecoveryStore::with_directory(Some(directory.clone()));
        first.save(&[doc("interrupted")]).unwrap();
        let (old_path, _) = first.own_paths(&directory);
        drop(first);
        let second = LocalRecoveryStore::with_directory(Some(directory.clone()));
        assert_ne!(old_path, second.own_paths(&directory).0);
        let loaded = second.load().unwrap();
        assert_eq!(
            loaded.documents,
            vec![doc("interrupted")],
            "{:?}",
            loaded.warnings
        );
        let (_, lock_path) = second.own_paths(&directory);
        fs::create_dir(&lock_path).unwrap(); // Makes ownership creation fail.
        assert!(second.save(&[doc("new")]).is_err());
        assert!(old_path.exists());
        assert_eq!(
            decode(&fs::read(old_path).unwrap()).unwrap(),
            vec![doc("interrupted")]
        );
    }
    #[cfg(unix)]
    #[test]
    fn backup_directory_and_snapshot_are_private_and_symlink_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("recovery");
        let store = LocalRecoveryStore::with_directory(Some(directory.clone()));
        store.save(&[doc("private")]).unwrap();
        let (snapshot, _) = store.own_paths(&directory);
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(snapshot).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let linked = root.path().join("linked");
        std::os::unix::fs::symlink(&directory, &linked).unwrap();
        let unsafe_store = LocalRecoveryStore::with_directory(Some(linked));
        assert!(unsafe_store.load().is_err());
        assert!(unsafe_store.save(&[doc("unsafe")]).is_err());
    }
}
