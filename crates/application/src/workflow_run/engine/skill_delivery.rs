use ora_contracts::WorkflowRunLocale;
use ora_domain::AgentRef;
use ora_utils::path::StrictRelativePath;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Describes how an Agent accepts filesystem-delivered skill packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSkillDelivery {
    /// The Agent cannot consume workflow-managed skills.
    Unsupported,
    /// The Agent discovers one copied package below each declared worktree-relative root.
    Filesystem {
        discovery_roots: SkillDiscoveryRoots,
    },
}

/// A non-empty, ordered set of worktree-relative roots an Agent scans for skill packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDiscoveryRoots(Vec<StrictRelativePath>);

impl SkillDiscoveryRoots {
    /// Creates a capability with one required root followed by any additional discovery roots.
    pub fn new(first: StrictRelativePath, additional: Vec<StrictRelativePath>) -> Self {
        let mut roots = vec![first];
        for root in additional {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        Self(roots)
    }

    /// Iterates over the stable, de-duplicated discovery-root order.
    pub fn iter(&self) -> impl Iterator<Item = &StrictRelativePath> {
        self.0.iter()
    }
}

/// Reports why an Agent's frozen skill-delivery capability could not be obtained.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentSkillDeliveryError {
    #[error("agent skill-delivery capability is unavailable")]
    Unavailable,
    #[error("agent skill-delivery capability is invalid: {message}")]
    Invalid { message: String },
}

/// Resolves the skill-delivery capability declared by an Agent provider.
///
/// Implementations are expected to return a stable capability snapshot suitable for freezing into
/// a workflow run. Plugin-backed implementations should read a previously validated capability
/// registry rather than contacting a live plugin during worktree creation.
pub trait AgentSkillDeliveryProvider: Send + Sync {
    /// Returns how the named Agent expects skill packages to be placed in its session worktree.
    fn skill_delivery(
        &self,
        agent_ref: &AgentRef,
    ) -> Result<AgentSkillDelivery, AgentSkillDeliveryError>;
}

/// Records the actual skill packages made available to one frozen workflow run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMaterializationReceipt {
    pub bindings: Vec<MaterializedSkillBinding>,
}

impl SkillMaterializationReceipt {
    /// Returns the frozen skill bindings belonging to one graph node in declaration order.
    pub fn bindings_for_node(&self, node_id: &str) -> Vec<&MaterializedSkillBinding> {
        self.bindings
            .iter()
            .filter(|binding| binding.node_id == node_id)
            .collect()
    }
}

/// Binds one graph node's declared skill to its executable name and actual package locations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializedSkillBinding {
    pub node_id: String,
    pub skill_id: String,
    pub invocation_name: String,
    pub package_paths: Vec<StrictRelativePath>,
}

impl MaterializedSkillBinding {
    /// Resolves every frozen package path beneath the run worktree for prompt presentation.
    pub fn absolute_package_paths(&self, worktree_root: &Path) -> Vec<PathBuf> {
        self.package_paths
            .iter()
            .map(|path| path.to_path(worktree_root))
            .collect()
    }
}

/// Internal payload frozen with a workflow run at creation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunPayload {
    pub locale: WorkflowRunLocale,
    pub skill_materialization: SkillMaterializationReceipt,
}

impl WorkflowRunPayload {
    /// Creates the immutable execution metadata captured while the run worktree is initialized.
    pub fn new(
        locale: WorkflowRunLocale,
        skill_materialization: SkillMaterializationReceipt,
    ) -> Self {
        Self {
            locale,
            skill_materialization,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Persisted receipts round-trip without losing normalized placement paths.
    #[test]
    fn workflow_run_payload_round_trips_materialized_skill_bindings() {
        let expected = WorkflowRunPayload::new(
            WorkflowRunLocale::EnUs,
            SkillMaterializationReceipt {
                bindings: vec![MaterializedSkillBinding {
                    node_id: "review".to_string(),
                    skill_id: "catalog-id".to_string(),
                    invocation_name: "review".to_string(),
                    package_paths: vec![StrictRelativePath::parse(".agent/skills/review").unwrap()],
                }],
            },
        );

        let encoded = serde_json::to_string(&expected).unwrap();
        assert_eq!(
            serde_json::from_str::<WorkflowRunPayload>(&encoded).unwrap(),
            expected
        );
    }

    /// Deserialization revalidates receipt paths so persisted traversal cannot escape a worktree.
    #[test]
    fn workflow_run_payload_rejects_unsafe_materialization_paths() {
        let encoded = r#"{"locale":"en-US","skillMaterialization":{"bindings":[{"nodeId":"review","skillId":"catalog-id","invocationName":"review","packagePaths":["../escape"]}]}}"#;

        assert!(serde_json::from_str::<WorkflowRunPayload>(encoded).is_err());
    }
}
