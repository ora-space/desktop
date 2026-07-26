use gitlancer::{CliGitRunner, Git, GlobalIdentity};
use ora_contracts::GitIdentityResponse;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;

/// The subset of `gh api user` consumed to fill a missing git identity.
#[derive(Debug, Deserialize)]
struct GhUser {
    login: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

/// Resolves the sidebar identity: global git config first, the GitHub CLI as a fallback.
///
/// GitHub-CLI users frequently authenticate with `gh` without ever setting
/// `git config --global user.*`, which would otherwise leave the profile blank. When
/// git has no name to show we ask `gh api user` for the authenticated account before
/// giving up. The GitHub lookup is skipped entirely once git already provides a name,
/// so the common path never pays for the network call.
pub fn resolve_git_identity() -> GitIdentityResponse {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let git = Git::new(CliGitRunner)
        .read_global_identity(&cwd)
        .unwrap_or_default();

    let gh = if has_text(git.name.as_deref()) {
        None
    } else {
        read_gh_user()
    };

    combine_identity(git, gh)
}

/// Merges the git and GitHub identities by precedence: git wins, then GitHub's
/// display name (or login), then GitHub's public email.
fn combine_identity(git: GlobalIdentity, gh: Option<GhUser>) -> GitIdentityResponse {
    let gh_name = gh
        .as_ref()
        .map(|user| clean(user.name.as_deref()).unwrap_or_else(|| user.login.trim().to_string()));
    let gh_email = gh.as_ref().and_then(|user| clean(user.email.as_deref()));

    GitIdentityResponse {
        name: clean(git.name.as_deref()).or(gh_name),
        email: clean(git.email.as_deref()).or(gh_email),
    }
}

/// Reads the authenticated GitHub account via the `gh` CLI, or `None` when it is
/// unavailable, unauthenticated, or offline.
fn read_gh_user() -> Option<GhUser> {
    let output = Command::new("gh").args(["api", "user"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice::<GhUser>(&output.stdout).ok()
}

/// Returns the trimmed value when it carries visible text, otherwise `None`.
fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Reports whether an optional field carries visible text.
fn has_text(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{GhUser, combine_identity};
    use gitlancer::GlobalIdentity;
    use ora_contracts::GitIdentityResponse;
    use pretty_assertions::assert_eq;

    fn gh(login: &str, name: Option<&str>, email: Option<&str>) -> GhUser {
        GhUser {
            login: login.to_string(),
            name: name.map(str::to_string),
            email: email.map(str::to_string),
        }
    }

    /// Verifies a configured git identity is used verbatim and the GitHub account is ignored.
    #[test]
    fn prefers_git_identity_over_github() {
        let git = GlobalIdentity {
            name: Some("RuihaoZhang".to_string()),
            email: Some("r9644360@gmail.com".to_string()),
        };

        assert_eq!(
            combine_identity(
                git,
                Some(gh("ObsisMc", Some("Ray"), Some("ray@github.com")))
            ),
            GitIdentityResponse {
                name: Some("RuihaoZhang".to_string()),
                email: Some("r9644360@gmail.com".to_string()),
            }
        );
    }

    /// Verifies a gh-only user (no git config) surfaces the GitHub name and email.
    #[test]
    fn falls_back_to_github_name_and_email() {
        assert_eq!(
            combine_identity(
                GlobalIdentity::default(),
                Some(gh("ObsisMc", Some("Ruihao Zhang"), Some("ray@github.com"))),
            ),
            GitIdentityResponse {
                name: Some("Ruihao Zhang".to_string()),
                email: Some("ray@github.com".to_string()),
            }
        );
    }

    /// Verifies the GitHub login stands in when the account has no display name,
    /// and a private (null) email stays empty.
    #[test]
    fn falls_back_to_github_login_with_private_email() {
        assert_eq!(
            combine_identity(GlobalIdentity::default(), Some(gh("ObsisMc", None, None))),
            GitIdentityResponse {
                name: Some("ObsisMc".to_string()),
                email: None,
            }
        );
    }

    /// Verifies a missing git email is filled from GitHub while the git name is kept.
    #[test]
    fn keeps_git_name_and_borrows_github_email() {
        let git = GlobalIdentity {
            name: Some("RuihaoZhang".to_string()),
            email: None,
        };

        assert_eq!(
            combine_identity(
                git,
                Some(gh("ObsisMc", Some("Ray"), Some("ray@github.com")))
            ),
            GitIdentityResponse {
                name: Some("RuihaoZhang".to_string()),
                email: Some("ray@github.com".to_string()),
            }
        );
    }

    /// Verifies both fields stay empty when neither git nor GitHub yields an identity.
    #[test]
    fn yields_empty_identity_without_any_source() {
        assert_eq!(
            combine_identity(GlobalIdentity::default(), None),
            GitIdentityResponse {
                name: None,
                email: None,
            }
        );
    }
}
