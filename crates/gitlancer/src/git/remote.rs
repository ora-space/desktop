use crate::domain::repo::Repository;
use crate::error::{GitExecError, GitlancerError, ParseError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

/// Carries the repository whose remote-tracking refs should be refreshed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchAllRequest<'a> {
    pub repository: &'a Repository,
}

/// Carries the repository whose current branch tracking relationship should be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadTrackingStatusRequest<'a> {
    pub repository: &'a Repository,
}

/// Returns the configured upstream and commit distance for the current branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadTrackingStatusResponse {
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

impl<R: GitRunner> Git<R> {
    /// Fetches every configured remote and prunes stale tracking refs without touching a worktree.
    pub fn fetch_all(&self, request: FetchAllRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&build_fetch_all_command(&request))?;
        Ok(())
    }

    /// Reads the current branch's upstream and ahead/behind counts from Git plumbing output.
    pub fn read_tracking_status(
        &self,
        request: ReadTrackingStatusRequest<'_>,
    ) -> Result<ReadTrackingStatusResponse, GitlancerError> {
        let upstream_output = self.runner().run(&GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "rev-parse".to_string(),
                "--abbrev-ref".to_string(),
                "--symbolic-full-name".to_string(),
                "@{upstream}".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ));
        let upstream = match upstream_output {
            Ok(output) => {
                let value = output.stdout.trim();
                (!value.is_empty()).then(|| value.to_string())
            }
            Err(GitExecError::NonZeroExit { .. }) => None,
            Err(error) => return Err(error.into()),
        };

        let Some(upstream) = upstream else {
            return Ok(ReadTrackingStatusResponse {
                upstream: None,
                ahead: 0,
                behind: 0,
            });
        };

        let output = self.runner().run(&GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "rev-list".to_string(),
                "--left-right".to_string(),
                "--count".to_string(),
                format!("HEAD...{upstream}"),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        let (ahead, behind) = parse_tracking_counts(&output.stdout)?;

        Ok(ReadTrackingStatusResponse {
            upstream: Some(upstream),
            ahead,
            behind,
        })
    }
}

/// Builds the non-destructive all-remote fetch command used by repository synchronization.
pub fn build_fetch_all_command(request: &FetchAllRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "fetch".to_string(),
            "--all".to_string(),
            "--prune".to_string(),
        ],
        GitEnv::default().with_variable("GIT_TERMINAL_PROMPT", "0"),
        GitIntent::Network,
    )
}

/// Parses Git's left/right count where the left side is ahead and the right side is behind.
fn parse_tracking_counts(stdout: &str) -> Result<(u32, u32), GitlancerError> {
    let mut fields = stdout.split_whitespace();
    let ahead = fields
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(ParseError::InvalidRemoteTrackingStatus)?;
    let behind = fields
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(ParseError::InvalidRemoteTrackingStatus)?;

    if fields.next().is_some() {
        return Err(ParseError::InvalidRemoteTrackingStatus.into());
    }

    Ok((ahead, behind))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{FetchAllRequest, build_fetch_all_command, parse_tracking_counts};
    use crate::domain::paths::RepoRoot;
    use crate::domain::repo::Repository;
    use crate::error::ParseError;

    /// Verifies fetch always prunes every remote with prompts disabled for automation.
    #[test]
    fn builds_all_remote_fetch_command() {
        let repository = Repository::new(RepoRoot::new("D:/gitlancer-remote-tests"));
        let command = build_fetch_all_command(&FetchAllRequest {
            repository: &repository,
        });

        assert_eq!(command.args, vec!["fetch", "--all", "--prune"]);
        assert_eq!(command.intent, crate::GitIntent::Network);
        assert_eq!(
            command.env.variables.get("GIT_TERMINAL_PROMPT"),
            Some(&"0".to_string())
        );
    }

    /// Verifies Git's two tracking counts map to ahead and behind in the public order.
    #[test]
    fn parses_tracking_counts() {
        assert_eq!(
            parse_tracking_counts("2\t3\n").expect("parse tracking counts"),
            (2, 3)
        );
        assert!(matches!(
            parse_tracking_counts("invalid"),
            Err(crate::GitlancerError::Parse(
                ParseError::InvalidRemoteTrackingStatus
            ))
        ));
    }
}
