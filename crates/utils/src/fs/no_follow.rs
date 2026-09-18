use std::fs::OpenOptions;

/// Configures `options` so the open refuses to traverse a symbolic link or reparse point at the
/// final path component.
///
/// Callers that treat a path as hostile until proven otherwise open the object *under that name*
/// rather than whatever it points at, then inspect the handle's metadata. Without this flag a
/// link swapped in between a pre-open check and the open itself would silently redirect the
/// write elsewhere.
pub fn refuse_final_link(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open the reparse point itself rather than its target, so
        // a post-open file-type check sees the link instead of whatever it points at.
        options.custom_flags(0x0020_0000);
    }
    options
}

#[cfg(test)]
mod tests {
    use super::refuse_final_link;
    use std::fs::OpenOptions;
    use tempfile::TempDir;

    /// Opening through a link under the guarded name either fails outright or yields a handle
    /// whose metadata is the link, so no write can reach the target.
    #[cfg(unix)]
    #[test]
    fn a_link_at_the_final_component_never_reaches_its_target() {
        let temp = TempDir::new().expect("temp dir");
        let target = temp.path().join("target.log");
        std::fs::write(&target, "").expect("target");
        let link = temp.path().join("link.log");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        let mut options = OpenOptions::new();
        options.append(true);
        let opened = refuse_final_link(&mut options).open(&link);

        let reached_target = match opened {
            Ok(file) => file.metadata().expect("metadata").file_type().is_file(),
            Err(_) => false,
        };
        assert!(!reached_target, "the link target was reached");
    }

    /// A plain file under the guarded name opens normally, so the guard costs nothing on the
    /// honest path.
    #[test]
    fn a_plain_file_still_opens() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("plain.log");
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        let file = refuse_final_link(&mut options)
            .open(&path)
            .expect("plain file opens");
        assert!(file.metadata().expect("metadata").file_type().is_file());
    }
}
