//! Reading and decoding image files away from the UI thread.
use crate::model::image_asset::{DecodedImage, decode};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::SystemTime,
};

/// What identifies one version of a file on disk, cheaply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

pub(crate) fn stamp(path: &Path) -> io::Result<FileStamp> {
    let metadata = fs::metadata(path)?;
    Ok(FileStamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

/// Read and decode `path` on a worker thread; the result arrives on the channel.
pub(crate) fn spawn_load(path: PathBuf) -> Receiver<Result<DecodedImage, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| decode(&bytes));
        let _ = sender.send(result);
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn missing_and_corrupt_files_fail_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let missing = spawn_load(dir.path().join("none.png"));
        assert!(
            missing
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .is_err()
        );
        let corrupt = dir.path().join("bad.png");
        fs::write(&corrupt, b"not a png").unwrap();
        let result = spawn_load(corrupt.clone())
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(result.is_err());
        assert!(stamp(&corrupt).is_ok());
        assert!(stamp(&dir.path().join("none.png")).is_err());
    }
}
