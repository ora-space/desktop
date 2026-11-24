//! Asks Git, cheaply, whether one directory lies inside a work tree.
use crate::error::{GitExecError, GitlancerError, ParseError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::output::GitOutput;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use std::path::Path;

/// What Git reports about one directory's place in a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkTreeProbe {
    /// Git discovered a work tree containing the directory (possibly an enclosing checkout).
    InsideWorkTree,
    /// Git discovered a repository but the directory is not in its work tree, such as a bare
    /// repository or a metadata directory.
    OutsideWorkTree,
    /// Git explicitly said the directory is not inside any repository.
    NotInRepository,
}

impl<R: GitRunner> Git<R> {
    /// Probes `directory` with `git rev-parse --is-inside-work-tree`.
    ///
    /// One cheap command that discovers upward exactly like every other Git command run from the
    /// directory. Only Git's explicit "not a git repository" answer becomes `NotInRepository`;
    /// any other failure (an untrusted owner, a broken `.git` link, Git missing) stays an error so
    /// callers never mistake a broken repository for a plain folder.
    ///
    /// `Git::discover_repository` treats any non-zero exit as "not a repository" because it only
    /// decides whether to attempt an operation; the difference is intentional.
    pub fn probe_work_tree(&self, directory: &Path) -> Result<WorkTreeProbe, GitlancerError> {
        classify_work_tree_probe(self.runner().run(&GitCommand::new(
            directory.to_path_buf(),
            vec!["rev-parse".to_string(), "--is-inside-work-tree".to_string()],
            GitEnv::default(),
            GitIntent::ReadOnly,
        )))
    }
}

/// The discovery failure Git prints after walking up from a directory without finding any
/// repository. The parenthesised tail separates it from `not a git repository: <path>`, which
/// Git prints for a broken `.git` link — a repository that is damaged, not a plain folder.
const NOT_IN_REPOSITORY: &str = "not a git repository (or any";

/// Maps the raw `rev-parse --is-inside-work-tree` outcome onto a probe answer.
///
/// Git runs with `LANG=C`; a message localized by a caller-level `LC_ALL` does not match and
/// falls back to an error, which is the safe direction.
fn classify_work_tree_probe(
    outcome: Result<GitOutput, GitExecError>,
) -> Result<WorkTreeProbe, GitlancerError> {
    match outcome {
        Ok(output) => match output.stdout.trim() {
            "true" => Ok(WorkTreeProbe::InsideWorkTree),
            "false" => Ok(WorkTreeProbe::OutsideWorkTree),
            _ => Err(ParseError::InvalidWorkTreeProbe.into()),
        },
        Err(GitExecError::NonZeroExit { ref stderr, .. }) if stderr.contains(NOT_IN_REPOSITORY) => {
            Ok(WorkTreeProbe::NotInRepository)
        }
        Err(other) => Err(GitlancerError::Exec(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkTreeProbe, classify_work_tree_probe};
    use crate::error::{GitExecError, GitlancerError};
    use crate::exec::output::GitOutput;
    use pretty_assertions::assert_eq;

    /// Builds the non-zero exit Git reports with the given stderr.
    fn failed(stderr: &str) -> Result<GitOutput, GitExecError> {
        Err(GitExecError::NonZeroExit {
            code: Some(128),
            args: vec!["rev-parse".to_string(), "--is-inside-work-tree".to_string()],
            stdout: String::new(),
            stderr: stderr.to_string(),
        })
    }

    /// Builds a successful run that printed `stdout`.
    fn succeeded(stdout: &str) -> Result<GitOutput, GitExecError> {
        Ok(GitOutput::new(
            Some(0),
            stdout.to_string(),
            String::new(),
            0,
        ))
    }

    /// Verifies only Git's explicit answers become probe results.
    #[test]
    fn classifies_explicit_answers() {
        let answers = [
            classify_work_tree_probe(succeeded("true\n")).ok(),
            classify_work_tree_probe(succeeded("false\n")).ok(),
            classify_work_tree_probe(failed(
                "fatal: not a git repository (or any of the parent directories): .git\n",
            ))
            .ok(),
            classify_work_tree_probe(failed(
                "fatal: not a git repository (or any parent up to mount point /mnt)\n\
                 Stopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).\n",
            ))
            .ok(),
        ];

        assert_eq!(
            answers,
            [
                Some(WorkTreeProbe::InsideWorkTree),
                Some(WorkTreeProbe::OutsideWorkTree),
                Some(WorkTreeProbe::NotInRepository),
                Some(WorkTreeProbe::NotInRepository),
            ]
        );
    }

    /// Verifies a repository Git refuses for another reason is an error, not a plain folder.
    #[test]
    fn keeps_other_failures_as_errors() {
        let outcomes = [
            classify_work_tree_probe(failed(
                "fatal: detected dubious ownership in repository at '/srv/project'\n",
            )),
            classify_work_tree_probe(failed("fatal: bad config line 1 in file .git/config\n")),
            classify_work_tree_probe(failed("fatal: not a git repository: /gone/worktree\n")),
            classify_work_tree_probe(succeeded("")),
        ];

        assert!(
            outcomes.iter().all(|outcome| matches!(
                outcome,
                Err(GitlancerError::Exec(_) | GitlancerError::Parse(_))
            )),
            "{outcomes:?}"
        );
    }
}
