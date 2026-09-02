//! Maps Agent plugin declarations and invokes the generic Consumer adapter protocol.

use ora_domain::PluginId;
use ora_effect::{
    AdapterReceipt, CapabilityRequirement, CapabilitySet, ConsumerAdapterIdentity,
    ConsumerDeclaration, ConsumerIdentity, ConsumerKind, CoordinationContract, CoordinationPlan,
    CoordinationRequirement, EffectKind, EffectTarget, FilesystemResourceTemplate,
    MaterializationContract, MaterializationFormat, ResourcePath, TargetProjection,
};
use ora_plugin_protocol::{
    AgentEffectCoordinationContext, AgentEffectReadinessContext, EFFECT_COORDINATE_METHOD,
    EFFECT_REACTIVATE_METHOD, EFFECT_VERIFY_READY_METHOD,
};
use ora_plugin_runtime::{
    PluginEffectCoordination, PluginRegistration, PluginRuntime, PluginRuntimeError,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use thiserror::Error;

/// Reports an invalid declaration or failed Consumer adapter invocation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum AgentEffectError {
    #[error("agent plugin Effect declaration is invalid: {0}")]
    InvalidDeclaration(String),
    #[error("agent plugin Effect IPC failed: {0}")]
    Ipc(String),
}

/// Abstracts one IPC generation so the Consumer protocol can be tested in isolation.
trait AgentEffectRuntime {
    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send;
}

impl AgentEffectRuntime for PluginRuntime {
    async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
        PluginRuntime::invoke(self, method, params)
            .await
            .map_err(|error: PluginRuntimeError| error.to_string())
    }
}

/// Converts one immutable plugin registration into a host-owned Consumer declaration.
pub(crate) fn registered_consumer_declaration(
    plugin_id: &PluginId,
    registration: &PluginRegistration,
) -> Result<Option<ConsumerDeclaration>, AgentEffectError> {
    if registration.effect_resources.is_empty() {
        return Ok(None);
    }
    let consumer = ConsumerIdentity::new(ConsumerKind::agent_plugin(), plugin_id.canonical())
        .map_err(|error| AgentEffectError::InvalidDeclaration(error.to_string()))?;
    let coordination = CoordinationContract::agent_restart_v1();
    let mut effect_protocols = BTreeMap::new();
    let mut materialization_contracts = BTreeSet::new();
    let mut resources = Vec::new();
    for resource in &registration.effect_resources {
        let relative_path = ResourcePath::parse(&resource.workspace_relative_path)
            .map_err(|error| AgentEffectError::InvalidDeclaration(error.to_string()))?;
        let format = MaterializationFormat::parse(&resource.materialization_format)
            .map_err(|error| AgentEffectError::InvalidDeclaration(error.to_string()))?;
        // Unpublished MCP file-materialization agents declared these formats. Skip them so Skill
        // Effect still registers without treating MCP as an Effect Resource.
        if resource.materialization_format == "ora/opencode-mcp-config.v1"
            || resource.materialization_format == "ora/claude-mcp-config.v1"
        {
            continue;
        }
        if format != MaterializationFormat::skill_directory_v1() {
            return Err(AgentEffectError::InvalidDeclaration(format!(
                "unsupported Effect materialization format {}",
                resource.materialization_format
            )));
        }
        let materialization = MaterializationContract::skill_directory_v1();
        let kind = EffectKind::skill();
        effect_protocols.insert(kind.clone(), 1);
        materialization_contracts.insert(materialization.capability_key());
        let coordination = match resource.coordination {
            PluginEffectCoordination::Uninterrupted => CoordinationRequirement::Uninterrupted,
            PluginEffectCoordination::QuiesceBeforeMutation => {
                CoordinationRequirement::QuiesceBeforeMutation(coordination.clone())
            }
        };
        resources.push(FilesystemResourceTemplate {
            relative_path,
            materialization_format: format,
            materialization_contract: materialization.clone(),
            accepts: CapabilityRequirement {
                effect_protocols: BTreeMap::from([(kind, 1)]),
                materialization_contracts: BTreeSet::from([materialization.capability_key()]),
            },
            coordination,
            ownership_relative_path: None,
        });
    }
    if resources.is_empty() {
        return Ok(None);
    }
    Ok(Some(ConsumerDeclaration {
        consumer,
        adapter: ConsumerAdapterIdentity::parse("ora/agent-plugin")
            .map_err(|error| AgentEffectError::InvalidDeclaration(error.to_string()))?,
        capabilities: CapabilitySet {
            effect_protocols,
            materialization_contracts,
            coordination_contracts: BTreeSet::from([coordination.capability_key()]),
            readiness_contracts: BTreeSet::from(["ora/agent-target-ready.v1".to_string()]),
        },
        resources,
    }))
}

/// Establishes the plugin-owned safe-to-mutate barrier for the complete Resource set.
pub(crate) async fn coordinate(
    runtime: &PluginRuntime,
    target: &EffectTarget,
    plan: &CoordinationPlan,
) -> Result<AdapterReceipt, AgentEffectError> {
    invoke_coordination(runtime, EFFECT_COORDINATE_METHOD, target, plan).await
}

/// Reactivates a plugin Target after every Resource mutation has been verified.
pub(crate) async fn reactivate(
    runtime: &PluginRuntime,
    target: &EffectTarget,
    plan: &CoordinationPlan,
) -> Result<AdapterReceipt, AgentEffectError> {
    invoke_coordination(runtime, EFFECT_REACTIVATE_METHOD, target, plan).await
}

/// Obtains exact readiness proof for one immutable Target projection.
pub(crate) async fn verify_ready(
    runtime: &PluginRuntime,
    target: &EffectTarget,
    projection: &TargetProjection,
) -> Result<AdapterReceipt, AgentEffectError> {
    let params = serde_json::to_value(AgentEffectReadinessContext {
        target_id: target.identity.as_str().to_string(),
        generation: projection.generation.value(),
        consumer_revision_id: target.consumer_revision.as_str().to_string(),
        projection_digest: projection.digest.digest().as_str().to_string(),
    })
    .map_err(|error| AgentEffectError::Ipc(error.to_string()))?;
    let proof = runtime
        .invoke(EFFECT_VERIFY_READY_METHOD, params)
        .await
        .map_err(|error| AgentEffectError::Ipc(error.to_string()))?;
    Ok(AdapterReceipt {
        version: 1,
        payload: proof,
    })
}

/// Invokes either half of the coordination protocol with identical exact Resource identity.
async fn invoke_coordination<Runtime: AgentEffectRuntime>(
    runtime: &Runtime,
    method: &str,
    target: &EffectTarget,
    plan: &CoordinationPlan,
) -> Result<AdapterReceipt, AgentEffectError> {
    let params = serde_json::to_value(AgentEffectCoordinationContext {
        target_id: target.identity.as_str().to_string(),
        resource_ids: plan
            .resources
            .iter()
            .map(ora_effect::EffectResourceId::as_str)
            .map(str::to_string)
            .collect(),
    })
    .map_err(|error| AgentEffectError::Ipc(error.to_string()))?;
    let proof = runtime
        .invoke(method, params)
        .await
        .map_err(AgentEffectError::Ipc)?;
    Ok(AdapterReceipt {
        version: 1,
        payload: proof,
    })
}
