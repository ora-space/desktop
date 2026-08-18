use std::{fmt, str::FromStr};
use thiserror::Error;

/// Holds a branch name accepted by Git's branch-name rules without invoking Git.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GitBranchName(String);

impl GitBranchName {
    /// Parses one short branch name and rejects full refs and checkout-history expressions.
    pub fn parse(value: &str) -> Result<Self, GitBranchNameError> {
        validate_branch_name(value)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated short branch name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for GitBranchName {
    /// Borrows the validated branch name.
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for GitBranchName {
    /// Writes the validated branch name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for GitBranchName {
    type Err = GitBranchNameError;

    /// Parses a branch name through the invariant-preserving constructor.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Describes which Git branch-name rule rejected an input.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GitBranchNameError {
    #[error("branch name must not be empty")]
    Empty,
    #[error("branch name must be short rather than a refs path")]
    FullRef,
    #[error("branch name must not be a checkout-history expression")]
    PreviousCheckoutExpression,
    #[error("branch name must not start with a hyphen")]
    LeadingHyphen,
    #[error("branch name must not start with a slash")]
    LeadingSlash,
    #[error("branch name must not end with a slash")]
    TrailingSlash,
    #[error("branch name must not contain consecutive slashes")]
    ConsecutiveSlashes,
    #[error("branch name components must not start with a dot")]
    DotPrefixedComponent,
    #[error("branch name components must not end with .lock")]
    LockSuffix,
    #[error("branch name must not end with a dot")]
    TrailingDot,
    #[error("branch name must not contain two consecutive dots")]
    ConsecutiveDots,
    #[error("branch name must not contain reflog syntax")]
    ReflogSyntax,
    #[error("branch name must not be a single at sign")]
    SingleAt,
    #[error("branch name must not contain spaces")]
    Space,
    #[error("branch name must not contain control characters")]
    ControlCharacter,
    #[error("branch name contains forbidden character {character:?}")]
    ForbiddenCharacter { character: char },
}

/// Applies the documented `git check-ref-format --branch` rules in deterministic order.
fn validate_branch_name(value: &str) -> Result<(), GitBranchNameError> {
    if value.is_empty() {
        return Err(GitBranchNameError::Empty);
    }
    if value.starts_with("refs/") {
        return Err(GitBranchNameError::FullRef);
    }
    if is_previous_checkout_expression(value) {
        return Err(GitBranchNameError::PreviousCheckoutExpression);
    }
    if value.starts_with('-') {
        return Err(GitBranchNameError::LeadingHyphen);
    }
    if value == "@" {
        return Err(GitBranchNameError::SingleAt);
    }
    if value.starts_with('/') {
        return Err(GitBranchNameError::LeadingSlash);
    }
    if value.ends_with('/') {
        return Err(GitBranchNameError::TrailingSlash);
    }
    if value.contains("//") {
        return Err(GitBranchNameError::ConsecutiveSlashes);
    }
    if value.contains("..") {
        return Err(GitBranchNameError::ConsecutiveDots);
    }
    if value.contains("@{") {
        return Err(GitBranchNameError::ReflogSyntax);
    }
    if value.ends_with('.') {
        return Err(GitBranchNameError::TrailingDot);
    }

    for component in value.split('/') {
        if component.starts_with('.') {
            return Err(GitBranchNameError::DotPrefixedComponent);
        }
        if component.ends_with(".lock") {
            return Err(GitBranchNameError::LockSuffix);
        }
    }

    for character in value.chars() {
        if character.is_control() {
            return Err(GitBranchNameError::ControlCharacter);
        }
        if character == ' ' {
            return Err(GitBranchNameError::Space);
        }
        if matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\') {
            return Err(GitBranchNameError::ForbiddenCharacter { character });
        }
    }

    Ok(())
}

/// Recognizes Git's special `@{-n}` previous-checkout spelling before ref validation.
fn is_previous_checkout_expression(value: &str) -> bool {
    value
        .strip_prefix("@{-")
        .and_then(|suffix| suffix.strip_suffix('}'))
        .is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::{GitBranchName, GitBranchNameError};
    use pretty_assertions::assert_eq;

    /// Verifies ordinary and nested short branch names retain their original spelling.
    #[test]
    fn parses_valid_branch_names() {
        let cases = ["main", "feature/weather-api", "release/v1.2", "修复/天气"];

        for input in cases {
            let Ok(branch) = GitBranchName::parse(input) else {
                panic!("expected {input:?} to be a valid branch name");
            };
            assert_eq!(branch.as_str(), input);
        }
    }

    /// Verifies the structural Git ref rules report stable error categories.
    #[test]
    fn rejects_invalid_branch_structure() {
        let cases = [
            ("", GitBranchNameError::Empty),
            ("refs/heads/main", GitBranchNameError::FullRef),
            ("refs/tags/v1", GitBranchNameError::FullRef),
            ("refs/remotes/origin/main", GitBranchNameError::FullRef),
            ("refs/pull/375/head", GitBranchNameError::FullRef),
            ("@{-1}", GitBranchNameError::PreviousCheckoutExpression),
            ("-feature", GitBranchNameError::LeadingHyphen),
            ("/feature", GitBranchNameError::LeadingSlash),
            ("feature/", GitBranchNameError::TrailingSlash),
            ("feature//api", GitBranchNameError::ConsecutiveSlashes),
            ("feature/../api", GitBranchNameError::ConsecutiveDots),
            ("feature/@{api", GitBranchNameError::ReflogSyntax),
            ("feature.", GitBranchNameError::TrailingDot),
            (".feature/api", GitBranchNameError::DotPrefixedComponent),
            ("feature/api.lock", GitBranchNameError::LockSuffix),
            ("@", GitBranchNameError::SingleAt),
        ];

        for (input, expected) in cases {
            assert_eq!(GitBranchName::parse(input), Err(expected), "{input}");
        }
    }

    /// Verifies every forbidden character family is rejected without platform dependence.
    #[test]
    fn rejects_invalid_branch_characters() {
        let cases = [
            ("feature api", GitBranchNameError::Space),
            ("feature\napi", GitBranchNameError::ControlCharacter),
            (
                "feature~api",
                GitBranchNameError::ForbiddenCharacter { character: '~' },
            ),
            (
                "feature^api",
                GitBranchNameError::ForbiddenCharacter { character: '^' },
            ),
            (
                "feature:api",
                GitBranchNameError::ForbiddenCharacter { character: ':' },
            ),
            (
                "feature?api",
                GitBranchNameError::ForbiddenCharacter { character: '?' },
            ),
            (
                "feature*api",
                GitBranchNameError::ForbiddenCharacter { character: '*' },
            ),
            (
                "feature[api",
                GitBranchNameError::ForbiddenCharacter { character: '[' },
            ),
            (
                "feature\\api",
                GitBranchNameError::ForbiddenCharacter { character: '\\' },
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(GitBranchName::parse(input), Err(expected), "{input}");
        }
    }
}
