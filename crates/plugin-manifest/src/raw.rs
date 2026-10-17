use serde::Deserialize;

use crate::agent_configuration::RawAgentConfiguration;
use crate::pack::RawPack;
use crate::webview::RawWebview;
use crate::workbench::RawWorkbench;

#[derive(Deserialize)]
pub(crate) struct RawPluginManifest {
    pub(crate) resolver: u64,
    pub(crate) identifier: String,
    pub(crate) title: Option<String>,
    pub(crate) kind: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) homepage: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) sha256: Option<String>,
    pub(crate) head: Option<RawHead>,
    pub(crate) dependencies: Option<RawDependencies>,
    pub(crate) workbench: Option<RawWorkbench>,
    pub(crate) webview: Option<RawWebview>,
    #[serde(default)]
    pub(crate) targets: Option<Vec<RawReleaseTarget>>,
    #[serde(default)]
    pub(crate) artifact: Option<RawArtifact>,
    #[serde(default)]
    pub(crate) agent_configuration: Option<RawAgentConfiguration>,
    #[serde(default)]
    pub(crate) pack: Option<RawPack>,
    #[serde(default)]
    pub(crate) marketplace_visible: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct RawInstalledManifest {
    pub(crate) resolver: Option<u64>,
    /// Identifier segment of the installed package, spelled `identifier` (not `name`) because an
    /// installed manifest is only ever addressed by the full id the host resolves by pairing this
    /// name with the namespace of the directory the package is installed under.
    pub(crate) identifier: String,
    pub(crate) title: Option<String>,
    pub(crate) kind: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) homepage: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) sha256: Option<String>,
    pub(crate) head: Option<RawHead>,
    pub(crate) dependencies: Option<RawDependencies>,
    pub(crate) workbench: Option<RawWorkbench>,
    pub(crate) webview: Option<RawWebview>,
    #[serde(default)]
    pub(crate) targets: Option<Vec<RawReleaseTarget>>,
    #[serde(default)]
    pub(crate) artifact: Option<RawArtifact>,
    // The release-form publication association is parsed here only so the installed form can
    // reject it explicitly; a package that repeated its own marketplace copy locator would carry
    // a second, unverifiable claim about bytes the host has directly beside it.
    #[serde(default)]
    pub(crate) agent_configuration: Option<RawAgentConfiguration>,
    #[serde(default)]
    pub(crate) pack: Option<RawPack>,
    #[serde(default)]
    pub(crate) marketplace_visible: Option<bool>,
}

#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawHead {
    pub(crate) repository: String,
    pub(crate) branch: String,
}

#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawDependencies {
    pub(crate) ora: Option<String>,
}

/// Raw form of one `[[targets]]` release entry.
#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawReleaseTarget {
    pub(crate) target: String,
    pub(crate) url: String,
    pub(crate) sha256: String,
}

/// Raw form of the installed `[artifact]` self-declaration.
#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawArtifact {
    pub(crate) target: String,
}

/// Holds the descriptive metadata shared by both manifest forms.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawMetadata {
    pub(crate) name: String,
    pub(crate) title: Option<String>,
    pub(crate) kind: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) homepage: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) head: Option<RawHead>,
    pub(crate) dependencies: Option<RawDependencies>,
    pub(crate) workbench: Option<RawWorkbench>,
    pub(crate) webview: Option<RawWebview>,
    pub(crate) targets: Option<Vec<RawReleaseTarget>>,
    pub(crate) artifact: Option<RawArtifact>,
    pub(crate) agent_configuration: Option<RawAgentConfiguration>,
    pub(crate) pack: Option<RawPack>,
    pub(crate) marketplace_visible: Option<bool>,
}

impl RawPluginManifest {
    /// Splits the release form into shared metadata and optional download fields.
    pub(crate) fn into_parts(self) -> (RawMetadata, u64, Option<String>, Option<String>) {
        let metadata = RawMetadata {
            // The marketplace release form spells the name segment `identifier` like the
            // installed form, and the download fields are optional now that the marketplace no
            // longer publishes `.orax` release URLs.
            name: self.identifier,
            title: self.title,
            kind: self.kind,
            version: self.version,
            description: self.description,
            homepage: self.homepage,
            license: self.license,
            head: self.head,
            dependencies: self.dependencies,
            workbench: self.workbench,
            webview: self.webview,
            targets: self.targets,
            artifact: self.artifact,
            agent_configuration: self.agent_configuration,
            pack: self.pack,
            marketplace_visible: self.marketplace_visible,
        };
        (metadata, self.resolver, self.url, self.sha256)
    }
}

impl RawInstalledManifest {
    /// Splits the installed form into shared metadata and optional download fields.
    pub(crate) fn into_parts(self) -> (RawMetadata, Option<u64>, Option<String>, Option<String>) {
        let metadata = RawMetadata {
            // The installed manifest spells the name segment `identifier`, mapping it onto the
            // shared metadata name so both forms converge on one validated domain model.
            name: self.identifier,
            title: self.title,
            kind: self.kind,
            version: self.version,
            description: self.description,
            homepage: self.homepage,
            license: self.license,
            head: self.head,
            dependencies: self.dependencies,
            workbench: self.workbench,
            webview: self.webview,
            targets: self.targets,
            artifact: self.artifact,
            agent_configuration: self.agent_configuration,
            pack: self.pack,
            marketplace_visible: self.marketplace_visible,
        };
        (metadata, self.resolver, self.url, self.sha256)
    }
}
