use ora_domain::TaskId;

/// Number of leading task-id characters used in Ora-owned branch names.
pub const TASK_BRANCH_PREFIX_LEN: usize = 8;

/// Derives the stable task branch name from the first eight characters of the task id.
///
/// This is the single source of the `ora/<prefix>` invariant: task creation,
/// workflow-run provisioning, and Git cleanup identity validation must all
/// derive branch names through this function so a persisted branch can always
/// be re-checked against the task that owns it.
pub fn branch_name_for_task(task_id: &TaskId) -> String {
    format!("ora/{}", task_branch_prefix(task_id))
}

/// Derives the short branch prefix used to keep task branch names readable.
pub fn task_branch_prefix(task_id: &TaskId) -> String {
    task_id
        .to_string()
        .chars()
        .take(TASK_BRANCH_PREFIX_LEN)
        .collect()
}
