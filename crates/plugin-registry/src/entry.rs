use ora_domain::{PluginId, PluginNamespace};
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
    /// The manifest identifier segment, serialized under the schema's `identifier` spelling.
    ///
    /// `name` remains an accepted alias so a cache written by an older desktop can still be
    /// loaded and replaced by the next successful registry sync.
    #[serde(rename = "identifier", alias = "name")]
    identifier: String,
    /// Human-readable display title from the manifest. Old cached indexes predate this field, so
    /// it defaults to empty and consumers fall back to `identifier` until the next resync.
    #[serde(default)]
    title: String,
    /// The plugin kind (`agent`, `workbench`, `webview`, `skill`, `mcp`, or `hook`) surfaced for
    /// the marketplace card.
    #[serde(default)]
    kind: String,
    namespace: String,
    /// Canonical URL of the marketplace source that published this entry.
    ///
    /// Two sources may publish the same `identifier`, and every display field on the card comes
    /// from the entry manifest, which either source can copy verbatim. Attribution is therefore
    /// the only thing that tells the two cards apart, and it must travel with the entry rather
    /// than being reconstructed from the namespace, which is an opaque digest to a reader.
    #[serde(default)]
    source_url: String,
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
    /// Builds one index record from a validated plugin manifest, the identity of the source that
    /// publishes it, and its already-validated icon.
    pub(crate) fn from_manifest(
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
        source_url: &str,
        logo: Option<String>,
    ) -> Self {
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
            id: entry_id(manifest, namespace),
            identifier: manifest.name().as_str().to_owned(),
            title: manifest.title().to_owned(),
            kind: manifest.kind().as_str().to_owned(),
            namespace: namespace.as_str().to_owned(),
            source_url: source_url.to_owned(),
            version: manifest.version().clone(),
            description: manifest.description().to_owned(),
            logo,
            release_targets,
        }
    }

    /// Returns the unique `namespace/identifier` identifier.
    pub fn id(&self) -> &PluginId {
        &self.id
    }

    /// Returns the plugin identifier segment.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Returns the human-readable display title, empty when an older cache indexed it without one.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the plugin kind, empty when an older cache indexed it without one.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Returns the namespace of the source that publishes this entry.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the canonical URL of the source that publishes this entry.
    pub fn source_url(&self) -> &str {
        &self.source_url
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

/// Derives the unique `namespace/identifier` a manifest resolves to under `namespace`.
///
/// The namespace comes from the publishing source, never from the manifest, so the same
/// `identifier` published by two repositories is two distinct entries rather than one entry that
/// silently shadows the other. Identifier construction is shared by index building and
/// install-time lookup so both agree on what a marketplace identifier means.
pub(crate) fn entry_id(manifest: &PluginManifest, namespace: &PluginNamespace) -> PluginId {
    // The manifest grammar is a strict subset of what `PluginId` accepts and the namespace is
    // already a validated segment, so this cannot fail for a manifest that parsed; the fallback
    // keeps the function total.
    PluginId::new(namespace.clone(), manifest.name().as_str())
        .unwrap_or_else(|error| unreachable!("validated manifest name is a plugin id: {error}"))
}
