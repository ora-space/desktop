//! Addresses and refs on the platform Git Server that execution inputs name.
//!
//! Every value is validated when it is constructed or decoded, so a Node never holds an address
//! outside the Git Server's HTTPS form or a ref outside the namespace its input is allowed to
//! name. The refs keep their exact spelling: Cloud fixes them and `RecordDispatch` compares them
//! byte for byte.

use ora_utils::GitBranchName;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;
use url::Url;

/// Namespace of the refs a delivery creates on the Git Server.
pub const DELIVERY_REF_PREFIX: &str = "refs/ora/deliveries/";
/// Namespace of the Binding snapshots and upstream branches the external line writes.
const UPSTREAM_REF_PREFIX: &str = "refs/ora/upstreams/";
/// Namespace of the Workspace branches.
const WORKSPACE_REF_PREFIX: &str = "refs/heads/workspaces/";
/// Longest Workspace branch short name a ref may carry.
pub const MAX_WORKSPACE_BRANCH_BYTES: usize = 200;

/// A Git Server repository address: `https`, no user information, query or fragment, kept in
/// its original spelling because Git receives exactly this text.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct GitRepositoryUrl(String);

/// Deliberately excludes the rejected address so diagnostics never echo what was sent.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Git Server repository must be an HTTPS URL without user information, query or fragment")]
pub struct InvalidGitRepositoryUrl;

impl GitRepositoryUrl {
    /// Checks the transport policy without normalizing distinct inputs together.
    pub fn parse(value: &str) -> Result<Self, InvalidGitRepositoryUrl> {
        // URL parsers repair whitespace, backslashes and missing authority delimiters. Git
        // receives the original text, so those alternative interpretations are refused.
        if value.chars().any(|c| c.is_whitespace() || c.is_control()) || value.contains('\\') {
            return Err(InvalidGitRepositoryUrl);
        }
        let rest = value
            .strip_prefix("https://")
            .ok_or(InvalidGitRepositoryUrl)?;
        let authority = rest.split('/').next().unwrap_or_default();
        let parsed = Url::parse(value).map_err(|_| InvalidGitRepositoryUrl)?;
        if authority.is_empty()
            || authority.contains('@')
            || parsed.host_str().is_none_or(str::is_empty)
            || matches!(parsed.path(), "" | "/")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || value.contains('?')
            || value.contains('#')
        {
            return Err(InvalidGitRepositoryUrl);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the address exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GitRepositoryUrl {
    type Error = InvalidGitRepositoryUrl;

    /// Keeps decoding subject to the same rules as local construction.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<GitRepositoryUrl> for String {
    /// Retains the original spelling in the wire and durable representations.
    fn from(value: GitRepositoryUrl) -> Self {
        value.0
    }
}

/// A ref outside the namespace its field allows, or with a malformed identity or commit.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{0} is outside its Git Server namespace or malformed")]
pub struct InvalidGitServerRef(&'static str);

/// `refs/ora/upstreams/<bindingId>/snapshots/<commit>`: the Binding snapshot a clone reads.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct SnapshotRef(String);

impl SnapshotRef {
    /// Requires a canonical Binding UUID and a full lowercase commit ID as the last segment.
    pub fn parse(value: &str) -> Result<Self, InvalidGitServerRef> {
        let invalid = InvalidGitServerRef("snapshot_ref");
        let rest = value
            .strip_prefix(UPSTREAM_REF_PREFIX)
            .ok_or(invalid.clone())?;
        let (binding, rest) = rest.split_once('/').ok_or(invalid.clone())?;
        let commit = rest.strip_prefix("snapshots/").ok_or(invalid.clone())?;
        if !is_canonical_uuid(binding) || !is_full_lowercase_commit(commit) {
            return Err(invalid);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the ref exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The Binding the snapshot belongs to; `ora-git fetch upstream` reads its branches.
    pub fn binding_id(&self) -> &str {
        self.segments().0
    }

    /// The commit the snapshot ref is named after, which must be the clone's base commit.
    pub fn commit(&self) -> &str {
        self.segments().1
    }

    /// Splits a value `parse` accepted into its Binding and commit.
    fn segments(&self) -> (&str, &str) {
        let rest = &self.0[UPSTREAM_REF_PREFIX.len()..];
        let (binding, rest) = rest.split_at(36);
        (binding, &rest["/snapshots/".len()..])
    }
}

impl TryFrom<String> for SnapshotRef {
    type Error = InvalidGitServerRef;

    /// Keeps decoding subject to the same rules as local construction.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<SnapshotRef> for String {
    /// Retains the exact spelling Cloud fixed.
    fn from(value: SnapshotRef) -> Self {
        value.0
    }
}

/// `refs/heads/workspaces/<workspaceId>/<branch>`: a Workspace branch on the Git Server.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct WorkspaceRef(String);

impl WorkspaceRef {
    /// Requires a canonical Workspace UUID and a literal short branch name other than `HEAD`.
    pub fn parse(value: &str) -> Result<Self, InvalidGitServerRef> {
        let invalid = InvalidGitServerRef("workspace_ref");
        let rest = value
            .strip_prefix(WORKSPACE_REF_PREFIX)
            .ok_or(invalid.clone())?;
        let (workspace, branch) = rest.split_once('/').ok_or(invalid.clone())?;
        if !is_canonical_uuid(workspace) || !is_workspace_branch(branch) {
            return Err(invalid);
        }
        Ok(Self(value.to_owned()))
    }

    /// The ref of `branch` in `workspace`, when both are valid.
    pub fn of(workspace: &str, branch: &str) -> Result<Self, InvalidGitServerRef> {
        Self::parse(&format!("{WORKSPACE_REF_PREFIX}{workspace}/{branch}"))
    }

    /// Returns the ref exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The Workspace the branch belongs to.
    pub fn workspace_id(&self) -> &str {
        &self.0[WORKSPACE_REF_PREFIX.len()..WORKSPACE_REF_PREFIX.len() + 36]
    }

    /// The branch short name, which is also the checkout's local branch.
    pub fn branch(&self) -> &str {
        &self.0[WORKSPACE_REF_PREFIX.len() + 37..]
    }
}

impl TryFrom<String> for WorkspaceRef {
    type Error = InvalidGitServerRef;

    /// Keeps decoding subject to the same rules as local construction.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<WorkspaceRef> for String {
    /// Retains the exact spelling Cloud fixed.
    fn from(value: WorkspaceRef) -> Self {
        value.0
    }
}

/// `refs/ora/deliveries/<deliveryWorkId>`: the ref one delivery creates and Cloud verifies.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeliveryRef(String);

impl DeliveryRef {
    /// Requires the delivery namespace and a suffix Git accepts as a ref component path.
    pub fn parse(value: &str) -> Result<Self, InvalidGitServerRef> {
        let valid = value
            .strip_prefix(DELIVERY_REF_PREFIX)
            .is_some_and(|suffix| GitBranchName::parse(suffix).is_ok());
        if !valid {
            return Err(InvalidGitServerRef("delivery_ref"));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the ref exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DeliveryRef {
    type Error = InvalidGitServerRef;

    /// Keeps decoding subject to the same rules as local construction.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<DeliveryRef> for String {
    /// Retains the exact spelling Cloud fixed.
    fn from(value: DeliveryRef) -> Self {
        value.0
    }
}

impl fmt::Display for WorkspaceRef {
    /// Prints the full ref; it carries no secret.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Whether `value` is a lowercase hyphenated UUID, the only spelling Cloud issues.
pub fn is_canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
}

/// Whether `value` is a full SHA-1 or SHA-256 object ID in lowercase hexadecimal.
pub fn is_full_lowercase_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `value` may name a Workspace branch: a literal short branch name, not `HEAD`, not an
/// option-looking name, bounded, and without the characters a refspec would interpret.
pub fn is_workspace_branch(value: &str) -> bool {
    value != "HEAD"
        && !value.starts_with('-')
        && value.len() <= MAX_WORKSPACE_BRANCH_BYTES
        && !value.contains([':', '+', '*'])
        && GitBranchName::parse(value).is_ok()
}
