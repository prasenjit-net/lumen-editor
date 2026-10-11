use std::{fs, io, path::Path};

/// Use exclusive native renames, not a check-then-overwrite operation.
pub(super) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let from = CString::new(from.as_os_str().as_bytes())?;
        let to = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both paths are live, NUL-terminated strings; native flags
        // forbid replacing a destination, including a racing external create.
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return Err(io::ErrorKind::Unsupported.into());
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        return if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        };
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: paths are valid, NUL-terminated UTF-16 buffers. MoveFileW
        // fails when the destination exists, rather than replacing its contents.
        let result = unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileW(from.as_ptr(), to.as_ptr())
        };
        if result != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

/// Copy bytes and directories without overwriting any destination. Do not
/// follow symlinks: doing so can escape the selected tree or cause recursion.
pub(super) fn copy_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(from)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Copying symbolic links is not supported.",
        ));
    }
    if metadata.is_dir() {
        let source_directory = fs::canonicalize(from)?;
        let target_parent = fs::canonicalize(to.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
        if to.starts_with(from) || target_parent.starts_with(&source_directory) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cannot copy a folder into itself.",
            ));
        }
        fs::create_dir(to)?;
        for child in fs::read_dir(from)? {
            let child = child?;
            copy_no_replace(&child.path(), &to.join(child.file_name()))?;
        }
        Ok(())
    } else if metadata.is_file() {
        let mut source = fs::File::open(from)?;
        let mut destination = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        io::copy(&mut source, &mut destination)?;
        fs::set_permissions(to, metadata.permissions())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Only ordinary files and folders can be copied.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory() -> std::path::PathBuf {
        // Parallel tests can observe the same clock tick on macOS.
        static NEXT_DIRECTORY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "lumen-explorer-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_DIRECTORY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn native_rename_and_copy_never_replace_an_existing_destination() {
        let directory = temporary_directory();
        let source = directory.join("source.txt");
        let occupied = directory.join("occupied.txt");
        let renamed = directory.join("renamed.txt");
        let copied = directory.join("copied.txt");
        fs::write(&source, "source bytes").unwrap();
        fs::write(&occupied, "destination bytes").unwrap();
        assert!(rename_no_replace(&source, &occupied).is_err());
        assert!(copy_no_replace(&source, &occupied).is_err());
        assert_eq!(fs::read_to_string(&occupied).unwrap(), "destination bytes");
        assert_eq!(fs::read_to_string(&source).unwrap(), "source bytes");
        copy_no_replace(&source, &copied).unwrap();
        rename_no_replace(&source, &renamed).unwrap();
        assert_eq!(fs::read_to_string(&copied).unwrap(), "source bytes");
        assert_eq!(fs::read_to_string(&renamed).unwrap(), "source bytes");
        assert!(!source.exists());
        for path in [occupied, renamed, copied] {
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn directory_copy_preserves_contents_and_rejects_self_recursion() {
        let directory = temporary_directory();
        let source = directory.join("source");
        let copied = directory.join("copy");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("data.bin"), [0, 255, 17, 64]).unwrap();
        assert!(copy_no_replace(&source, &source.join("child")).is_err());
        assert!(!source.join("child").exists());
        copy_no_replace(&source, &copied).unwrap();
        assert_eq!(fs::read(copied.join("data.bin")).unwrap(), [0, 255, 17, 64]);
        for folder in [source, copied] {
            fs::remove_file(folder.join("data.bin")).unwrap();
            fs::remove_dir(folder).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }
}
