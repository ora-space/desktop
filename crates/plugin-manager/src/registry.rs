use crate::{EffectiveEnablement, ValidatedPackage};
use ora_plugin_protocol::{
    AgentProviderKey, ContentDigest, ContentOwnerId, JsonSafeU64, PluginId, PluginManifest,
    PluginVersion,
};
use std::collections::HashMap;

/// One immutable running-eligibility descriptor derived from a fresh package proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredPlugin {
    pub plugin_id: PluginId,
    pub plugin_version: PluginVersion,
    pub content_digest: ContentDigest,
    pub content_owner: ContentOwnerId,
    pub enablement_epoch: JsonSafeU64,
}

/// One manifest-declared Agent provider routed by a structured global key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredAgent {
    pub key: AgentProviderKey,
    pub contract_version: u32,
}

/// An immutable registry snapshot published before its bounded revision event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrySnapshot {
    pub revision: JsonSafeU64,
    pub agents_by_provider: HashMap<AgentProviderKey, RegisteredAgent>,
    pub plugins_by_id: HashMap<PluginId, RegisteredPlugin>,
}

impl RegistrySnapshot {
    pub fn empty() -> Self {
        Self {
            revision: JsonSafeU64::new(0)
                .unwrap_or_else(|error| panic!("zero registry revision must be valid: {error}")),
            agents_by_provider: HashMap::new(),
            plugins_by_id: HashMap::new(),
        }
    }
}

/// Input facts from state and catalog that can produce an enabled registry entry.
pub struct RegistryCandidate<'a> {
    pub package: &'a ValidatedPackage,
    pub content_owner: &'a ContentOwnerId,
    pub enablement_epoch: JsonSafeU64,
    pub effective_enablement: EffectiveEnablement,
}

/// The single registry writer with source-revision rollback protection.
#[derive(Debug)]
pub struct RuntimeRegistry {
    snapshot: RegistrySnapshot,
    applied_catalog_revision: JsonSafeU64,
    applied_state_revision: JsonSafeU64,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self {
            snapshot: RegistrySnapshot::empty(),
            applied_catalog_revision: JsonSafeU64::new(0)
                .unwrap_or_else(|error| panic!("zero catalog revision must be valid: {error}")),
            applied_state_revision: JsonSafeU64::new(0)
                .unwrap_or_else(|error| panic!("zero state revision must be valid: {error}")),
        }
    }

    pub fn snapshot(&self) -> RegistrySnapshot {
        self.snapshot.clone()
    }

    /// Atomically replaces a changed registry and refuses either source revision moving backward.
    pub fn reconcile(
        &mut self,
        catalog_revision: JsonSafeU64,
        state_revision: JsonSafeU64,
        candidates: &[RegistryCandidate<'_>],
    ) -> Result<RegistrySnapshot, RegistryError> {
        if catalog_revision < self.applied_catalog_revision
            || state_revision < self.applied_state_revision
        {
            return Err(RegistryError::SourceRevisionRollback);
        }

        let mut plugins = HashMap::new();
        let mut agents = HashMap::new();
        for candidate in candidates {
            if candidate.effective_enablement != EffectiveEnablement::Enabled {
                continue;
            }
            let PluginManifest::Agent { contributes, .. } = &candidate.package.manifest.ora else {
                continue;
            };
            let plugin_id = candidate.package.manifest.ora.id().clone();
            let plugin = RegisteredPlugin {
                plugin_id: plugin_id.clone(),
                plugin_version: candidate.package.manifest.version.clone(),
                content_digest: candidate.package.digest.digest.clone(),
                content_owner: candidate.content_owner.clone(),
                enablement_epoch: candidate.enablement_epoch,
            };
            if plugins.insert(plugin_id.clone(), plugin).is_some() {
                return Err(RegistryError::DuplicatePlugin { plugin_id });
            }
            for contribution in &contributes.agents {
                let key = AgentProviderKey {
                    plugin_id: plugin_id.clone(),
                    provider_id: contribution.id.clone(),
                };
                let registered = RegisteredAgent {
                    key: key.clone(),
                    contract_version: contribution.contract_version,
                };
                if agents.insert(key.clone(), registered).is_some() {
                    return Err(RegistryError::DuplicateProvider { key });
                }
            }
        }
        // Source scans can advance or repeat without changing admission. Keeping the published
        // revision stable is what makes the post-activation TOCTOU check meaningful.
        if self.snapshot.plugins_by_id == plugins && self.snapshot.agents_by_provider == agents {
            self.applied_catalog_revision = catalog_revision;
            self.applied_state_revision = state_revision;
            return Ok(self.snapshot.clone());
        }

        let revision = self
            .snapshot
            .revision
            .checked_increment()
            .map_err(|_| RegistryError::RevisionExhausted)?;
        self.snapshot = RegistrySnapshot {
            revision,
            agents_by_provider: agents,
            plugins_by_id: plugins,
        };
        self.applied_catalog_revision = catalog_revision;
        self.applied_state_revision = state_revision;
        Ok(self.snapshot.clone())
    }
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry invariant failures that close admission rather than overwriting entries.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("registry source revision attempted to move backward")]
    SourceRevisionRollback,
    #[error("duplicate canonical plugin id: {plugin_id}")]
    DuplicatePlugin { plugin_id: PluginId },
    #[error("duplicate structured Agent provider key: {key:?}")]
    DuplicateProvider { key: AgentProviderKey },
    #[error("registry revision reached the JSON-safe maximum")]
    RevisionExhausted,
}

#[cfg(test)]
mod tests {
    use super::{RegistryError, RegistrySnapshot, RuntimeRegistry};
    use ora_plugin_protocol::JsonSafeU64;
    use pretty_assertions::assert_eq;

    /// A stale catalog or state scan cannot overwrite a newer registry decision.
    #[test]
    fn rejects_source_revision_rollback() {
        let mut registry = RuntimeRegistry::new();
        let one =
            JsonSafeU64::new(1).unwrap_or_else(|error| panic!("expected revision one: {error}"));
        registry
            .reconcile(one, one, &[])
            .unwrap_or_else(|error| panic!("expected first reconcile: {error}"));
        let zero =
            JsonSafeU64::new(0).unwrap_or_else(|error| panic!("expected revision zero: {error}"));
        assert_eq!(
            registry.reconcile(zero, one, &[]),
            Err(RegistryError::SourceRevisionRollback)
        );
    }

    /// Repeating an equivalent source scan cannot invalidate a generation during activation.
    #[test]
    fn preserves_revision_when_admission_projection_is_unchanged() {
        let mut registry = RuntimeRegistry::new();
        let one =
            JsonSafeU64::new(1).unwrap_or_else(|error| panic!("expected revision one: {error}"));
        let two =
            JsonSafeU64::new(2).unwrap_or_else(|error| panic!("expected revision two: {error}"));

        let first = registry
            .reconcile(one, one, &[])
            .unwrap_or_else(|error| panic!("expected first reconcile: {error}"));
        let second = registry
            .reconcile(two, two, &[])
            .unwrap_or_else(|error| panic!("expected equivalent reconcile: {error}"));

        assert_eq!(
            (first, second),
            (RegistrySnapshot::empty(), RegistrySnapshot::empty())
        );
    }
}
