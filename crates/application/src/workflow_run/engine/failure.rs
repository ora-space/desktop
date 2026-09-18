use super::node_type::NodeType;
use super::ports::FileChange;
use serde::{Deserialize, Serialize};

/// Mechanical classification of why a node run failed. Drives the "will resuming the same
/// snapshot probably work" prediction shown in the UI; never inferred by an AI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeFailureKind {
    MissingAgentRef,
    WorkflowModelNotFound,
    MissingAgentConfig,
    InvalidRunPayload,
    PromptTemplate,
    StructuredOutput,
    MissingSkillMaterialization,
    SessionEndedWithoutStopReason,
    SessionBindingRejected,
    BaselinePersist,
    Repository,
    Session,
    AgentRefusal,
    UnknownStopReason,
    InterruptedByRestart,
    MultipleOutputs,
    ConditionEvaluation,
}

impl NodeFailureKind {
    /// `true` = environment/transient: re-running the same snapshot is a sensible first move.
    /// `false` = the failure is caused by the workflow definition or the agent's behaviour, so the
    /// same snapshot will most likely fail again (the UI still allows resuming; it only warns).
    pub const fn resumable(self) -> bool {
        match self {
            Self::WorkflowModelNotFound
            | Self::MissingAgentConfig
            | Self::Session
            | Self::SessionEndedWithoutStopReason
            | Self::SessionBindingRejected
            | Self::InterruptedByRestart
            | Self::Repository
            | Self::BaselinePersist => true,
            Self::StructuredOutput
            | Self::AgentRefusal
            | Self::PromptTemplate
            | Self::MissingAgentRef
            | Self::MissingSkillMaterialization
            | Self::InvalidRunPayload
            | Self::UnknownStopReason
            | Self::MultipleOutputs
            | Self::ConditionEvaluation => false,
        }
    }

    /// `true` = the failure came from the agent's own output/behaviour, so telling the next
    /// attempt what went wrong can change the outcome. Environment, definition and engine
    /// failures are `false`: repeating them to the agent cannot help.
    pub const fn inject_into_prompt(self) -> bool {
        match self {
            Self::StructuredOutput
            | Self::AgentRefusal
            | Self::UnknownStopReason
            | Self::MultipleOutputs => true,
            Self::MissingAgentRef
            | Self::WorkflowModelNotFound
            | Self::MissingAgentConfig
            | Self::InvalidRunPayload
            | Self::PromptTemplate
            | Self::MissingSkillMaterialization
            | Self::SessionEndedWithoutStopReason
            | Self::SessionBindingRejected
            | Self::BaselinePersist
            | Self::Repository
            | Self::Session
            | Self::InterruptedByRestart
            | Self::ConditionEvaluation => false,
        }
    }

    /// The serialized snake_case name (same string serde produces); used as a translation key.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingAgentRef => "missing_agent_ref",
            Self::WorkflowModelNotFound => "workflow_model_not_found",
            Self::MissingAgentConfig => "missing_agent_config",
            Self::InvalidRunPayload => "invalid_run_payload",
            Self::PromptTemplate => "prompt_template",
            Self::StructuredOutput => "structured_output",
            Self::MissingSkillMaterialization => "missing_skill_materialization",
            Self::SessionEndedWithoutStopReason => "session_ended_without_stop_reason",
            Self::SessionBindingRejected => "session_binding_rejected",
            Self::BaselinePersist => "baseline_persist",
            Self::Repository => "repository",
            Self::Session => "session",
            Self::AgentRefusal => "agent_refusal",
            Self::UnknownStopReason => "unknown_stop_reason",
            Self::InterruptedByRestart => "interrupted_by_restart",
            Self::MultipleOutputs => "multiple_outputs",
            Self::ConditionEvaluation => "condition_evaluation",
        }
    }
}

/// What is persisted under `payload.error_detail` of a failed node run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeFailureDetail {
    pub kind: NodeFailureKind,
    pub message: String,
    /// `std::error::Error::source()` chain of the originating error, outermost first; empty
    /// when the failure was raised by the engine itself.
    pub source_chain: Vec<String>,
    /// 1 for the first run of this node in this workflow run; +1 per soft-deleted predecessor
    /// row (same `run_id` + `node_id`, `is_deleted = 1`). Filled in by the repository.
    pub attempt: u32,
    pub resumable: bool,
    /// Whether a same-version rerun of this node injects this failure into the agent prompt;
    /// mirrors `NodeFailureKind::inject_into_prompt`.
    pub injects_previous_failure: bool,
    /// Unix millis, the repository's `now`.
    pub recorded_at: i64,
}

/// Failure input from the caller: the repository adds `attempt` and `recorded_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFailure {
    pub kind: NodeFailureKind,
    pub message: String,
    pub source_chain: Vec<String>,
    pub output: Option<String>,
    /// Worktree files this node touched before failing; empty when none could be recorded.
    pub file_changes: Vec<FileChange>,
}

impl NodeFailure {
    /// Classifies a swift/composite runtime failure string into a structured `NodeFailure`.
    ///
    /// Lives here rather than in the scheduling core so `engine.rs` can stay free of
    /// `NodeType::` literals (ADR "node runtime orchestration" D1).
    pub(super) fn from_runtime(node_type: NodeType, message: String) -> Self {
        let kind = match node_type {
            NodeType::Condition => NodeFailureKind::ConditionEvaluation,
            NodeType::Output if message.starts_with("multiple active output nodes:") => {
                NodeFailureKind::MultipleOutputs
            }
            NodeType::Start
            | NodeType::Output
            | NodeType::Agent
            | NodeType::Prompt
            | NodeType::Tool
            | NodeType::Iteration
            | NodeType::Loop => NodeFailureKind::InvalidRunPayload,
        };
        Self::new(kind, message)
    }

    /// Builds a failure with an empty source chain, no retained output, and no file changes.
    pub fn new(kind: NodeFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source_chain: Vec::new(),
            output: None,
            file_changes: Vec::new(),
        }
    }

    /// Attaches any raw node output worth retaining alongside the failure.
    pub fn with_output(self, output: Option<String>) -> Self {
        Self { output, ..self }
    }

    /// Replaces the source chain collected from the originating error.
    pub fn with_source_chain(self, source_chain: Vec<String>) -> Self {
        Self {
            source_chain,
            ..self
        }
    }

    /// Attaches the worktree files this node touched before it failed.
    pub fn with_file_changes(self, file_changes: Vec<FileChange>) -> Self {
        Self {
            file_changes,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NodeFailureDetail, NodeFailureKind};
    use pretty_assertions::assert_eq;

    #[test]
    fn resumable_matches_the_environment_versus_definition_split() {
        assert_eq!(NodeFailureKind::MissingAgentRef.resumable(), false);
        assert_eq!(NodeFailureKind::WorkflowModelNotFound.resumable(), true);
        assert_eq!(NodeFailureKind::MissingAgentConfig.resumable(), true);
        assert_eq!(NodeFailureKind::InvalidRunPayload.resumable(), false);
        assert_eq!(NodeFailureKind::PromptTemplate.resumable(), false);
        assert_eq!(NodeFailureKind::StructuredOutput.resumable(), false);
        assert_eq!(
            NodeFailureKind::MissingSkillMaterialization.resumable(),
            false
        );
        assert_eq!(
            NodeFailureKind::SessionEndedWithoutStopReason.resumable(),
            true
        );
        assert_eq!(NodeFailureKind::SessionBindingRejected.resumable(), true);
        assert_eq!(NodeFailureKind::BaselinePersist.resumable(), true);
        assert_eq!(NodeFailureKind::Repository.resumable(), true);
        assert_eq!(NodeFailureKind::Session.resumable(), true);
        assert_eq!(NodeFailureKind::AgentRefusal.resumable(), false);
        assert_eq!(NodeFailureKind::UnknownStopReason.resumable(), false);
        assert_eq!(NodeFailureKind::InterruptedByRestart.resumable(), true);
        assert_eq!(NodeFailureKind::MultipleOutputs.resumable(), false);
        assert_eq!(NodeFailureKind::ConditionEvaluation.resumable(), false);
    }

    #[test]
    fn inject_into_prompt_marks_only_agent_behaviour_failures() {
        assert_eq!(NodeFailureKind::StructuredOutput.inject_into_prompt(), true);
        assert_eq!(NodeFailureKind::AgentRefusal.inject_into_prompt(), true);
        assert_eq!(
            NodeFailureKind::UnknownStopReason.inject_into_prompt(),
            true
        );
        assert_eq!(NodeFailureKind::MultipleOutputs.inject_into_prompt(), true);
        assert_eq!(NodeFailureKind::MissingAgentRef.inject_into_prompt(), false);
        assert_eq!(
            NodeFailureKind::WorkflowModelNotFound.inject_into_prompt(),
            false
        );
        assert_eq!(
            NodeFailureKind::MissingAgentConfig.inject_into_prompt(),
            false
        );
        assert_eq!(
            NodeFailureKind::InvalidRunPayload.inject_into_prompt(),
            false
        );
        assert_eq!(NodeFailureKind::PromptTemplate.inject_into_prompt(), false);
        assert_eq!(
            NodeFailureKind::MissingSkillMaterialization.inject_into_prompt(),
            false
        );
        assert_eq!(
            NodeFailureKind::SessionEndedWithoutStopReason.inject_into_prompt(),
            false
        );
        assert_eq!(
            NodeFailureKind::SessionBindingRejected.inject_into_prompt(),
            false
        );
        assert_eq!(NodeFailureKind::BaselinePersist.inject_into_prompt(), false);
        assert_eq!(NodeFailureKind::Repository.inject_into_prompt(), false);
        assert_eq!(NodeFailureKind::Session.inject_into_prompt(), false);
        assert_eq!(
            NodeFailureKind::InterruptedByRestart.inject_into_prompt(),
            false
        );
        assert_eq!(
            NodeFailureKind::ConditionEvaluation.inject_into_prompt(),
            false
        );
    }

    #[test]
    fn as_str_matches_serde_snake_case_for_every_kind() {
        assert_eq!(
            serde_json::to_string(&NodeFailureKind::InterruptedByRestart).unwrap(),
            "\"interrupted_by_restart\""
        );
        for kind in [
            NodeFailureKind::MissingAgentRef,
            NodeFailureKind::WorkflowModelNotFound,
            NodeFailureKind::MissingAgentConfig,
            NodeFailureKind::InvalidRunPayload,
            NodeFailureKind::PromptTemplate,
            NodeFailureKind::StructuredOutput,
            NodeFailureKind::MissingSkillMaterialization,
            NodeFailureKind::SessionEndedWithoutStopReason,
            NodeFailureKind::SessionBindingRejected,
            NodeFailureKind::BaselinePersist,
            NodeFailureKind::Repository,
            NodeFailureKind::Session,
            NodeFailureKind::AgentRefusal,
            NodeFailureKind::UnknownStopReason,
            NodeFailureKind::InterruptedByRestart,
            NodeFailureKind::MultipleOutputs,
            NodeFailureKind::ConditionEvaluation,
        ] {
            let name = kind.as_str();
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn node_failure_detail_round_trips_through_json() {
        let detail = NodeFailureDetail {
            kind: NodeFailureKind::InterruptedByRestart,
            message: r#"{"reason":"interrupted_by_restart"}"#.to_string(),
            source_chain: vec!["outer".to_string(), "inner".to_string()],
            attempt: 2,
            resumable: true,
            injects_previous_failure: false,
            recorded_at: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&detail).unwrap();
        let parsed: NodeFailureDetail = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, detail);
        assert!(json.contains(
            "\"resumable\":true,\"injects_previous_failure\":false,\"recorded_at\":1700000000000"
        ));
    }
}
