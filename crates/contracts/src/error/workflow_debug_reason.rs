use derive_more::Display;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Stable details for run-local debug refusals. Display supplies guidance for older clients.
#[derive(Clone, Debug, Deserialize, Display, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "code", rename_all = "snake_case")]
#[ts(export_to = "error.ts")]
pub enum WorkflowDebugReason {
    #[display(
        "The input for this attempt was not recorded. Select an attempt with recorded input."
    )]
    InputNotRecorded,
    #[display("Select a current attempt or a reachable position in this run.")]
    InvalidTarget,
    #[display("Node {node} in {scope} is not ready for its first dispatch. Check its prerequisites ({prerequisites}).", prerequisites = prerequisites.join(", "))]
    PrerequisitesNotReady {
        node: String,
        scope: String,
        prerequisites: Vec<String>,
    },
    #[display("Select a live round of Loop {node}.")]
    LiveRoundRequired { node: String },
    #[display("Round {round} of Loop {node} has already exited. Select a live round.")]
    RoundExited { node: String, round: u32 },
    #[display("Supply {field} for foreach member {node}.")]
    PositionFieldRequired { field: String, node: String },
    #[display(
        "The frozen input for {node} is unavailable. Select an execution with recorded input."
    )]
    ContainerInputMissing { node: String },
    #[display(
        "Index {index} does not identify the bound, unsettled item of {node}. Select its current item."
    )]
    ItemOutOfRange { node: String, index: u32 },
    #[display(
        "Attempt {attempt} used revision {revision}, before baseline {baseline}. Select an attempt from the current snapshot."
    )]
    BeforeBaseline {
        attempt: String,
        revision: u32,
        baseline: u32,
    },
    #[display(
        "LoopExit cleanup is still in progress. Wait for it to finish before previewing this edit."
    )]
    LoopExitCleanupPending,
    #[display(
        "The Loop round ended without confirmation that its cancelled siblings stopped. Use ordinary failure recovery to resume the run."
    )]
    LoopExitRecoveryRequired,
    #[display("Check that {variable} is declared and has a recorded value at this position.")]
    VariableUnavailable { variable: String },
    #[display("Check the field path in variable reference {variable}.")]
    InvalidVariablePath { variable: String },
    #[display("Check the type of {variable} and its comparison value for operator {operator}.")]
    ComparisonTypeMismatch { variable: String, operator: String },
    #[display("Enter a text value.")]
    ExpectedText,
    #[display("Select a variable using a list of field names.")]
    ExpectedSelector,
    #[display("Enter a non-negative integer.")]
    ExpectedInteger,
    #[display("Supply a configuration object.")]
    ExpectedObject,
    #[display("Choose an editable configuration field.")]
    FieldNotEditable,
    #[display("Select a field that exists in this configuration.")]
    FieldMissing,
    #[display("Change at least one configuration field.")]
    EmptyPatch,
    #[display("Select an array variable declared at the root.")]
    RootArrayRequired,
    #[display("Assign an array to {variable} at the selected position.")]
    ExpectedArray { variable: String },
    #[display("Select an array for {variable} with at most {limit} items.")]
    ArrayExceedsLimit { variable: String, limit: u32 },
}
