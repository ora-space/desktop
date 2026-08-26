pub mod app_event;

pub mod agent;
pub mod agent_import;
pub mod developer_mode;
pub mod effect;
pub mod error;
pub mod file_system;
pub mod git;
pub mod plugin;
pub mod project;
pub mod proxy;
pub mod runtime_log_level;
pub mod session;
pub mod skill;
pub mod skill_import;
pub mod spec;
pub mod task;
pub mod task_diff;
pub mod workflow;
pub mod workflow_run;
pub mod workspace;
pub use agent_import::*;
pub use app_event::*;

pub use agent::*;
pub use developer_mode::*;
pub use effect::*;
pub use error::*;
pub use file_system::*;
pub use git::*;
pub use plugin::*;
pub use project::*;
pub use proxy::*;
pub use runtime_log_level::*;
pub use session::*;
pub use skill::*;
pub use skill_import::*;
pub use spec::*;
use std::path::Path;
pub use task::*;
pub use task_diff::*;
use ts_rs::{Config, ExportError};
pub use workflow::*;
pub use workflow_run::*;
pub use workspace::*;

/// Exports every contract DTO family into the shared TypeScript package for frontend consumers.
///
/// Each module owns the exhaustive list of its own TypeScript bindings, so adding a new contract
/// type only requires registering it next to its definition rather than in this aggregation point.
pub fn export_typescript_bindings_to(
    output_directory: impl AsRef<Path>,
) -> Result<(), ExportError> {
    let config = Config::new().with_out_dir(output_directory.as_ref());
    agent_import::export(&config)?;

    app_event::export(&config)?;
    agent::export(&config)?;
    developer_mode::export(&config)?;
    effect::export(&config)?;
    error::export(&config)?;
    file_system::export(&config)?;
    git::export(&config)?;
    plugin::export(&config)?;
    project::export(&config)?;
    proxy::export(&config)?;
    runtime_log_level::export(&config)?;
    session::export(&config)?;
    skill::export(&config)?;
    skill_import::export(&config)?;
    spec::export(&config)?;
    task::export(&config)?;
    task_diff::export(&config)?;
    workflow::export(&config)?;
    workflow_run::export(&config)?;
    workspace::export(&config)?;

    Ok(())
}
