use std::path::{Component, Path, PathBuf};

/// Deletes only ordinary descendants proven to remain beneath one pinned trusted root.
#[derive(Debug, Clone)]
pub struct SafeTreeDeleter {
    allowed_root: PathBuf,
}

impl SafeTreeDeleter {
    pub fn new(allowed_root: impl Into<PathBuf>) -> Self {
        Self {
            allowed_root: allowed_root.into(),
        }
    }

    /// Removes one strict descendant without following reparse points or named streams.
    pub fn delete(&self, target: &Path) -> Result<(), SafeDeleteError> {
        match std::fs::symlink_metadata(target) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(SafeDeleteError::Io),
        }
        let relative = target
            .strip_prefix(&self.allowed_root)
            .map_err(|_| SafeDeleteError::OutsideAllowedRoot)?;
        validate_relative(relative)?;
        if relative.as_os_str().is_empty() {
            return Err(SafeDeleteError::RefusedAllowedRoot);
        }
        delete_tree_platform(&self.allowed_root, relative)
    }
}

/// Audits one pinned filesystem object for named streams without following reparse points.
pub fn audit_no_named_streams(path: &Path) -> Result<(), SafeDeleteError> {
    audit_no_named_streams_platform(path)
}

/// Audits one pinned ordinary single-link file including its stream namespace.
pub fn audit_regular_file(path: &Path) -> Result<(), SafeDeleteError> {
    audit_regular_file_platform(path)
}

#[cfg(windows)]
fn audit_no_named_streams_platform(path: &Path) -> Result<(), SafeDeleteError> {
    windows::audit_no_named_streams(path)
}

#[cfg(windows)]
fn audit_regular_file_platform(path: &Path) -> Result<(), SafeDeleteError> {
    windows::audit_regular_file(path)
}

#[cfg(not(windows))]
fn audit_no_named_streams_platform(_path: &Path) -> Result<(), SafeDeleteError> {
    Ok(())
}

#[cfg(not(windows))]
fn audit_regular_file_platform(path: &Path) -> Result<(), SafeDeleteError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| SafeDeleteError::Io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(SafeDeleteError::UnsupportedObject);
    }
    if metadata.nlink() != 1 {
        return Err(SafeDeleteError::MultipleHardLinks);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SafeDeleteError {
    #[error("delete target is outside the allowed root")]
    OutsideAllowedRoot,
    #[error("deleting the allowed root itself is forbidden")]
    RefusedAllowedRoot,
    #[error("delete target contains a non-normal path component")]
    InvalidRelativePath,
    #[error("delete target contains a reparse point or unsupported object")]
    UnsupportedObject,
    #[error("delete target contains a named stream")]
    NamedStream,
    #[error("delete target has multiple hard links")]
    MultipleHardLinks,
    #[error("delete target identity changed during traversal")]
    IdentityChanged,
    #[error("safe delete I/O failed")]
    Io,
}

fn validate_relative(path: &Path) -> Result<(), SafeDeleteError> {
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SafeDeleteError::InvalidRelativePath);
    }
    Ok(())
}

#[cfg(windows)]
fn delete_tree_platform(allowed_root: &Path, relative: &Path) -> Result<(), SafeDeleteError> {
    windows::delete_tree(allowed_root, relative)
}

#[cfg(not(windows))]
fn delete_tree_platform(allowed_root: &Path, relative: &Path) -> Result<(), SafeDeleteError> {
    let root = std::fs::canonicalize(allowed_root).map_err(|_| SafeDeleteError::Io)?;
    let target = allowed_root.join(relative);
    let canonical = std::fs::canonicalize(&target).map_err(|_| SafeDeleteError::Io)?;
    if !canonical.starts_with(&root) {
        return Err(SafeDeleteError::OutsideAllowedRoot);
    }
    delete_no_follow(&target)
}

#[cfg(not(windows))]
fn delete_no_follow(path: &Path) -> Result<(), SafeDeleteError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| SafeDeleteError::Io)?;
    if metadata.file_type().is_symlink() {
        return Err(SafeDeleteError::UnsupportedObject);
    }
    if metadata.is_file() {
        if metadata.nlink() != 1 {
            return Err(SafeDeleteError::MultipleHardLinks);
        }
        return std::fs::remove_file(path).map_err(|_| SafeDeleteError::Io);
    }
    if !metadata.is_dir() {
        return Err(SafeDeleteError::UnsupportedObject);
    }
    for entry in std::fs::read_dir(path).map_err(|_| SafeDeleteError::Io)? {
        delete_no_follow(&entry.map_err(|_| SafeDeleteError::Io)?.path())?;
    }
    std::fs::remove_dir(path).map_err(|_| SafeDeleteError::Io)
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsStr;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::{Path, PathBuf};
    use std::ptr::null;

    use windows_sys::Win32::Foundation::{
        ERROR_HANDLE_EOF, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_DISPOSITION_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfo, FindClose, FindFirstStreamW,
        FindNextStreamW, FindStreamInfoStandard, GetFileInformationByHandle,
        GetFinalPathNameByHandleW, GetVolumeInformationByHandleW, OPEN_EXISTING,
        SetFileInformationByHandle, VOLUME_NAME_DOS, WIN32_FIND_STREAM_DATA,
    };
    use windows_sys::Win32::System::SystemServices::FILE_NAMED_STREAMS;

    use super::SafeDeleteError;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

    pub(super) fn delete_tree(allowed_root: &Path, relative: &Path) -> Result<(), SafeDeleteError> {
        let root = open_pinned(allowed_root)?;
        let root_path = final_path(&root.handle)?;
        if root.information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || root.information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        {
            return Err(SafeDeleteError::UnsupportedObject);
        }
        ensure_no_named_streams(&root)?;
        let target = allowed_root.join(relative);
        let expected = root_path.join(relative);
        delete_entry(&target, &expected)
    }

    fn delete_entry(path: &Path, expected: &Path) -> Result<(), SafeDeleteError> {
        let pinned = open_pinned(path)?;
        if !same_windows_path(&final_path(&pinned.handle)?, expected) {
            return Err(SafeDeleteError::IdentityChanged);
        }
        if pinned.information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(SafeDeleteError::UnsupportedObject);
        }
        ensure_no_named_streams(&pinned)?;
        let is_directory = pinned.information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        if !is_directory && pinned.information.nNumberOfLinks != 1 {
            return Err(SafeDeleteError::MultipleHardLinks);
        }
        if is_directory {
            for entry in std::fs::read_dir(path).map_err(|_| SafeDeleteError::Io)? {
                let entry = entry.map_err(|_| SafeDeleteError::Io)?;
                delete_entry(&entry.path(), &expected.join(entry.file_name()))?;
            }
        }
        revalidate(&pinned, expected)?;
        ensure_no_named_streams(&pinned)?;
        mark_delete_on_close(&pinned.handle)
    }

    pub(super) fn audit_no_named_streams(path: &Path) -> Result<(), SafeDeleteError> {
        let pinned = open_with_access(path, FILE_READ_ATTRIBUTES)?;
        if pinned.information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(SafeDeleteError::UnsupportedObject);
        }
        ensure_no_named_streams(&pinned)
    }

    pub(super) fn audit_regular_file(path: &Path) -> Result<(), SafeDeleteError> {
        let pinned = open_with_access(path, FILE_READ_ATTRIBUTES)?;
        if pinned.information.dwFileAttributes
            & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)
            != 0
        {
            return Err(SafeDeleteError::UnsupportedObject);
        }
        if pinned.information.nNumberOfLinks != 1 {
            return Err(SafeDeleteError::MultipleHardLinks);
        }
        ensure_no_named_streams(&pinned)
    }

    struct PinnedObject {
        handle: OwnedHandle,
        information: BY_HANDLE_FILE_INFORMATION,
    }

    fn open_pinned(path: &Path) -> Result<PinnedObject, SafeDeleteError> {
        open_with_access(path, DELETE | FILE_READ_ATTRIBUTES)
    }

    fn open_with_access(path: &Path, access: u32) -> Result<PinnedObject, SafeDeleteError> {
        let wide = wide_nul(path.as_os_str());
        // SAFETY: the path buffer is NUL-terminated; no security/template pointers are retained.
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE || raw.is_null() {
            return Err(SafeDeleteError::Io);
        }
        // SAFETY: CreateFileW returned one unique live handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let information = object_information(&handle)?;
        Ok(PinnedObject {
            handle,
            information,
        })
    }

    fn object_information(
        handle: &OwnedHandle,
    ) -> Result<BY_HANDLE_FILE_INFORMATION, SafeDeleteError> {
        let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
        // SAFETY: the pinned handle and exact output structure remain live for this call.
        if unsafe { GetFileInformationByHandle(handle.as_raw_handle() as HANDLE, &mut information) }
            == 0
        {
            return Err(SafeDeleteError::Io);
        }
        Ok(information)
    }

    fn final_path(handle: &OwnedHandle) -> Result<PathBuf, SafeDeleteError> {
        let raw = handle.as_raw_handle() as HANDLE;
        // SAFETY: a zero-sized query obtains the exact UTF-16 buffer length.
        let length =
            unsafe { GetFinalPathNameByHandleW(raw, std::ptr::null_mut(), 0, VOLUME_NAME_DOS) };
        if length == 0 {
            return Err(SafeDeleteError::Io);
        }
        let mut buffer = vec![0_u16; length as usize + 1];
        // SAFETY: the buffer is writable and sized from the preceding query.
        let written = unsafe {
            GetFinalPathNameByHandleW(
                raw,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_DOS,
            )
        };
        if written == 0 || written as usize >= buffer.len() {
            return Err(SafeDeleteError::Io);
        }
        buffer.truncate(written as usize);
        Ok(std::ffi::OsString::from_wide(&buffer).into())
    }

    fn ensure_no_named_streams(pinned: &PinnedObject) -> Result<(), SafeDeleteError> {
        let path = final_path(&pinned.handle)?;
        let wide = wide_nul(path.as_os_str());
        let mut data = WIN32_FIND_STREAM_DATA::default();
        // SAFETY: the path and output data are valid for the duration of enumeration.
        let search = unsafe {
            FindFirstStreamW(
                wide.as_ptr(),
                FindStreamInfoStandard,
                (&mut data as *mut WIN32_FIND_STREAM_DATA).cast(),
                0,
            )
        };
        if search == INVALID_HANDLE_VALUE {
            let error = std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or_default() as u32;
            if matches!(error, ERROR_HANDLE_EOF | ERROR_NO_MORE_FILES) {
                return revalidate_identity(pinned);
            }
            if error == ERROR_INVALID_PARAMETER && !volume_supports_named_streams(&pinned.handle)? {
                return revalidate_identity(pinned);
            }
            return Err(SafeDeleteError::Io);
        }
        let search = FindStreamHandle(search);
        loop {
            let length = data
                .cStreamName
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(data.cStreamName.len());
            let name = std::ffi::OsString::from_wide(&data.cStreamName[..length]);
            if name != OsStr::new("::$DATA") {
                return Err(SafeDeleteError::NamedStream);
            }
            // SAFETY: the search handle remains live and data is a writable exact structure.
            if unsafe {
                FindNextStreamW(search.0, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast())
            } == 0
            {
                let error = std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or_default() as u32;
                return if matches!(error, ERROR_HANDLE_EOF | ERROR_NO_MORE_FILES) {
                    revalidate_identity(pinned)
                } else {
                    Err(SafeDeleteError::Io)
                };
            }
        }
    }

    fn volume_supports_named_streams(handle: &OwnedHandle) -> Result<bool, SafeDeleteError> {
        let mut flags = 0u32;
        // SAFETY: optional output buffers are null and the flags pointer is valid for this call.
        let succeeded = unsafe {
            GetVolumeInformationByHandleW(
                handle.as_raw_handle() as HANDLE,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                std::ptr::null_mut(),
                0,
            )
        };
        if succeeded == 0 {
            return Err(SafeDeleteError::Io);
        }
        Ok(flags & FILE_NAMED_STREAMS != 0)
    }

    fn revalidate_identity(pinned: &PinnedObject) -> Result<(), SafeDeleteError> {
        let current = object_information(&pinned.handle)?;
        if !same_identity(&pinned.information, &current)
            || current.dwFileAttributes != pinned.information.dwFileAttributes
            || current.nNumberOfLinks != pinned.information.nNumberOfLinks
        {
            return Err(SafeDeleteError::IdentityChanged);
        }
        Ok(())
    }

    fn revalidate(pinned: &PinnedObject, expected: &Path) -> Result<(), SafeDeleteError> {
        revalidate_identity(pinned)?;
        if !same_windows_path(&final_path(&pinned.handle)?, expected) {
            return Err(SafeDeleteError::IdentityChanged);
        }
        Ok(())
    }

    fn same_identity(
        left: &BY_HANDLE_FILE_INFORMATION,
        right: &BY_HANDLE_FILE_INFORMATION,
    ) -> bool {
        left.dwVolumeSerialNumber == right.dwVolumeSerialNumber
            && left.nFileIndexHigh == right.nFileIndexHigh
            && left.nFileIndexLow == right.nFileIndexLow
    }

    struct FindStreamHandle(HANDLE);

    impl Drop for FindStreamHandle {
        fn drop(&mut self) {
            // SAFETY: this owns one successful FindFirstStreamW search handle.
            unsafe { FindClose(self.0) };
        }
    }

    fn mark_delete_on_close(handle: &OwnedHandle) -> Result<(), SafeDeleteError> {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: the information class, pointer, and byte size exactly match the structure.
        if unsafe {
            SetFileInformationByHandle(
                handle.as_raw_handle() as HANDLE,
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(SafeDeleteError::Io);
        }
        Ok(())
    }

    fn wide_nul(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn same_windows_path(left: &Path, right: &Path) -> bool {
        normalize(left) == normalize(right)
    }

    fn normalize(path: &Path) -> String {
        path.as_os_str()
            .to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    }

    // Keeps the generated layout check explicit when windows-sys changes this structure.
    const _: usize = size_of::<WIN32_FIND_STREAM_DATA>();
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::{SafeDeleteError, SafeTreeDeleter};

    /// Deletes a nested ordinary tree but refuses the authorized root itself.
    #[test]
    fn deletes_only_strict_descendants() {
        let temp = TempDir::new().unwrap_or_else(|error| panic!("safe delete temp: {error}"));
        let root = temp.path().join("data");
        let target = root.join("plugin").join("owner");
        std::fs::create_dir_all(target.join("nested"))
            .unwrap_or_else(|error| panic!("safe delete dirs: {error}"));
        std::fs::write(target.join("nested").join("value.bin"), b"value")
            .unwrap_or_else(|error| panic!("safe delete file: {error}"));
        let deleter = SafeTreeDeleter::new(&root);
        deleter
            .delete(&target)
            .unwrap_or_else(|error| panic!("safe delete target: {error}"));
        assert!(!target.exists());
        assert_eq!(
            deleter.delete(&root),
            Err(SafeDeleteError::RefusedAllowedRoot)
        );
    }
}
