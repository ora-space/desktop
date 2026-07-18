use crate::{InstalledRecord, PendingOperation, PluginError, UserEnablement};
use ora_plugin_protocol::{AgentProviderKey, ContentOwnerId, JsonSafeU64, OperationId, PluginId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use tokio::sync::{mpsc, oneshot};

pub const STATE_SCHEMA_VERSION_V1: u32 = 1;
pub const LAUNCH_GRANT_SCHEMA_VERSION_V1: u32 = 1;

/// The complete durable management snapshot; runtime generations never enter this model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginStateSnapshot {
    pub schema_version: u32,
    pub revision: JsonSafeU64,
    pub plugins: BTreeMap<PluginId, PluginStateRecord>,
    pub pending_operations: Vec<PendingOperation>,
    pub launch_grants: BTreeMap<PluginId, PluginLaunchGrant>,
}

impl PluginStateSnapshot {
    pub fn empty() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION_V1,
            revision: JsonSafeU64::new(0)
                .unwrap_or_else(|error| panic!("zero state revision must be valid: {error}")),
            plugins: BTreeMap::new(),
            pending_operations: Vec::new(),
            launch_grants: BTreeMap::new(),
        }
    }
}

/// Durable per-plugin facts with installation, user intent, and crash policy kept distinct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginStateRecord {
    pub user_enablement: UserEnablement,
    pub installation: InstalledRecord,
    pub crash_policy: CrashPolicy,
    pub enablement_epoch: JsonSafeU64,
}

/// A bounded crash window that remains blocked until explicit user reset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CrashPolicy {
    Normal {
        recent_crashes_unix_ms: Vec<JsonSafeU64>,
    },
    BlockedByCrashLoop {
        recent_crashes_unix_ms: Vec<JsonSafeU64>,
        blocked_at_unix_ms: JsonSafeU64,
    },
}

impl CrashPolicy {
    pub fn normal() -> Self {
        Self::Normal {
            recent_crashes_unix_ms: Vec::new(),
        }
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::BlockedByCrashLoop { .. })
    }
}

/// Host-owned launch authorization metadata; resolved secret values never enter state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginLaunchGrant {
    pub plugin_id: PluginId,
    pub content_owner: ContentOwnerId,
    pub schema_version: u32,
    pub revision: JsonSafeU64,
    pub environment: Vec<EnvironmentBinding>,
}

/// One explicitly named environment target and an unresolved Host reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentBinding {
    pub target: EnvironmentVariableName,
    pub value: LaunchValueReference,
}

/// A validated Windows environment variable name without a secret value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct EnvironmentVariableName(String);

impl EnvironmentVariableName {
    pub fn parse(value: impl Into<String>) -> Result<Self, LaunchGrantError> {
        let value = value.into();
        if value.is_empty() || value.len() > 32_767 || value.contains(['=', '\0']) {
            return Err(LaunchGrantError::InvalidEnvironmentVariableName);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for EnvironmentVariableName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// The only unresolved reference categories accepted by launch grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LaunchValueReference {
    HostConfiguration { key: String },
    Credential { key: String },
    DiscoveredExecutable { provider: AgentProviderKey },
    AuthorizedPath { path_id: String },
}

/// A launch-time value that preserves whether its memory must be treated as secret.
#[derive(Debug)]
pub enum ResolvedLaunchValue {
    Plain { value: OsString },
    Secret { value: SecretValue },
}

/// Secret launch bytes deliberately omit Clone, Debug content, and serialization.
pub struct SecretValue(OsString);

impl SecretValue {
    pub fn new(value: OsString) -> Self {
        Self(value)
    }

    pub fn expose_for_process(&self) -> &std::ffi::OsStr {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

/// Resolves only references previously approved in a launch grant.
pub trait LaunchValueResolver {
    /// Resolves a reference at process launch without persisting or logging its value.
    fn resolve(
        &self,
        reference: &LaunchValueReference,
    ) -> impl Future<Output = Result<ResolvedLaunchValue, LaunchGrantError>> + Send;
}

/// Launch grant validation and resolution failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchGrantError {
    #[error("environment variable name is invalid")]
    InvalidEnvironmentVariableName,
    #[error("launch grant schema or installation binding is invalid")]
    InvalidGrantBinding,
    #[error("launch value reference is unavailable")]
    ReferenceUnavailable,
}

/// Closed commands accepted by the single state writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateMutation {
    SetEnablement {
        plugin_id: PluginId,
        enablement: UserEnablement,
        advance_epoch: bool,
    },
    SetLaunchGrant {
        grant: PluginLaunchGrant,
    },
    RevokeLaunchGrant {
        plugin_id: PluginId,
    },
    AddPending {
        operation: PendingOperation,
    },
    ReplacePending {
        operation: PendingOperation,
    },
    AbortPending {
        operation_id: OperationId,
    },
    CompleteInstall {
        plugin_id: PluginId,
        record: InstalledRecord,
        operation_id: OperationId,
    },
    CompleteRemoval {
        plugin_id: PluginId,
        operation_id: OperationId,
    },
    SetCrashPolicy {
        plugin_id: PluginId,
        policy: CrashPolicy,
    },
    ResetCrashLoop {
        plugin_id: PluginId,
    },
}

/// Persists snapshots for the state actor without exposing a second writer.
pub trait StatePersistence: Send + 'static {
    /// Commits old backup and new primary before the actor publishes the candidate snapshot.
    fn commit(
        &mut self,
        previous: &PluginStateSnapshot,
        candidate: &PluginStateSnapshot,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;
}

/// Cloneable command handle; the spawned task is the only snapshot owner and persistence caller.
#[derive(Debug, Clone)]
pub struct StateStore {
    sender: mpsc::Sender<StateCommand>,
}

impl StateStore {
    /// Starts a single-writer actor from an already reconciled durable snapshot.
    pub fn start<P>(initial: PluginStateSnapshot, persistence: P, capacity: usize) -> Self
    where
        P: StatePersistence,
    {
        let (sender, receiver) = mpsc::channel(capacity.max(1));
        tokio::spawn(run_state_actor(initial, persistence, receiver));
        Self { sender }
    }

    /// Returns the current immutable snapshot from the single owner.
    pub async fn snapshot(&self) -> Result<PluginStateSnapshot, PluginError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(StateCommand::Snapshot { reply })
            .await
            .map_err(|_| actor_stopped())?;
        response.await.map_err(|_| actor_stopped())
    }

    /// Serializes one closed mutation, durable commit, and in-memory publication.
    pub async fn commit(
        &self,
        mutation: StateMutation,
    ) -> Result<PluginStateSnapshot, PluginError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(StateCommand::Commit { mutation, reply })
            .await
            .map_err(|_| actor_stopped())?;
        response.await.map_err(|_| actor_stopped())?
    }
}

enum StateCommand {
    Snapshot {
        reply: oneshot::Sender<PluginStateSnapshot>,
    },
    Commit {
        mutation: StateMutation,
        reply: oneshot::Sender<Result<PluginStateSnapshot, PluginError>>,
    },
}

/// Owns the only mutable snapshot and publishes it only after persistence succeeds.
async fn run_state_actor<P>(
    mut snapshot: PluginStateSnapshot,
    mut persistence: P,
    mut receiver: mpsc::Receiver<StateCommand>,
) where
    P: StatePersistence,
{
    while let Some(command) = receiver.recv().await {
        match command {
            StateCommand::Snapshot { reply } => {
                let _ = reply.send(snapshot.clone());
            }
            StateCommand::Commit { mutation, reply } => {
                let result = apply_mutation(&snapshot, mutation).and_then(|mut candidate| {
                    let next_revision = snapshot.revision.checked_increment().map_err(|error| {
                        PluginError::Internal {
                            message: error.to_string(),
                        }
                    })?;
                    candidate.revision = next_revision;
                    Ok(candidate)
                });
                let result = match result {
                    Ok(candidate) => match persistence.commit(&snapshot, &candidate).await {
                        Ok(()) => {
                            snapshot = candidate;
                            Ok(snapshot.clone())
                        }
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error),
                };
                let _ = reply.send(result);
            }
        }
    }
}

/// Applies one mutation to a private candidate snapshot before any durable write begins.
fn apply_mutation(
    current: &PluginStateSnapshot,
    mutation: StateMutation,
) -> Result<PluginStateSnapshot, PluginError> {
    let mut candidate = current.clone();
    match mutation {
        StateMutation::SetEnablement {
            plugin_id,
            enablement,
            advance_epoch,
        } => {
            let record =
                candidate
                    .plugins
                    .get_mut(&plugin_id)
                    .ok_or_else(|| PluginError::NotFound {
                        plugin_id: plugin_id.clone(),
                    })?;
            record.user_enablement = enablement;
            if enablement == UserEnablement::Enabled {
                record.crash_policy = CrashPolicy::normal();
            }
            if advance_epoch {
                record.enablement_epoch =
                    record
                        .enablement_epoch
                        .checked_increment()
                        .map_err(|error| PluginError::Internal {
                            message: error.to_string(),
                        })?;
            }
        }
        StateMutation::SetLaunchGrant { grant } => {
            validate_grant_binding(&candidate, &grant)?;
            candidate
                .launch_grants
                .insert(grant.plugin_id.clone(), grant);
        }
        StateMutation::RevokeLaunchGrant { plugin_id } => {
            candidate.launch_grants.remove(&plugin_id);
        }
        StateMutation::AddPending { operation } => {
            if candidate
                .pending_operations
                .iter()
                .any(|existing| pending_operation_id(existing) == pending_operation_id(&operation))
            {
                return Err(PluginError::Internal {
                    message: "duplicate pending operation id".to_string(),
                });
            }
            candidate.pending_operations.push(operation);
        }
        StateMutation::ReplacePending { operation } => {
            let operation_id = pending_operation_id(&operation).clone();
            let existing = candidate
                .pending_operations
                .iter_mut()
                .find(|existing| pending_operation_id(existing) == &operation_id)
                .ok_or_else(|| PluginError::Internal {
                    message: "pending operation does not exist".to_string(),
                })?;
            *existing = operation;
        }
        StateMutation::AbortPending { operation_id } => {
            remove_pending(&mut candidate, &operation_id)?;
        }
        StateMutation::CompleteInstall {
            plugin_id,
            record,
            operation_id,
        } => {
            candidate.plugins.insert(
                plugin_id,
                PluginStateRecord {
                    user_enablement: UserEnablement::Disabled,
                    installation: record,
                    crash_policy: CrashPolicy::normal(),
                    enablement_epoch: JsonSafeU64::new(0).map_err(|error| {
                        PluginError::Internal {
                            message: error.to_string(),
                        }
                    })?,
                },
            );
            remove_pending(&mut candidate, &operation_id)?;
        }
        StateMutation::CompleteRemoval {
            plugin_id,
            operation_id,
        } => {
            candidate.plugins.remove(&plugin_id);
            candidate.launch_grants.remove(&plugin_id);
            remove_pending(&mut candidate, &operation_id)?;
        }
        StateMutation::SetCrashPolicy { plugin_id, policy } => {
            candidate
                .plugins
                .get_mut(&plugin_id)
                .ok_or(PluginError::NotFound { plugin_id })?
                .crash_policy = policy;
        }
        StateMutation::ResetCrashLoop { plugin_id } => {
            candidate
                .plugins
                .get_mut(&plugin_id)
                .ok_or(PluginError::NotFound { plugin_id })?
                .crash_policy = CrashPolicy::normal();
        }
    }
    Ok(candidate)
}

/// Requires launch authorization to remain bound to the current content owner and schema.
fn validate_grant_binding(
    snapshot: &PluginStateSnapshot,
    grant: &PluginLaunchGrant,
) -> Result<(), PluginError> {
    let record = snapshot
        .plugins
        .get(&grant.plugin_id)
        .ok_or_else(|| PluginError::NotFound {
            plugin_id: grant.plugin_id.clone(),
        })?;
    if grant.schema_version != LAUNCH_GRANT_SCHEMA_VERSION_V1
        || grant.content_owner != record.installation.content_owner
        || grant.environment.len() > 128
    {
        return Err(PluginError::Internal {
            message: LaunchGrantError::InvalidGrantBinding.to_string(),
        });
    }
    if snapshot
        .launch_grants
        .get(&grant.plugin_id)
        .is_some_and(|current| grant.revision <= current.revision)
    {
        return Err(PluginError::Internal {
            message: "launch grant revision must advance".to_owned(),
        });
    }
    let mut targets = std::collections::BTreeSet::new();
    for binding in &grant.environment {
        if !targets.insert(binding.target.as_str().to_ascii_uppercase())
            || !valid_reference(&binding.value)
        {
            return Err(PluginError::Internal {
                message: "launch grant environment bindings are invalid".to_owned(),
            });
        }
    }
    Ok(())
}

/// Bounds attacker-controlled lookup keys before they reach configuration or credential stores.
fn valid_reference(reference: &LaunchValueReference) -> bool {
    let key = match reference {
        LaunchValueReference::HostConfiguration { key }
        | LaunchValueReference::Credential { key } => Some(key.as_str()),
        LaunchValueReference::AuthorizedPath { path_id } => Some(path_id.as_str()),
        LaunchValueReference::DiscoveredExecutable { .. } => None,
    };
    key.is_none_or(|key| !key.is_empty() && key.len() <= 1024 && !key.contains('\0'))
}

/// Returns the operation identity independent of the journal variant.
fn pending_operation_id(operation: &PendingOperation) -> &OperationId {
    match operation {
        PendingOperation::Install(install) => &install.operation_id,
        PendingOperation::Remove(removal) => &removal.operation_id,
    }
}

/// Removes exactly one completed journal entry and rejects missing or duplicated facts.
fn remove_pending(
    snapshot: &mut PluginStateSnapshot,
    operation_id: &OperationId,
) -> Result<(), PluginError> {
    let before = snapshot.pending_operations.len();
    snapshot
        .pending_operations
        .retain(|operation| pending_operation_id(operation) != operation_id);
    if snapshot.pending_operations.len() + 1 != before {
        return Err(PluginError::Internal {
            message: "completed pending operation was missing or duplicated".to_string(),
        });
    }
    Ok(())
}

/// Produces the stable boundary failure when the write-capable actor has exited.
fn actor_stopped() -> PluginError {
    PluginError::Internal {
        message: "state actor stopped".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginStateSnapshot, StateMutation, StatePersistence, StateStore};
    use crate::PluginError;
    use pretty_assertions::assert_eq;
    use std::future::ready;

    struct MemoryPersistence;

    impl StatePersistence for MemoryPersistence {
        fn commit(
            &mut self,
            _previous: &PluginStateSnapshot,
            _candidate: &PluginStateSnapshot,
        ) -> impl std::future::Future<Output = Result<(), PluginError>> + Send {
            ready(Ok(()))
        }
    }

    /// The actor advances revision only after the injected persistence commit succeeds.
    #[tokio::test]
    async fn serializes_state_commits() {
        let store = StateStore::start(PluginStateSnapshot::empty(), MemoryPersistence, 8);
        let plugin_id = ora_plugin_protocol::PluginId::parse("ora.missing")
            .unwrap_or_else(|error| panic!("expected plugin id: {error}"));
        let result = store
            .commit(StateMutation::RevokeLaunchGrant { plugin_id })
            .await
            .unwrap_or_else(|error| panic!("expected idempotent grant revoke: {error}"));
        assert_eq!(result.revision.get(), 1);
        assert_eq!(store.snapshot().await, Ok(result));
    }
}
