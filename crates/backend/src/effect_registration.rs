//! Converges Scope/Consumer pairings independently of plugin and Workspace creation order.

use crate::error::BackendError;
use ora_db::SqliteEffectRepository;
use ora_domain::Workspace;
use ora_effect::ConsumerDeclaration;

/// Re-declares every current Consumer against the complete current Workspace snapshot.
///
/// Declaration persistence is idempotent. Re-running this on every worker pass closes the gap for
/// Workspaces created after a plugin registered without making event delivery correctness input.
pub(crate) fn converge_workspace_targets(
    repository: &SqliteEffectRepository,
    workspaces: &[Workspace],
    declarations: &[ConsumerDeclaration],
) -> Result<usize, BackendError> {
    for declaration in declarations {
        repository
            .declare_consumer(declaration, workspaces)
            .map_err(|error| {
                BackendError::internal("failed to converge Effect Target declarations", error)
            })?;
    }
    Ok(declarations.len())
}
