//! Evaluates a plugin's optional `[dependencies].ora` requirement against a host version.
//!
//! Parsing and matching live here so marketplace install, local import, update preparation, and
//! startup discovery share one decision. Callers must supply the running Desktop product version;
//! a workspace crate version such as `0.0.0` would accept or reject packages independently of the
//! product the user actually runs.

use crate::PluginDependencies;
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

/// Compares `dependencies` with `host_version` using the same SemVer requirement grammar the
/// manifest parser already accepted.
pub fn evaluate_ora_host_dependency(
    dependencies: Option<&PluginDependencies>,
    host_version: &Version,
) -> OraHostDependencyMatch {
    match dependencies {
        None => OraHostDependencyMatch::Satisfied,
        Some(dependencies) if dependencies.ora().matches(host_version) => {
            OraHostDependencyMatch::Satisfied
        }
        Some(dependencies) => OraHostDependencyMatch::Unsatisfied {
            actual: host_version.clone(),
            required: dependencies.ora().clone(),
        },
    }
}
