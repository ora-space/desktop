//! The two durable Cloud commands that can be delivered to an Agent session.
use crate::*;

/// A validated command retaining its original identity through retries and replies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentCommand {
    Submit(SubmitUserTurnMessage),
    End(EndSessionMessage),
}

impl AgentCommand {
    /// The identity Node deduplicates and Cloud records as delivered.
    pub fn id(&self) -> &CommandId {
        match self {
            Self::Submit(v) => &v.payload.command_id,
            Self::End(v) => &v.payload.command_id,
        }
    }
    /// The execution receiving this command.
    pub fn execution(&self) -> &ExecutionId {
        match self {
            Self::Submit(v) => &v.execution_id,
            Self::End(v) => &v.execution_id,
        }
    }
    /// The registered Node-local operation, not the Cloud IssueRun ID.
    pub fn operation(&self) -> &OperationId {
        match self {
            Self::Submit(v) => &v.operation_id,
            Self::End(v) => &v.operation_id,
        }
    }
    /// Rebuilds the exact wire message for an initial send or lost-reply retry.
    pub fn message(&self) -> ControllerToNodeMessage {
        match self {
            Self::Submit(v) => ControllerToNodeMessage::SubmitUserTurn(v.clone()),
            Self::End(v) => ControllerToNodeMessage::EndSession(v.clone()),
        }
    }
}
