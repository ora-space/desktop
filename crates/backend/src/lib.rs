mod agent;
mod agent_runtime;
mod aggregate_lifecycle;
mod bootstrap;
mod clock;
mod error;
mod git_cleanup;
mod identity;
mod project;
mod session;
mod skill;
mod task;

pub use agent_runtime::SessionEventStream;
pub use bootstrap::{Backend, BackendBootstrapError, BackendPaths};
pub use error::{BackendError, BackendErrorKind};
