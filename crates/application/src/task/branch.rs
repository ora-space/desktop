use ora_domain::WorkspaceId;

/// Number of leading workspace-id characters used in Ora-owned branch names.
pub const WORKSPACE_BRANCH_PREFIX_LEN: usize = 8;

/// Derives the stable worktree branch name from the first eight characters of the workspace id.
///
/// The Workspace owns the physical worktree resources, so cleanup can re-check
/// this invariant without treating the user-facing Task projection as an owner.
pub fn branch_name_for_workspace(workspace_id: &WorkspaceId) -> String {
    format!("ora/{}", workspace_branch_prefix(workspace_id))
}

/// Derives the short branch prefix used to keep workspace branch names readable.
pub fn workspace_branch_prefix(workspace_id: &WorkspaceId) -> String {
    workspace_id
        .to_string()
        .chars()
        .take(WORKSPACE_BRANCH_PREFIX_LEN)
        .collect()
}
