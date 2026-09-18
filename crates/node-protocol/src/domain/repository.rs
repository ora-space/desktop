use crate::{BranchName, MessageValidationError, NodeId};
use ora_utils::GitBranchName;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;
use url::Url;

/// A non-secret HTTPS or explicit SSH source, preserving exact spelling for deduplication.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct CloneRepositoryUrl(String);

/// Deliberately excludes the rejected address so diagnostics cannot echo credentials.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(
    "clone source must be an explicit HTTPS or SSH repository URL without credentials, query or fragment"
)]
pub struct InvalidCloneRepositoryUrl;

impl CloneRepositoryUrl {
    /// Checks transport policy without normalizing distinct execution inputs together.
    pub fn parse(value: &str) -> Result<Self, InvalidCloneRepositoryUrl> {
        // URL parsers can repair whitespace, backslashes and missing authority delimiters.
        // Git receives the original text, so do not accept those alternative interpretations.
        if value.chars().any(|c| c.is_whitespace() || c.is_control()) || value.contains('\\') {
            return Err(InvalidCloneRepositoryUrl);
        }
        let (scheme, rest) = value.split_once("://").ok_or(InvalidCloneRepositoryUrl)?;
        if !matches!(scheme, "https" | "ssh") || rest.starts_with('/') {
            return Err(InvalidCloneRepositoryUrl);
        }
        let parsed = Url::parse(value).map_err(|_| InvalidCloneRepositoryUrl)?;
        let authority = rest.split('/').next().ok_or(InvalidCloneRepositoryUrl)?;
        if parsed.host_str().is_none_or(str::is_empty)
            || parsed.path().is_empty()
            || parsed.path() == "/"
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || (scheme == "https" && authority.contains('@'))
            || authority
                .rsplit_once('@')
                .is_some_and(|(userinfo, _)| userinfo.is_empty() || userinfo.contains(':'))
        {
            return Err(InvalidCloneRepositoryUrl);
        }
        Ok(Self(value.to_owned()))
    }

    /// Exposes the validated source only to explicit persistence and execution consumers.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CloneRepositoryUrl {
    /// Prevents enclosing message diagnostics from disclosing full repository addresses.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CloneRepositoryUrl([redacted])")
    }
}

impl TryFrom<String> for CloneRepositoryUrl {
    type Error = InvalidCloneRepositoryUrl;

    /// Keeps deserialization subject to the same rules as locally constructed requests.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<CloneRepositoryUrl> for String {
    /// Retains original source spelling in the wire and durable input representation.
    fn from(value: CloneRepositoryUrl) -> Self {
        value.0
    }
}

/// Clone intent has no Main Workspace, destination, credentials or caller-selected Git flags.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CloneExecutionSpec {
    pub node_id: NodeId,
    pub repository: CloneRepositoryUrl,
    pub branch: BranchName,
}

impl CloneExecutionSpec {
    /// Validates a literal source branch; remote existence must still be checked by execution.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.node_id.is_empty() {
            return Err(MessageValidationError::EmptyField { field: "node_id" });
        }
        if self.branch.as_str() == "HEAD" || GitBranchName::parse(self.branch.as_str()).is_err() {
            return Err(MessageValidationError::InvalidCloneBranch);
        }
        Ok(())
    }
}
