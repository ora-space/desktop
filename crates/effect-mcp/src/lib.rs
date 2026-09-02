//! Built-in MCP Effect rendering, planning, and shared configuration-file mutation.

mod adapter;
mod builtin;
mod planner;
#[cfg(test)]
mod planner_tests;
mod template;

pub use adapter::{McpConfigResourceAdapter, McpOwnershipLedger, McpOwnershipRecord};
pub use builtin::{BuiltinEffectPlanner, BuiltinResourceAdapter};
pub use planner::McpPlanner;
pub use template::{
    McpAgentFormat, McpTemplateError, configured_environment, materialized_configuration,
    resolve_template,
};
