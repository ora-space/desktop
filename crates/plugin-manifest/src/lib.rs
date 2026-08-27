//! Parses and validates the `orax.toml` manifest of one Ora plugin, both in its marketplace
//! release form and in the form shipped inside an installed package.

mod enums;
mod error;
mod manifest;
mod name;
mod sha256;
mod target;
mod urls;
mod webview;
mod workbench;

pub use enums::{PluginKind, PluginKindError, PluginNamespace, PluginNamespaceError};
pub use error::{InvalidFieldReason, ManifestError, ManifestField, RuleField};
pub use manifest::{
    PluginArtifact, PluginDependencies, PluginHead, PluginManifest, PluginReleaseSource,
    PluginReleaseTarget,
};
pub use name::{PluginName, PluginNameError};
pub use sha256::{Sha256Digest, Sha256DigestError};
pub use target::{HookTarget, HookTargetError};
pub use urls::{HomepageUrl, ReleaseUrl, RepositoryUrl, UrlError};
pub use webview::{
    DownloadAction, DownloadActionError, DownloadDisposition, DownloadPolicy, DownloadRule, Origin,
    PageMatcher, PathPrefix, PathPrefixError, PluginWebview, StartUrl, WebviewUrlError,
};
pub use workbench::{MethodName, MethodNameError, PluginWorkbench};

#[cfg(test)]
mod tests;
