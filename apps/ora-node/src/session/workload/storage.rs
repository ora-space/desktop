//! Deployment validation precedes view deletion or session admission.
use std::{fs, io, os::unix::fs::MetadataExt, path::Path};

/// Confirms both device identity and actual hard-link support, without copying package bytes.
pub(crate) fn validate(home: &Path, workload: &Path) -> io::Result<()> {
    let installed = ora_plugin_manager::installed_root(home);
    ora_utils::path::create_directories_without_symlinks(&installed)?;
    ora_utils::fs::require_local_filesystem(workload)?;
    let device = fs::metadata(home)?.dev();
    if fs::metadata(workload)?.dev() != device || fs::metadata(&installed)?.dev() != device {
        return Err(io::Error::other(
            "Node home, installed packages and workload views must share a filesystem",
        ));
    }
    let name = format!(".node-link-check-{}", uuid::Uuid::new_v4());
    let source = installed.join(&name);
    let destination = workload.join(&name);
    let result = (|| {
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&source)?;
        fs::hard_link(&source, &destination)
    })();
    let _ = fs::remove_file(destination);
    let _ = fs::remove_file(source);
    result
}
