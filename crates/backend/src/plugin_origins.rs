//! Classifies plugin origins from the marketplace source bindings, freshly for every query.

use ora_db::SqlitePluginSourceNamespaceRepository;
use ora_domain::{PluginOriginCatalog, PluginOriginSource};
use ora_logging::ora_warn;

/// Reads every recorded source binding into a [`PluginOriginCatalog`] on demand.
///
/// Nothing is cached: a new namesake source changes other sources' labels, and reading the
/// binding table — a handful of rows — per query is cheaper than keeping a cache coherent.
#[derive(Clone, Debug)]
pub(crate) struct SourceNamespaceOrigins {
    namespaces: SqlitePluginSourceNamespaceRepository,
}

impl SourceNamespaceOrigins {
    pub(crate) fn new(namespaces: SqlitePluginSourceNamespaceRepository) -> Self {
        Self { namespaces }
    }
}

impl PluginOriginSource for SourceNamespaceOrigins {
    fn plugin_origins(&self) -> PluginOriginCatalog {
        match self.namespaces.list_bindings() {
            Ok(bindings) => PluginOriginCatalog::from_bindings(
                bindings
                    .into_iter()
                    .map(|binding| (binding.namespace, binding.canonical_url)),
            ),
            // Origins only label entries; failing every plugin and Skill list over them would
            // hide what the user is trying to see, so every derived namespace degrades instead.
            Err(error) => {
                ora_warn!(%error, "failed to read plugin source bindings for origin labels");
                PluginOriginCatalog::EMPTY
            }
        }
    }
}
