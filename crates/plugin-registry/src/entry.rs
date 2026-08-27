use ora_domain::PluginId;
use ora_plugin_manifest::{PluginManifest, PluginReleaseSource};
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
    /// The plugin kind (`agent`, `workbench`, `webview`, `skill`, `mcp`, or `hook`) surfaced for
    /// the marketplace card.
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
    /// Cached release-source target support, so the UI can disable installation of an
    /// unsupported target before downloading any artifact.
    ///
    /// `None` means the listing has no downloadable release. `Some([])` is a universal release
    /// compatible with every host. `Some(non-empty)` lists the exact target triples.
    #[serde(default)]
    release_targets: Option<Vec<String>>,
}

impl RegistryEntry {
    /// Builds one index record from a validated plugin manifest and its already-validated icon.
    pub(crate) fn from_manifest(manifest: &PluginManifest, logo: Option<String>) -> Self {
        let release_targets = match manifest.release_source() {
            Some(PluginReleaseSource::Universal { .. }) => Some(Vec::new()),
            Some(PluginReleaseSource::Targets(targets)) => Some(
                targets
                    .iter()
                    .map(|target| target.target().as_str().to_owned())
                    .collect(),
            ),
            None => None,
        };
        Self {
            id: entry_id(manifest),
            name: manifest.name().as_str().to_owned(),
            title: manifest.title().to_owned(),
            kind: manifest.kind().as_str().to_owned(),
            namespace: manifest.namespace().as_str().to_owned(),
            version: manifest.version().clone(),
            description: manifest.description().to_owned(),
            logo,
            release_targets,
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

    /// Returns the target triples the release ships artifacts for.
    ///
    /// `None` means the listing has no downloadable release. An empty slice is a universal
    /// release. A non-empty slice is the exact targeted triples.
    pub fn release_targets(&self) -> Option<&[String]> {
        self.release_targets.as_deref()
    }

    /// Returns whether the current host can install this release.
    pub fn is_compatible_with_host(&self) -> bool {
        self.host_compatibility().is_ok()
    }

    /// Returns a human-readable incompatibility reason for the current host, or `None` when the
    /// host can install the release.
    pub fn incompatible_reason_for_host(&self) -> Option<String> {
        self.host_compatibility().err()
    }

    /// Computes host compatibility once so callers can take either the success or the reason
    /// without allocating a reason string just to discard it.
    pub fn host_compatibility(&self) -> Result<(), String> {
        match &self.release_targets {
            None => Err("this listing has no downloadable release".to_string()),
            Some(targets) if targets.is_empty() => Ok(()),
            Some(targets) => {
                let Some(host) = crate::host::current_host_target() else {
                    return Err(format!(
                        "this release supports {} but the host is not a supported plugin target",
                        targets.join(", ")
                    ));
                };
                if targets.iter().any(|target| target == host.as_str()) {
                    Ok(())
                } else {
                    Err(format!(
                        "this release supports {} but your host is {host}",
                        targets.join(", ")
                    ))
                }
            }
        }
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
