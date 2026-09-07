use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Identifies the executable kind of one workflow node.
///
/// Naming mirrors the backend column `workflow_node_runs.node_type`; the wire field that produces
/// this value is React Flow's `data.kind`. `Start`, `Agent`, `Condition`, and `Output` are
/// executable; the remaining variants are recognized so the parser can reject graphs that contain
/// them instead of silently skipping or downgrading them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeType {
    Start,
    Agent,
    Prompt,
    Condition,
    Tool,
    Output,
}

/// Returned when a wire node-type string has no registered variant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown node type: {0}")]
pub struct UnknownNodeType(pub String);

impl NodeType {
    /// Returns the stable string form persisted in `workflow_node_runs.node_type`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Agent => "agent",
            Self::Prompt => "prompt",
            Self::Condition => "condition",
            Self::Tool => "tool",
            Self::Output => "output",
        }
    }

    /// Returns whether v1 can execute this node type.
    ///
    /// Known-but-unsupported types stay recognized so workflow start rejects them explicitly
    /// rather than skipping or degrading them.
    pub fn supported(self) -> bool {
        matches!(
            self,
            Self::Start | Self::Agent | Self::Condition | Self::Output
        )
    }
}

impl FromStr for NodeType {
    type Err = UnknownNodeType;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "start" => Ok(Self::Start),
            "agent" => Ok(Self::Agent),
            "prompt" => Ok(Self::Prompt),
            "condition" => Ok(Self::Condition),
            "tool" => Ok(Self::Tool),
            "output" => Ok(Self::Output),
            _ => Err(UnknownNodeType(value.to_string())),
        }
    }
}

impl fmt::Display for NodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
