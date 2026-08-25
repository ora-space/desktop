use ora_domain::PluginId;
use ora_plugin_manifest::PluginManifest;
use semver::Version;
use serde::{Deserialize, Serialize};

/// One lightweight metadata record surfaced in the registry index for UI consumption.
///
/// The index is a derived artifact, so this type stores the display fields as plain strings
/// rather than re-validating manifest invariants at load time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegistryEntry {
    id: PluginId,
    name: String,
    /// Human-readable display title from the manifest. Old cached indexes predate this field, so
    /// it defaults to empty and consumers fall back to `name` until the next resync.
    #[serde(default)]
    title: String,
    /// The plugin kind (`agent`, `workbench`, or `webview`) surfaced for the marketplace card.
    #[serde(default)]
    kind: String,
    namespace: String,
    version: Version,
    description: String,
    /// Trusted SVG source for the entry icon, absent when the entry ships none.
    ///
    /// The icon is inlined into the index rather than referenced by path so consumers can render
    /// the marketplace listing straight from the cached index without reaching back into the
    /// source checkout, which install-time resolution is the only step that still needs.
    #[serde(default)]
    logo: Option<String>,
}

impl RegistryEntry {
    /// Builds one index record from a validated plugin manifest and its already-validated icon.
    pub(crate) fn from_manifest(manifest: &PluginManifest, logo: Option<String>) -> Self {
        Self {
            id: entry_id(manifest),
            name: manifest.name().as_str().to_owned(),
            title: manifest.title().to_owned(),
            kind: manifest.kind().as_str().to_owned(),
            namespace: manifest.namespace().as_str().to_owned(),
            version: manifest.version().clone(),
            description: manifest.description().to_owned(),
            logo,
        }
    }

    /// Returns the unique `namespace/name` identifier.
    pub fn id(&self) -> &PluginId {
        &self.id
    }

    /// Returns the plugin name (the identifier segment).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the human-readable display title, empty when an older cache indexed it without one.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the plugin kind, empty when an older cache indexed it without one.
    pub fn kind(&self) -> &str {
        &self.kind
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

    /// Returns the trusted SVG source of the entry icon, when one is published.
    pub fn logo(&self) -> Option<&str> {
        self.logo.as_deref()
    }
}

/// Derives the unique `namespace/name` identifier a manifest resolves to.
///
/// Identifier construction is shared by index building and install-time lookup, so both agree on
/// what a marketplace identifier means without the lookup path having to build a whole entry.
pub(crate) fn entry_id(manifest: &PluginManifest) -> PluginId {
    // The manifest grammar is a strict subset of what `PluginId` accepts, so this cannot fail for
    // a manifest that already parsed; the fallback keeps the function total.
    PluginId::new(manifest.namespace().as_str(), manifest.name().as_str())
        .unwrap_or_else(|error| unreachable!("validated manifest name is a plugin id: {error}"))
}
