//! The workflow run execution engine.
//!
//! Owns frozen-graph parsing and topology, the engine persistence port, and the run engine.
//! Agent-node execution is delegated through the `NodeExecutor` port (implemented in the backend)
//! and persistence through `WorkflowRunEngineRepository` (implemented in the database layer).

// The design places the run engine in `engine/engine.rs`, so the nested module name matches the
// containing directory on purpose.
mod agent_config;
mod branch_projection;
mod condition;
#[allow(clippy::module_inception)]
mod engine;
mod failure;
mod graph;
mod handlers;
mod iteration;
mod loop_bindings;
mod loop_config;
mod loop_graph;
mod loop_round;
mod node_executor;
mod node_runtime;
mod node_type;
mod ports;
mod region;
mod skill_delivery;
mod snapshot_switch;
mod start_input;
mod structured_output;
mod variable_pool;
mod variable_template;
mod variable_value;

pub use agent_config::AgentMcp;
pub use engine::WorkflowRunEngine;
pub use failure::{NodeFailure, NodeFailureDetail, NodeFailureKind};
pub use graph::{
    AgentConfig, AgentExecutor, AgentOutputContract, AgentSkill, GraphError,
    StructuredTextExposure, WorkflowGraph, WorkflowGraphNode,
};
pub use handlers::WorkflowRunControlHandler;
pub use iteration::{
    CompositeRegion, IterationConfig, IterationErrorStrategy, IterationLedger, RoundOutcome,
};
pub use loop_config::{LoopConfig, LoopInitialValue, LoopVariable};
pub use loop_round::{LoopRoundDecision, LoopRoundError, LoopRoundExecutionState};
pub use node_executor::{EngineError, NodeExecutor, WorkflowRunCallback, WorkflowValidationError};
pub use node_type::{NodeType, UnknownNodeType};
pub use ports::{
    AdvanceWorkflowRunResult, BindWorkflowNodeSessionResult, CancelWorkflowRunResult,
    ExecutionContext, FailurePropagation, FileChange, IterationRoundContinuation, LoopRoundAdvance,
    LoopRoundToStart, NoRunInvalidations, NodeRunToStart, RestartWorkflowRunResult,
    ResumeWorkflowRunResult, StartPrerequisitesError, StartWorkflowRunResult,
    UpdateWorkflowRunInputResult, WorkflowNodeRunIdGenerator, WorkflowRunEngineRepository,
    WorkflowRunInvalidationPublisher, WorkflowRunWorkspaceInitializer,
};
pub use region::{resume_clear_node_ids, resume_unit_owner_id, running_row_blocks_resume};
pub use skill_delivery::{
    AgentSkillDelivery, AgentSkillDeliveryError, AgentSkillDeliveryProvider,
    MaterializedSkillBinding, SkillDiscoveryRoots, SkillMaterializationReceipt, WorkflowRunPayload,
    WorkflowRunPayloadError,
};
pub use snapshot_switch::{SnapshotIncompatibility, SnapshotSwitchPlan, plan_snapshot_switch};
pub use structured_output::{StructuredOutputError, extract_json_object, validate_against_schema};
pub use variable_pool::{WorkflowVariablePool, WorkflowVariablePoolError};
pub use variable_template::{VariableTemplateError, render_variable_template};

#[cfg(test)]
mod d2_tests;
#[cfg(test)]
mod resume_tests;
#[cfg(test)]
mod tests;
