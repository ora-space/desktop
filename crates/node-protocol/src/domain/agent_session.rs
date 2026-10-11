//! Agent session execution of an IssueRun: its input, its only terminal result, and the settled
//! history records it streams as non-terminal Thread events.

use crate::{
    ExecutionId, MessageValidationError, ModelBindingId, NodeId, NodeRuntimeIdentity, PluginId,
    PluginVersion, PriorRevision, TurnId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Largest encoded Thread record; a larger record is sent truncated and kept whole only in the
/// session history, so one huge tool output never crowds control frames or exceeds Cloud's entry
/// limit.
pub const MAX_THREAD_RECORD_BYTES: usize = 256 * 1024;
/// Largest total text of one user turn, matching Cloud's public message limit.
pub const MAX_USER_TURN_TEXT_BYTES: usize = 64 * 1024;

/// Commit identity exported to the Agent's process tree as `GIT_AUTHOR_*` and `GIT_COMMITTER_*`.
/// It is never written into a Git configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GitIdentity {
    pub name: String,
    pub email: String,
}

impl GitIdentity {
    /// Mirrors Cloud's identity rules so a value that would corrupt a Git signature line, such as
    /// one containing a newline or angle bracket, never reaches the environment.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        let name_ok = (1..=200).contains(&self.name.chars().count())
            && !self.name.trim().is_empty()
            && !self
                .name
                .chars()
                .any(|c| c.is_control() || matches!(c, '<' | '>'));
        let email_ok = self.email.len() <= 254
            && self
                .email
                .split_once('@')
                .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
            && !self
                .email
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>'));
        if name_ok && email_ok {
            return Ok(());
        }
        Err(MessageValidationError::InvalidGitIdentity)
    }
}

/// One block of user content; only text in the first version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentBlock {
    Text { text: String },
}

/// One user turn handed to the Agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UserTurn {
    pub turn_id: TurnId,
    pub content: Vec<ContentBlock>,
}

impl UserTurn {
    /// Requires an identity and non-empty, bounded text.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.turn_id.is_empty() {
            return Err(MessageValidationError::EmptyField { field: "turn_id" });
        }
        let mut total = 0usize;
        for block in &self.content {
            match block {
                ContentBlock::Text { text } => total += text.len(),
            }
        }
        if self.content.is_empty() || total == 0 || total > MAX_USER_TURN_TEXT_BYTES {
            return Err(MessageValidationError::InvalidUserTurn);
        }
        Ok(())
    }
}

/// Starts the agent plugin in the checkout of a previous clone execution and sends the first turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionSpec {
    pub node_id: NodeId,
    pub agent_plugin_id: PluginId,
    pub agent_plugin_version: PluginVersion,
    /// A successful clone execution on this Node; the Node resolves the checkout from its ledger,
    /// so no Node-local path crosses the protocol.
    pub checkout_execution_id: ExecutionId,
    /// Only a binding reference crosses the control protocol; model credentials never do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_binding_id: Option<ModelBindingId>,
    pub git_identity: GitIdentity,
    pub initial_turn: UserTurn,
    /// The Revision this session resumes; the Node restores it into the checkout before the agent
    /// starts, and only a Node advertising `revision_restore` is sent such a session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_revision: Option<PriorRevision>,
}

impl AgentSessionSpec {
    /// Rejects inputs the Node could not start deterministically.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.node_id.is_empty() {
            return Err(MessageValidationError::EmptyField { field: "node_id" });
        }
        self.agent_plugin_id.validate()?;
        if self.agent_plugin_version.as_str().trim().is_empty() {
            return Err(MessageValidationError::EmptyField {
                field: "agent_plugin_version",
            });
        }
        if self.checkout_execution_id.is_empty() {
            return Err(MessageValidationError::EmptyField {
                field: "checkout_execution_id",
            });
        }
        self.git_identity.validate()?;
        if self
            .model_binding_id
            .as_ref()
            .is_some_and(ModelBindingId::is_empty)
        {
            return Err(MessageValidationError::EmptyField {
                field: "model_binding_id",
            });
        }
        self.initial_turn.validate()?;
        if let Some(prior) = &self.prior_revision {
            prior.validate()?;
        }
        Ok(())
    }
}

/// Why a session execution ended.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionEndReason {
    UserEnded,
    IdleTimeout,
    Cancelled,
    /// The agent plugin was unavailable or its runtime gave up; `detail` names the cause.
    AgentFailed,
    /// The Node restarted; the session history was sealed and the Workspace kept.
    Interrupted,
}

/// The only terminal result of a session execution, sent after its last Thread event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionEnded {
    pub node: NodeRuntimeIdentity,
    pub reason: AgentSessionEndReason,
    /// Bounded snake_case code such as `agent_plugin_unavailable`; never raw Agent output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Session execution's disjoint wire tag cannot be decoded as another business's result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum AgentSessionResult {
    AgentSessionEnded(AgentSessionEnded),
}

impl AgentSessionResult {
    /// Keeps the terminal detail a code so no Agent output leaks into Cloud's run status.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        let Self::AgentSessionEnded(ended) = self;
        ended
            .node
            .validate()
            .map_err(|field| MessageValidationError::EmptyField { field })?;
        if let Some(detail) = &ended.detail
            && !is_bounded_code(detail)
        {
            return Err(MessageValidationError::InvalidDetailCode);
        }
        Ok(())
    }

    /// Preserves the original incarnation when a restarted Node reports stored evidence.
    pub(crate) fn node(&self) -> &NodeRuntimeIdentity {
        let Self::AgentSessionEnded(ended) = self;
        &ended.node
    }
}

/// Accepts short snake_case codes only.
fn is_bounded_code(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

/// One settled `ora-history` record, sent as a non-terminal event of a session execution. The
/// protocol treats the record as an opaque JSON object: its schema belongs to `ora-history`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadEvent {
    /// Set when the record belongs to a user turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub record: Map<String, Value>,
    /// The Node replaced an oversized record with a shortened one; the history keeps the original.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

impl ThreadEvent {
    /// Bounds the encoded record so every event fits Cloud's Thread entry limit.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.turn_id.as_ref().is_some_and(TurnId::is_empty) {
            return Err(MessageValidationError::EmptyField { field: "turn_id" });
        }
        let encoded = serde_json::to_vec(&self.record)
            .map_err(|_| MessageValidationError::ThreadRecordTooLarge)?;
        if encoded.len() > MAX_THREAD_RECORD_BYTES {
            return Err(MessageValidationError::ThreadRecordTooLarge);
        }
        Ok(())
    }
}
