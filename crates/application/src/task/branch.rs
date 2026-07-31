use ora_domain::TaskId;

const TASK_BRANCH_PREFIX_LEN: usize = 8;

/// Derives the short task-id prefix used for both branch names and collision checks.
pub fn branch_prefix_for_task(task_id: &TaskId) -> String {
    task_id
        .to_string()
        .chars()
        .take(TASK_BRANCH_PREFIX_LEN)
        .collect()
}

/// Derives the Ora-owned branch name shared by task creation and aggregate cleanup.
pub fn branch_name_for_task(task_id: &TaskId) -> String {
    let branch_prefix = branch_prefix_for_task(task_id);
    format!("ora/{branch_prefix}")
}
