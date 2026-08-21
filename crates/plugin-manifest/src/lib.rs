//! Parses and validates the `orax.toml` manifest of one Ora plugin, both in its marketplace
//! release form and in the form shipped inside an installed package.

mod enums;
mod error;
mod manifest;
mod name;
mod sha256;
mod ui;
mod urls;

pub use enums::{PluginKind, PluginKindError, PluginNamespace, PluginNamespaceError};
pub use error::{InvalidFieldReason, ManifestError, ManifestField, SurfaceField};
pub use manifest::{PluginDependencies, PluginHead, PluginManifest};
pub use name::{PluginName, PluginNameError};
pub use sha256::{Sha256Digest, Sha256DigestError};
pub use ui::{
    PluginUi, SurfaceDeclaration, SurfaceInstances, SurfaceInstancesError, SurfaceSource,
    WebDataMode, WebDataModeError,
};
pub use urls::{HomepageUrl, ReleaseUrl, RepositoryUrl, UrlError};

#[cfg(test)]
mod tests;
