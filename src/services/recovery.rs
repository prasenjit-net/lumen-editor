//! Crash-recovery snapshots, separate from project files and editor history.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryDocument {
    pub(crate) path: Option<PathBuf>,
    pub(crate) name: String,
    pub(crate) text: String,
    pub(crate) saved_text: String,
}

pub(crate) trait RecoveryStore {
    fn load(&self) -> io::Result<Vec<RecoveryDocument>>;
    fn save(&self, documents: &[RecoveryDocument]) -> io::Result<()>;
}

pub(crate) struct LocalRecoveryStore {
    path: Option<PathBuf>,
}

impl LocalRecoveryStore {
    pub(crate) fn new() -> Self {
        Self {
            path: super::settings::user_path()
                .and_then(|path| path.parent().map(|dir| dir.join("recovery.bin"))),
        }
    }
}

impl RecoveryStore for LocalRecoveryStore {
    fn load(&self) -> io::Result<Vec<RecoveryDocument>> {
        let Some(path) = &self.path else {
            return Ok(Vec::new());
        };
        match fs::read(path) {
            Ok(bytes) => decode(&bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    fn save(&self, documents: &[RecoveryDocument]) -> io::Result<()> {
        let Some(path) = &self.path else {
            if documents.is_empty() {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "User recovery directory is unavailable",
            ));
        };
        if documents.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
            return Ok(());
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut staging = tempfile::NamedTempFile::new_in(parent)?;
        staging.write_all(&encode(documents))?;
        staging.as_file().sync_all()?;
        staging.persist(path).map_err(|error| error.error)?;
        Ok(())
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
    #[test]
    fn roundtrip_and_reject_truncated_snapshots() {
        let documents = vec![RecoveryDocument {
            path: Some("a.rs".into()),
            name: "a.rs".into(),
            text: "edited\n🦀".into(),
            saved_text: "saved".into(),
        }];
        let bytes = encode(&documents);
        assert_eq!(decode(&bytes).unwrap(), documents);
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
    }
}
