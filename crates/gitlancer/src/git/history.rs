use crate::domain::refs::{BranchName, CommitId};
use crate::domain::repo::Repository;
use crate::error::{GitExecError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::parse;

const COMMIT_RECORD_FORMAT: &str = "%H%x00%P%x00%an%x00%ae%x00%aI%x00%s%x1e";
const REFERENCE_RECORD_FORMAT: &str = "%(refname)\t%(objectname)";
const HISTORY_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const COMMIT_DETAIL_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

/// Carries the repository and bounded history size needed for a graph query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListCommitsRequest<'a> {
    pub repository: &'a Repository,
    pub limit: usize,
}

/// Returns the commits needed to draw a repository graph in newest-first order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListCommitsResponse {
    pub commits: Vec<CommitSummary>,
}

/// Carries the repository whose refs should be read for graph labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListReferencesRequest<'a> {
    pub repository: &'a Repository,
}

/// Returns local, remote-tracking, and tag refs together with their target commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListReferencesResponse {
    pub references: Vec<RepositoryReference>,
}

/// Carries one repository and the commit object whose metadata and changed paths are requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetCommitRequest<'a> {
    pub repository: &'a Repository,
    pub commit_id: &'a CommitId,
}

/// Returns the selected commit together with its changed paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetCommitResponse {
    pub commit: CommitDetails,
}

/// Describes one commit using fields that are stable across Git clients and graph renderers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    pub id: CommitId,
    pub parents: Vec<CommitId>,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
}

/// Adds changed file metadata to a commit summary for the detail pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetails {
    pub summary: CommitSummary,
    pub files: Vec<CommitFile>,
}

/// Describes one path changed by a commit using Git's name-status code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    pub status: String,
    pub path: String,
}

/// Identifies what kind of ref owns a graph label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    Local,
    Remote,
    Tag,
}

/// Associates a visible ref name with the commit it decorates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryReference {
    pub name: String,
    pub commit_id: CommitId,
    pub kind: ReferenceKind,
}

/// Returns the symbolic branch and commit currently checked out by a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryHead {
    pub branch_name: Option<BranchName>,
    pub commit_id: Option<CommitId>,
}

impl<R: GitRunner> Git<R> {
    /// Lists all reachable commits through machine-readable output bounded for UI history use.
    pub fn list_commits(
        &self,
        request: ListCommitsRequest<'_>,
    ) -> Result<ListCommitsResponse, GitlancerError> {
        if request.limit == 0 {
            return Ok(ListCommitsResponse {
                commits: Vec::new(),
            });
        }

        let limit = request.limit;
        let command = GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "log".to_string(),
                "--all".to_string(),
                "--date-order".to_string(),
                "--no-color".to_string(),
                format!("--format={COMMIT_RECORD_FORMAT}"),
                format!("-n{limit}"),
                "HEAD".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output =
            self.runner()
                .run_bounded(&command, HISTORY_OUTPUT_LIMIT, HISTORY_OUTPUT_LIMIT)?;

        Ok(ListCommitsResponse {
            commits: parse::history::parse_commit_history(&output.stdout)?,
        })
    }

    /// Lists refs from Git's ref database so graph labels never depend on human-oriented output.
    pub fn list_references(
        &self,
        request: ListReferencesRequest<'_>,
    ) -> Result<ListReferencesResponse, GitlancerError> {
        let command = GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "for-each-ref".to_string(),
                format!("--format={REFERENCE_RECORD_FORMAT}"),
                "refs/heads".to_string(),
                "refs/remotes".to_string(),
                "refs/tags".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output =
            self.runner()
                .run_bounded(&command, HISTORY_OUTPUT_LIMIT, HISTORY_OUTPUT_LIMIT)?;

        Ok(ListReferencesResponse {
            references: parse::history::parse_references(&output.stdout)?,
        })
    }

    /// Loads one commit's metadata and changed paths without allowing the id to become an option.
    pub fn get_commit(
        &self,
        request: GetCommitRequest<'_>,
    ) -> Result<GetCommitResponse, GitlancerError> {
        let command = GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "show".to_string(),
                "--root".to_string(),
                "--no-color".to_string(),
                format!("--format={COMMIT_RECORD_FORMAT}"),
                "--name-status".to_string(),
                "--find-renames".to_string(),
                "--end-of-options".to_string(),
                request.commit_id.as_str().to_string(),
                "--".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output = self.runner().run_bounded(
            &command,
            COMMIT_DETAIL_OUTPUT_LIMIT,
            HISTORY_OUTPUT_LIMIT,
        )?;

        Ok(GetCommitResponse {
            commit: parse::history::parse_commit_details(&output.stdout)?,
        })
    }

    /// Reads the symbolic branch and commit currently checked out by the repository.
    pub fn read_head(&self, repository: &Repository) -> Result<RepositoryHead, GitlancerError> {
        let branch_name = self.read_optional_value(
            repository,
            vec![
                "symbolic-ref".to_string(),
                "--quiet".to_string(),
                "--short".to_string(),
                "HEAD".to_string(),
            ],
        )?;
        let commit_id = self.read_optional_value(
            repository,
            vec![
                "rev-parse".to_string(),
                "--verify".to_string(),
                "HEAD".to_string(),
            ],
        )?;

        Ok(RepositoryHead {
            branch_name: branch_name.map(BranchName::new),
            commit_id: commit_id.map(CommitId::new),
        })
    }

    /// Treats Git's expected detached-or-empty-HEAD exit as an optional value while preserving real failures.
    fn read_optional_value(
        &self,
        repository: &Repository,
        args: Vec<String>,
    ) -> Result<Option<String>, GitlancerError> {
        let command = GitCommand::new(
            repository.root().as_path().to_path_buf(),
            args,
            GitEnv::default(),
            GitIntent::ReadOnly,
        );

        match self.runner().run(&command) {
            Ok(output) => {
                let value = output.stdout.trim();
                Ok((!value.is_empty()).then(|| value.to_string()))
            }
            Err(GitExecError::NonZeroExit { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}
