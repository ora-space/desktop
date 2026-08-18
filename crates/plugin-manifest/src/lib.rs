//! Parses and validates the TOML manifest for one published Ora plugin release.

mod enums;
mod error;
mod manifest;
mod name;
mod sha256;
mod urls;

pub use enums::{PluginKind, PluginKindError, PluginNamespace, PluginNamespaceError};
pub use error::{InvalidFieldReason, ManifestError, ManifestField};
pub use manifest::{PluginDependencies, PluginHead, PluginManifest};
pub use name::{PluginName, PluginNameError};
pub use sha256::{Sha256Digest, Sha256DigestError};
pub use urls::{HomepageUrl, ReleaseUrl, RepositoryUrl, UrlError};

#[cfg(test)]
mod tests;
