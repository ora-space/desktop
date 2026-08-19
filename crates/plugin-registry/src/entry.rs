use ora_plugin_manifest::PluginManifest;
use semver::Version;
use serde::{Deserialize, Serialize};

/// One lightweight metadata record surfaced in the registry index for UI consumption.
///
/// The index is a derived artifact, so this type stores the display fields as plain strings
/// rather than re-validating manifest invariants at load time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegistryEntry {
    id: String,
    name: String,
    namespace: String,
    version: Version,
    description: String,
}

impl RegistryEntry {
    /// Builds one index record from a validated plugin manifest.
    pub(crate) fn from_manifest(manifest: &PluginManifest) -> Self {
        let id = format!("{}/{}", manifest.namespace(), manifest.name());
        Self {
            id,
            name: manifest.name().as_str().to_owned(),
            namespace: manifest.namespace().as_str().to_owned(),
            version: manifest.version().clone(),
            description: manifest.description().to_owned(),
        }
    }

    /// Returns the unique `namespace/name` identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the plugin name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the plugin source namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the published plugin version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Returns the plugin description.
    pub fn description(&self) -> &str {
        &self.description
    }
}
