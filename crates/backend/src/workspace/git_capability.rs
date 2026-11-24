use crate::error::{BackendError, ErrorClassification};
use gitlancer::git::work_tree_probe::WorkTreeProbe;
use gitlancer::{CliGitRunner, Git};
use ora_contracts::{EmptyErrorParams, PublicError, WorkspaceGit};
use ora_logging::{ErrorReport, ora_warn};
use std::path::Path;
use std::time::Duration;

/// How long one probe may take before its answer counts as `Unknown`.
///
/// `rev-parse` normally answers in milliseconds; a Git stuck on something like a stalled network
/// mount must not hold a whole workspace list with it. The limit bounds the Git process and its
/// output; the directory `stat` calls around it are not bounded.
const PROBE_TIME_LIMIT: Duration = Duration::from_secs(5);

/// Asks Git, at this moment, whether commands run from `directory` act on a work tree.
///
/// One `git rev-parse --is-inside-work-tree`, which discovers upward like every other Git
/// command, so a plain folder nested inside another checkout reports `Repository`: the diff
/// reader and writers also operate on that enclosing checkout from there. `NotRepository` needs
/// Git's explicit "not a git repository" answer and no `.git` entry in the folder or above it;
/// any other outcome (an untrusted owner, a damaged repository, Git missing, the time limit) is
/// `Unknown`, so clients never tell the user to `git init` a broken repository. Nothing is cached
/// because a user can `git init` (or delete `.git`) at any time without telling Ora.
pub(crate) fn probe_workspace_git(directory: &Path) -> WorkspaceGit {
    match Git::new(CliGitRunner).probe_work_tree(directory, PROBE_TIME_LIMIT) {
        Ok(WorkTreeProbe::InsideWorkTree) => WorkspaceGit::Repository,
        Ok(WorkTreeProbe::NotInRepository) => WorkspaceGit::NotRepository,
        // A bare repository or metadata directory: not a plain folder, yet nothing to review.
        Ok(WorkTreeProbe::OutsideWorkTree) => WorkspaceGit::Unknown,
        Err(error) => {
            // Reporting `Unknown` keeps clients on their previous behavior instead of claiming
            // the folder is plain.
            let report = ErrorReport::from_error(&error);
            ora_warn!(
                operation = "workspace_git_probe",
                error.message = report.message(),
                error.chain = report.chain(),
                error.chain_depth = report.chain_depth(),
                "failed to probe workspace Git capability",
            );
            WorkspaceGit::Unknown
        }
    }
}

/// Rejects a Git review operation on a directory Git reports as outside any repository.
///
/// Without this guard the reader and writers fail deep inside Git command execution and the
/// client only sees an internal error. An `Unknown` probe lets the operation run, so the real
/// failure and its diagnostics surface exactly as before.
pub(crate) fn require_workspace_git(directory: &Path) -> Result<(), BackendError> {
    match probe_workspace_git(directory) {
        WorkspaceGit::NotRepository => Err(BackendError::new(
            ErrorClassification::Conflict,
            PublicError::WorkspaceNotGitRepository(EmptyErrorParams {}),
            "workspace is not a Git repository",
        )),
        WorkspaceGit::Repository | WorkspaceGit::Unknown => Ok(()),
    }
}
