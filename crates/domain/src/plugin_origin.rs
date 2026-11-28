use crate::plugin_namespace::PluginNamespace;
use ora_utils::url::distinguishing_repository_labels;
use std::collections::BTreeMap;
use thiserror::Error;

/// Classifies where an installed or listed plugin was first brought in from, for display only.
///
/// The classification follows the namespace alone, never the current marketplace configuration:
/// a namespace records the first source that introduced the plugin, so removing or disabling that
/// source later does not change the plugin's origin. The marketplace label is presentation, not
/// identity — it can change when a namesake source is added — and nothing may match on it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginOrigin {
    /// Imported from a local `.orax` archive (the reserved `local` namespace).
    LocalArchive,
    /// Published by the default first-party marketplace (the reserved `official` namespace).
    Official,
    /// First brought in from a third-party marketplace source.
    ///
    /// `source_url` is absent only when the namespace has lost its binding, which the append-only
    /// binding table should make impossible; see [`MissingSourceBinding`].
    Marketplace {
        source_url: Option<String>,
        label: String,
    },
}

/// Reports a derived namespace whose source binding cannot be found.
///
/// A binding is written before any plugin from its source is installed and is never deleted, so
/// this is a damaged-state signal. Callers log it and show the degraded origin instead of failing
/// a whole list over one entry.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("plugin namespace `{namespace}` has no recorded marketplace source")]
pub struct MissingSourceBinding {
    namespace: String,
}

impl MissingSourceBinding {
    /// Returns the namespace whose binding is missing, for diagnostics.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the best origin still showable: a marketplace without URL, labelled by the slug.
    ///
    /// The slug is the readable half of a derived namespace and was itself taken from the source
    /// URL's last segment, so it approximates the label the binding would have produced.
    pub fn into_degraded_origin(self) -> PluginOrigin {
        let slug = self
            .namespace
            .split_once('.')
            .map_or(self.namespace.as_str(), |(slug, _digest)| slug)
            .to_owned();
        PluginOrigin::Marketplace {
            source_url: None,
            label: slug,
        }
    }
}

/// One consistent snapshot of every marketplace source binding, with display labels computed.
///
/// Labels are computed against every binding ever recorded plus the default marketplace's URL,
/// rather than the configured sources: a plugin installed from a removed source is still listed,
/// and comparing only live sources could let two installed plugins show the same label. The
/// snapshot is built per query and never persisted, because adding one binding can change the
/// labels of others.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginOriginCatalog {
    marketplaces: BTreeMap<String, MarketplaceOrigin>,
}

/// The URL and computed label of one bound marketplace namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
struct MarketplaceOrigin {
    source_url: String,
    label: String,
}

impl PluginOriginCatalog {
    /// A snapshot with no bindings, under which only the reserved namespaces resolve.
    pub const EMPTY: Self = Self {
        marketplaces: BTreeMap::new(),
    };

    /// Builds the snapshot from `(namespace, canonical_url)` bindings.
    ///
    /// The default marketplace never appears in the binding table (it short-circuits to the
    /// reserved `official` namespace), but its canonical URL joins the comparison set so a lone
    /// fork of the official market is labelled `owner/marketplace` instead of the bare
    /// `marketplace` (origin presentation D3). The default source itself stays "official" and
    /// takes no marketplace entry, so its URL only ever disambiguates others.
    pub fn from_bindings(bindings: impl IntoIterator<Item = (String, String)>) -> Self {
        let (namespaces, urls): (Vec<String>, Vec<String>) = bindings.into_iter().unzip();
        // Appended last so the zip below keeps one label per binding and drops the default's.
        let mut comparison: Vec<&str> = urls.iter().map(String::as_str).collect();
        comparison.push(PluginNamespace::DEFAULT_MARKETPLACE_URL);
        let labels = distinguishing_repository_labels(&comparison);
        Self {
            marketplaces: namespaces
                .into_iter()
                .zip(urls.into_iter().zip(labels))
                .map(|(namespace, (source_url, label))| {
                    (namespace, MarketplaceOrigin { source_url, label })
                })
                .collect(),
        }
    }

    /// Classifies one plugin namespace, as spelled in a Canonical Plugin ID.
    pub fn origin_of(&self, namespace: &str) -> Result<PluginOrigin, MissingSourceBinding> {
        match namespace {
            PluginNamespace::LOCAL => Ok(PluginOrigin::LocalArchive),
            PluginNamespace::OFFICIAL => Ok(PluginOrigin::Official),
            derived => self
                .marketplaces
                .get(derived)
                .map(|origin| PluginOrigin::Marketplace {
                    source_url: Some(origin.source_url.clone()),
                    label: origin.label.clone(),
                })
                .ok_or_else(|| MissingSourceBinding {
                    namespace: derived.to_owned(),
                }),
        }
    }
}

/// Supplies a fresh [`PluginOriginCatalog`] to every query that shows plugins or plugin Skills.
///
/// Consumers ask once per query and resolve all their entries against that one snapshot, so
/// labels within a single response are consistent. Implementations read the binding store each
/// time instead of caching, because a new namesake source must relabel others without a restart.
/// A store failure must not fail the caller's whole list: implementations log it and return an
/// empty catalog, under which every derived namespace degrades like a missing binding.
pub trait PluginOriginSource: Send + Sync + 'static {
    /// Returns the current snapshot of every recorded source binding.
    fn plugin_origins(&self) -> PluginOriginCatalog;
}

/// A fixed catalog is its own source, for hosts that know their bindings up front and for tests.
impl PluginOriginSource for PluginOriginCatalog {
    fn plugin_origins(&self) -> PluginOriginCatalog {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginOrigin, PluginOriginCatalog};
    use crate::PluginNamespace;
    use pretty_assertions::assert_eq;

    /// Reserved namespaces classify without any binding, and bound ones carry a disambiguated label.
    #[test]
    fn classifies_reserved_and_bound_namespaces() {
        let catalog = PluginOriginCatalog::from_bindings([
            (
                "marketplace.11111111".to_string(),
                "https://github.com/acme/marketplace".to_string(),
            ),
            (
                "marketplace.22222222".to_string(),
                "https://github.com/obsismc/marketplace".to_string(),
            ),
        ]);

        assert_eq!(
            [
                catalog.origin_of(PluginNamespace::LOCAL),
                catalog.origin_of(PluginNamespace::OFFICIAL),
                catalog.origin_of("marketplace.22222222"),
            ],
            [
                Ok(PluginOrigin::LocalArchive),
                Ok(PluginOrigin::Official),
                Ok(PluginOrigin::Marketplace {
                    source_url: Some("https://github.com/obsismc/marketplace".to_string()),
                    label: "obsismc/marketplace".to_string(),
                }),
            ],
        );
    }

    /// A lone fork of the default marketplace is labelled against the default URL even though the
    /// default source itself is not a binding: it must read `obsismc/marketplace`, not the bare
    /// `marketplace` that would sit indistinguishably beside "official" (origin presentation D3).
    #[test]
    fn a_lone_official_fork_is_labelled_against_the_default_marketplace() {
        let catalog = PluginOriginCatalog::from_bindings([(
            "marketplace.22222222".to_string(),
            "https://github.com/obsismc/marketplace".to_string(),
        )]);

        assert_eq!(
            [
                catalog.origin_of(PluginNamespace::OFFICIAL),
                catalog.origin_of("marketplace.22222222"),
            ],
            [
                Ok(PluginOrigin::Official),
                Ok(PluginOrigin::Marketplace {
                    source_url: Some("https://github.com/obsismc/marketplace".to_string()),
                    label: "obsismc/marketplace".to_string(),
                }),
            ],
        );
    }

    /// A namespace without a binding degrades to its slug instead of failing.
    #[test]
    fn an_unbound_namespace_degrades_to_its_slug() {
        let missing = PluginOriginCatalog::default()
            .origin_of("tools.33333333")
            .expect_err("no binding");

        assert_eq!(
            (
                missing.namespace().to_owned(),
                missing.into_degraded_origin()
            ),
            (
                "tools.33333333".to_string(),
                PluginOrigin::Marketplace {
                    source_url: None,
                    label: "tools".to_string(),
                },
            ),
        );
    }
}
