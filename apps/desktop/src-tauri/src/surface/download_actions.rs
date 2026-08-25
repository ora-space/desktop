//! Host download actions shared by the prompt and automatic dispositions.
//!
//! A webview-plugin download ends in a host-owned action (`import_skill`, `save_as`). The prompt
//! flow runs the action from `surface_resolve_download` after the user picked one; the automatic
//! flow runs it straight from the download pipeline. Both go through [`DownloadActionHost`] so
//! the action implementation exists exactly once and `downloadCompleted` is only emitted after
//! the action really ran.

use ora_backend::{Backend, BackendError};
use ora_contracts::{PrepareSkillImportRequest, SkillImportSource};
use std::path::Path;

/// Executes host-owned download actions against a staged artifact.
///
/// A trait rather than a direct `Backend` dependency so the download pipeline (generic over its
/// gateway for tests) can run automatic actions through a recording fake. Implementations must
/// be safe to call from a blocking task: the pipeline runs them off the webview event thread.
pub trait DownloadActionHost: Send + Sync + 'static {
    /// Hands a landed archive to the two-phase skill import and returns the prepared session id.
    ///
    /// The typed error is preserved so the prompt flow can relay the exact backend condition to
    /// the frontend; the automatic flow reduces it to a display string.
    fn prepare_skill_import(&self, archive: &Path, file_name: &str)
    -> Result<String, BackendError>;
}

impl DownloadActionHost for Backend {
    fn prepare_skill_import(
        &self,
        archive: &Path,
        file_name: &str,
    ) -> Result<String, BackendError> {
        Backend::prepare_skill_import(
            self,
            PrepareSkillImportRequest {
                source: SkillImportSource::Archive {
                    path: archive.to_string_lossy().into_owned(),
                    file_name: file_name.to_owned(),
                },
            },
        )
        .map(|response| response.session.session_id)
    }
}
