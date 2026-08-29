//! Names the outcome of comparing a plugin's optional `[dependencies].ora` with a host version.
//!
//! Matching itself lives on `PluginManifest::ora_host_compatibility` so marketplace install,
//! local import, update preparation, and startup discovery cannot call a second helper that
//! drifts. Callers must supply the running Desktop product version; a workspace crate version
//! such as `0.0.0` would accept or reject packages independently of the product the user actually
//! runs.

use semver::{Version, VersionReq};

/// Outcome of comparing a parsed `[dependencies].ora` requirement with a Desktop product version.
///
/// Absence of a requirement is `Satisfied`: existing packages that omit the field stay installable
/// and discoverable. A malformed constraint never reaches this type; `PluginManifest` parse reports
/// the existing `invalid_manifest` field error instead of incompatibility.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OraHostDependencyMatch {
    Satisfied,
    Unsatisfied {
        actual: Version,
        required: VersionReq,
    },
}
