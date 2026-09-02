use ora_effect::{
    AdapterReceipt, ApplyReceipt, CleanupReceipt, EffectMutation, EffectOperation,
    EffectOperationId, EffectOperationIntent, EffectResource, ExactPlannedState,
    ExactPreviousState, Fingerprint, JsonMergeOperationPlan, LocalTimestamp, ManagedIdentity,
    NativeResourceIdentity, ObservedItem, OperationArtifact, OwnershipEvidence, PlannedMutation,
    PreparedOperation, ReconcileAttemptId, ResourceAdapter, ResourceAdapterError,
    ResourceObservation, VerificationReceipt, VersionedAdapterPlan, VersionedMaterializationInput,
    VersionedResourceDescriptor,
};
use ora_utils::jsonc::{
    nested_value, parse_value, remove_nested_object_entry, set_nested_object_entry,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

mod filesystem;

use filesystem::{ensure_contained_file_path, read_ledger, read_optional_bounded};

const LEDGER_SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

/// Secret-free sidecar proving which exact server keys Ora may update or remove.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOwnershipLedger {
    pub schema_version: u32,
    pub materialization_format: String,
    pub managed: BTreeMap<String, McpOwnershipRecord>,
}

/// One sidecar record tied to immutable ownership, revision, and rendered content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOwnershipRecord {
    pub managed_identity: ManagedIdentity,
    pub plugin_id: ora_domain::PluginId,
    pub server_name: String,
    pub configuration_revision: u64,
    pub fingerprint: Fingerprint,
}

/// Shared JSON/JSONC file adapter for OpenCode and Claude project MCP configuration.
#[derive(Clone, Copy, Debug, Default)]
pub struct McpConfigResourceAdapter;

impl ResourceAdapter for McpConfigResourceAdapter {
    fn prepare_operation(
        &self,
        resource: &EffectResource,
        attempt: ReconcileAttemptId,
        generation: ora_effect::Generation,
        sequence: u32,
        mutation: PlannedMutation,
        prepared_at: LocalTimestamp,
    ) -> Result<PreparedOperation, ResourceAdapterError> {
        prepare(
            resource,
            attempt,
            generation,
            sequence,
            mutation,
            prepared_at,
        )
        .map_err(ResourceAdapterError::new)
    }

    fn observe(
        &self,
        resource: &EffectResource,
    ) -> Result<ResourceObservation, ResourceAdapterError> {
        observe(resource).map_err(ResourceAdapterError::new)
    }

    fn apply(&self, operation: &EffectOperation) -> Result<ApplyReceipt, ResourceAdapterError> {
        apply(operation).map_err(ResourceAdapterError::new)
    }

    fn verify(
        &self,
        operation: &EffectOperation,
    ) -> Result<VerificationReceipt, ResourceAdapterError> {
        verify(operation).map_err(ResourceAdapterError::new)
    }

    fn cleanup(
        &self,
        artifact: &OperationArtifact,
    ) -> Result<CleanupReceipt, ResourceAdapterError> {
        Err(ResourceAdapterError::new(
            McpAdapterError::UnexpectedArtifact(artifact.identity.clone()),
        ))
    }
}

/// Converts a pure key mutation into a secret-free immutable operation journal.
fn prepare(
    resource: &EffectResource,
    attempt: ReconcileAttemptId,
    generation: ora_effect::Generation,
    sequence: u32,
    mutation: PlannedMutation,
    prepared_at: LocalTimestamp,
) -> Result<PreparedOperation, McpAdapterError> {
    let VersionedResourceDescriptor::FilesystemFileV1(descriptor) = &resource.descriptor else {
        return Err(McpAdapterError::WrongResourceDescriptor);
    };
    let operation_id = EffectOperationId::random();
    let operation_root = descriptor
        .workspace_root
        .join(".ora-mcp-operations")
        .join(operation_id.as_str());
    let native_identity = match &mutation.planned {
        ExactPlannedState::Present {
            native_identity, ..
        } => native_identity.clone(),
        ExactPlannedState::Missing => match &mutation.expected {
            ExactPreviousState::Present {
                native_identity, ..
            } => native_identity.clone(),
            ExactPreviousState::Missing => return Err(McpAdapterError::InvalidOperationState),
        },
    };
    let input = mutation
        .input
        .map(|input| match input {
            VersionedMaterializationInput::OpenCodeMcpConfigV1(input)
            | VersionedMaterializationInput::ClaudeMcpConfigV1(input) => Ok(input),
            VersionedMaterializationInput::SkillDirectoryV1(_) => Err(McpAdapterError::WrongInput),
        })
        .transpose()?;
    let plan = JsonMergeOperationPlan {
        workspace_root: descriptor.workspace_root.clone(),
        resource_relative_path: descriptor.relative_path.clone(),
        ownership_relative_path: descriptor.ownership_relative_path.clone(),
        configuration_path: descriptor
            .workspace_root
            .join(descriptor.relative_path.to_path_buf()),
        ownership_path: descriptor
            .workspace_root
            .join(descriptor.ownership_relative_path.to_path_buf()),
        staging_path: operation_root.join("staging"),
        backup_path: operation_root.join("backup"),
        mutation: mutation.mutation,
        managed_identity: mutation.managed_identity,
        native_identity,
        desired_effect: mutation.desired_effect,
        input,
    };
    Ok(PreparedOperation {
        operation: EffectOperation::prepare(
            operation_id,
            EffectOperationIntent {
                attempt,
                resource: resource.identity.clone(),
                generation,
                sequence,
                mutation: mutation.mutation,
                expected: mutation.expected,
                planned: mutation.planned,
                payload: VersionedAdapterPlan::JsonMergeV1(Box::new(plan)),
            },
            prepared_at,
        )?,
        artifacts: Vec::new(),
    })
}

/// Observes server entries and sidecar claims without turning either into ownership by itself.
fn observe(resource: &EffectResource) -> Result<ResourceObservation, McpAdapterError> {
    let VersionedResourceDescriptor::FilesystemFileV1(descriptor) = &resource.descriptor else {
        return Err(McpAdapterError::WrongResourceDescriptor);
    };
    let configuration_path = descriptor
        .workspace_root
        .join(descriptor.relative_path.to_path_buf());
    let ownership_path = descriptor
        .workspace_root
        .join(descriptor.ownership_relative_path.to_path_buf());
    ensure_contained_file_path(&descriptor.workspace_root, &configuration_path)?;
    ensure_contained_file_path(&descriptor.workspace_root, &ownership_path)?;
    let object_key = object_key(resource)?;
    let source = read_optional_bounded(&configuration_path)?.unwrap_or_else(|| "{}".to_string());
    let parsed = parse_value(&source)?;
    let ledger = read_ledger(&ownership_path, resource.format.as_str())?;
    let mut items = BTreeMap::new();
    if let Some(entries) = parsed.get(object_key).and_then(Value::as_object) {
        for (server_name, configuration) in entries {
            let native_identity = NativeResourceIdentity::parse(server_name.clone())?;
            let fingerprint = value_fingerprint(configuration)?;
            let ownership_evidence = ledger
                .managed
                .get(server_name)
                .map_or(OwnershipEvidence::NoOwnershipEvidence, |record| {
                    OwnershipEvidence::Claims(record.managed_identity.clone())
                });
            items.insert(
                native_identity.clone(),
                ObservedItem {
                    native_identity,
                    fingerprint,
                    ownership_evidence,
                },
            );
        }
    }
    for (server_name, record) in &ledger.managed {
        if !items
            .keys()
            .any(|identity| identity.as_str() == server_name)
        {
            let native_identity = NativeResourceIdentity::parse(format!("sidecar:{server_name}"))?;
            items.insert(
                native_identity.clone(),
                ObservedItem {
                    native_identity,
                    fingerprint: record.fingerprint.clone(),
                    ownership_evidence: OwnershipEvidence::Claims(record.managed_identity.clone()),
                },
            );
        }
    }
    if resource.format == ora_effect::MaterializationFormat::opencode_mcp_config_v1() {
        let jsonc_path = configuration_path.with_extension("jsonc");
        ensure_contained_file_path(&descriptor.workspace_root, &jsonc_path)?;
        if let Some(jsonc) = read_optional_bounded(&jsonc_path)? {
            let parsed_jsonc = parse_value(&jsonc)?;
            if let Some(entries) = parsed_jsonc.get(object_key).and_then(Value::as_object) {
                for (server_name, configuration) in entries {
                    let native_identity =
                        NativeResourceIdentity::parse(format!("jsonc:{server_name}"))?;
                    items.insert(
                        native_identity.clone(),
                        ObservedItem {
                            native_identity,
                            fingerprint: value_fingerprint(configuration)?,
                            ownership_evidence: OwnershipEvidence::NoOwnershipEvidence,
                        },
                    );
                }
            }
        }
    }
    let summary = serde_json::to_vec(&items)?;
    Ok(ResourceObservation {
        resource: resource.identity.clone(),
        items,
        fingerprint: Fingerprint::sha256(&summary),
    })
}

/// Applies one key merge only from the exact expected state or completes an idempotent replay.
fn apply(operation: &EffectOperation) -> Result<ApplyReceipt, McpAdapterError> {
    let VersionedAdapterPlan::JsonMergeV1(plan) = operation.payload() else {
        return Err(McpAdapterError::WrongOperationPlan);
    };
    ensure_paths(plan)?;
    ensure_contained_file_path(&plan.workspace_root, &plan.configuration_path)?;
    ensure_contained_file_path(&plan.workspace_root, &plan.ownership_path)?;
    if state_matches(operation, plan, StateToMatch::Planned)? {
        return Ok(apply_receipt(operation));
    }
    if !state_matches(operation, plan, StateToMatch::Expected)? {
        return Err(McpAdapterError::RecoveryRequired(
            operation.identity().clone(),
        ));
    }
    let parent = plan
        .configuration_path
        .parent()
        .ok_or(McpAdapterError::UnsafeOperationPath)?;
    fs::create_dir_all(parent).map_err(|source| McpAdapterError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    if let Some(parent) = plan.ownership_path.parent() {
        fs::create_dir_all(parent).map_err(|source| McpAdapterError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let source =
        read_optional_bounded(&plan.configuration_path)?.unwrap_or_else(|| "{}".to_string());
    let object_key = object_key_from_plan(plan)?;
    let mut ledger = read_ledger(&plan.ownership_path, materialization_format(plan)?)?;
    let updated = match operation.mutation() {
        EffectMutation::Create | EffectMutation::Update | EffectMutation::Replace => {
            let input = plan.input.as_ref().ok_or(McpAdapterError::MissingInput)?;
            ledger.managed.insert(
                input.server_name.clone(),
                McpOwnershipRecord {
                    managed_identity: plan.managed_identity.clone(),
                    plugin_id: input.plugin_id.clone(),
                    server_name: input.server_name.clone(),
                    configuration_revision: input.configuration_revision,
                    fingerprint: value_fingerprint(&input.configuration)?,
                },
            );
            set_nested_object_entry(
                &source,
                object_key,
                plan.native_identity.as_str(),
                &input.configuration,
            )?
        }
        EffectMutation::Delete => {
            ledger.managed.remove(plan.native_identity.as_str());
            remove_nested_object_entry(&source, object_key, plan.native_identity.as_str())?
        }
    };
    ora_utils::atomic::write(&plan.configuration_path, updated.as_bytes()).map_err(|source| {
        McpAdapterError::Io {
            path: plan.configuration_path.clone(),
            source,
        }
    })?;
    let ledger_json = serde_json::to_vec_pretty(&ledger)?;
    ora_utils::atomic::write(&plan.ownership_path, &ledger_json).map_err(|source| {
        McpAdapterError::Io {
            path: plan.ownership_path.clone(),
            source,
        }
    })?;
    Ok(apply_receipt(operation))
}

/// Proves the exact planned entry and ownership record after application.
fn verify(operation: &EffectOperation) -> Result<VerificationReceipt, McpAdapterError> {
    let VersionedAdapterPlan::JsonMergeV1(plan) = operation.payload() else {
        return Err(McpAdapterError::WrongOperationPlan);
    };
    if !state_matches(operation, plan, StateToMatch::Planned)? {
        return Err(McpAdapterError::VerificationFailed(
            operation.identity().clone(),
        ));
    }
    Ok(VerificationReceipt {
        operation: operation.identity().clone(),
        proof: AdapterReceipt {
            version: 1,
            payload: json!({"state":"planned"}),
        },
    })
}

#[derive(Clone, Copy)]
enum StateToMatch {
    Expected,
    Planned,
}

/// Compares both the shared-file entry and sidecar claim to one journal state.
fn state_matches(
    operation: &EffectOperation,
    plan: &JsonMergeOperationPlan,
    state: StateToMatch,
) -> Result<bool, McpAdapterError> {
    let source =
        read_optional_bounded(&plan.configuration_path)?.unwrap_or_else(|| "{}".to_string());
    let parsed = parse_value(&source)?;
    let current = nested_value(
        &parsed,
        object_key_from_plan(plan)?,
        plan.native_identity.as_str(),
    );
    let ledger = read_ledger(&plan.ownership_path, materialization_format(plan)?)?;
    let record = ledger.managed.get(plan.native_identity.as_str());
    let exact = match state {
        StateToMatch::Expected => operation.expected(),
        StateToMatch::Planned => match operation.planned() {
            ExactPlannedState::Missing => return Ok(current.is_none() && record.is_none()),
            ExactPlannedState::Present {
                fingerprint,
                managed_identity,
                ..
            } => {
                return Ok(current.map(value_fingerprint).transpose()?.as_ref()
                    == Some(fingerprint)
                    && record.is_some_and(|record| {
                        &record.managed_identity == managed_identity
                            && &record.fingerprint == fingerprint
                    }));
            }
        },
    };
    match exact {
        ExactPreviousState::Missing => Ok(current.is_none() && record.is_none()),
        ExactPreviousState::Present {
            fingerprint,
            managed_identity,
            ..
        } => Ok(
            current.map(value_fingerprint).transpose()?.as_ref() == Some(fingerprint)
                && record.is_some_and(|record| &record.managed_identity == managed_identity),
        ),
    }
}

/// Validates persisted paths against their immutable descriptor-derived joins.
fn ensure_paths(plan: &JsonMergeOperationPlan) -> Result<(), McpAdapterError> {
    if plan.configuration_path
        != plan
            .workspace_root
            .join(plan.resource_relative_path.to_path_buf())
        || plan.ownership_path
            != plan
                .workspace_root
                .join(plan.ownership_relative_path.to_path_buf())
    {
        return Err(McpAdapterError::UnsafeOperationPath);
    }
    Ok(())
}

fn object_key(resource: &EffectResource) -> Result<&'static str, McpAdapterError> {
    if resource.format == ora_effect::MaterializationFormat::opencode_mcp_config_v1() {
        Ok("mcp")
    } else if resource.format == ora_effect::MaterializationFormat::claude_mcp_config_v1() {
        Ok("mcpServers")
    } else {
        Err(McpAdapterError::UnsupportedFormat)
    }
}

fn object_key_from_plan(plan: &JsonMergeOperationPlan) -> Result<&'static str, McpAdapterError> {
    match plan.resource_relative_path.as_str() {
        ".opencode/opencode.json" => Ok("mcp"),
        ".mcp.json" => Ok("mcpServers"),
        _ => Err(McpAdapterError::UnsupportedFormat),
    }
}

fn materialization_format(plan: &JsonMergeOperationPlan) -> Result<&'static str, McpAdapterError> {
    match plan.resource_relative_path.as_str() {
        ".opencode/opencode.json" => Ok("ora/opencode-mcp-config.v1"),
        ".mcp.json" => Ok("ora/claude-mcp-config.v1"),
        _ => Err(McpAdapterError::UnsupportedFormat),
    }
}

fn value_fingerprint(value: &Value) -> Result<Fingerprint, McpAdapterError> {
    serde_json::to_vec(value)
        .map(|bytes| Fingerprint::sha256(&bytes))
        .map_err(Into::into)
}

fn apply_receipt(operation: &EffectOperation) -> ApplyReceipt {
    ApplyReceipt {
        operation: operation.identity().clone(),
        proof: AdapterReceipt {
            version: 1,
            payload: json!({"state":"applied"}),
        },
    }
}

/// Reports a shared-file observation or mutation that cannot be proven safe.
#[derive(Debug, Error)]
enum McpAdapterError {
    #[error("MCP Resource uses the wrong descriptor")]
    WrongResourceDescriptor,
    #[error("MCP operation uses the wrong adapter plan")]
    WrongOperationPlan,
    #[error("MCP mutation received a non-MCP input")]
    WrongInput,
    #[error("MCP mutation is missing its materialization input")]
    MissingInput,
    #[error("MCP operation state is contradictory")]
    InvalidOperationState,
    #[error("MCP materialization format is unsupported")]
    UnsupportedFormat,
    #[error("MCP operation path is outside its declared Resource")]
    UnsafeOperationPath,
    #[error("MCP ownership sidecar does not match the declared Resource")]
    OwnershipMismatch,
    #[error("MCP file exceeds the bounded read limit: {0}")]
    TooLarge(PathBuf),
    #[error("MCP operation {0} requires explicit recovery")]
    RecoveryRequired(EffectOperationId),
    #[error("MCP operation {0} did not reach its planned state")]
    VerificationFailed(EffectOperationId),
    #[error("MCP adapter received unexpected artifact {0}")]
    UnexpectedArtifact(ora_effect::ArtifactId),
    #[error("MCP filesystem operation failed at {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Jsonc(#[from] ora_utils::jsonc::JsoncEditError),
    #[error(transparent)]
    Identity(#[from] ora_effect::IdentityError),
    #[error(transparent)]
    Transition(#[from] ora_effect::OperationTransitionError),
}
