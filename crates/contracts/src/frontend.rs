//! HTTP path templates shared by server adapters and frontend contract generation.

mod spec;

pub use spec::{
    PROJECT_SPEC_SOURCES_PATH, SPEC_CATALOG_PATH, SPEC_READ_PATH, SPEC_RESOLVE_SOURCE_PATH,
    SPEC_WATCH_PATH,
};

pub const PROJECTS_PATH: &str = "/api/projects";
pub const PROJECT_PATH: &str = "/api/projects/{projectId}";
pub const PROJECT_BRANCHES_PATH: &str = "/api/projects/{projectId}/branches";
pub const REPOSITORY_SNAPSHOT_PATH: &str = "/api/projects/{projectId}/repository/snapshot";
pub const REPOSITORY_COMMIT_PATH: &str = "/api/projects/{projectId}/repository/commits/{commitId}";
pub const REPOSITORY_COMMIT_DIFF_PATH: &str =
    "/api/projects/{projectId}/repository/commits/{commitId}/diff";
pub const REPOSITORY_WORKING_TREE_DIFF_PATH: &str =
    "/api/projects/{projectId}/repository/working-tree-diff";
pub const REPOSITORY_BRANCH_CREATE_PATH: &str = "/api/projects/{projectId}/repository/branches";
pub const REPOSITORY_BRANCH_CHECKOUT_PATH: &str = "/api/projects/{projectId}/repository/checkout";
pub const REPOSITORY_FETCH_PATH: &str = "/api/projects/{projectId}/repository/fetch";
pub const REPOSITORY_PULL_PATH: &str = "/api/projects/{projectId}/repository/pull";
pub const REPOSITORY_SYNC_PATH: &str = "/api/projects/{projectId}/repository/sync";
pub const REPOSITORY_CONFLICT_RESOLVE_PATH: &str =
    "/api/projects/{projectId}/repository/conflicts/resolve";
pub const REPOSITORY_PUSH_PATH: &str = "/api/projects/{projectId}/repository/push";
pub const REPOSITORY_STAGE_CHANGES_PATH: &str =
    "/api/projects/{projectId}/repository/changes/stage";
pub const REPOSITORY_UNSTAGE_CHANGES_PATH: &str =
    "/api/projects/{projectId}/repository/changes/unstage";
pub const REPOSITORY_COMMIT_CHANGES_PATH: &str = "/api/projects/{projectId}/repository/commit";
pub const PROJECT_WORK_CONTEXT_OPEN_PATH: &str = "/api/project-work-contexts/open";
pub const PROJECT_WORK_CONTEXT_RENEW_PATH: &str = "/api/project-work-contexts/renew";
pub const TASKS_PATH: &str = "/api/tasks";
pub const TASK_PATH: &str = "/api/tasks/{taskId}";
pub const TASK_WORKSPACE_PATH: &str = "/api/tasks/{taskId}/workspace";
pub const TASK_DIFF_PATH: &str = "/api/tasks/{taskId}/diff";
pub const TASK_COMMIT_PATH: &str = "/api/tasks/{taskId}/git/commit";
pub const TASK_PUSH_PATH: &str = "/api/tasks/{taskId}/git/push";
pub const TASK_DIFF_COMMENTS_PATH: &str = "/api/tasks/{taskId}/diff/comments";
pub const TASK_DIFF_COMMENT_REPLIES_PATH: &str =
    "/api/tasks/{taskId}/diff/comments/{commentId}/replies";
pub const TASK_DIFF_COMMENT_STATUS_PATH: &str =
    "/api/tasks/{taskId}/diff/comments/{commentId}/status";
pub const SESSIONS_PATH: &str = "/api/sessions";
pub const SESSION_PATH: &str = "/api/sessions/{sessionId}";
pub const SESSION_LOAD_PATH: &str = "/api/sessions/{sessionId}/load";
pub const SESSION_PROMPT_PATH: &str = "/api/sessions/{sessionId}/prompt";
pub const SESSION_PERMISSION_RESPONSE_PATH: &str = "/api/sessions/{sessionId}/permissions/respond";
pub const SESSION_STOP_PATH: &str = "/api/sessions/{sessionId}/stop";
pub const SESSION_SWITCH_AGENT_PATH: &str = "/api/sessions/{sessionId}/agent";
pub const SESSION_RESUME_HISTORY_PATH: &str = "/api/sessions/{sessionId}/history/resume";
pub const AGENT_RUNTIME_STATUS_PATH: &str = "/api/agent-runtime/status";
pub const SESSION_WARM_PATH: &str = "/api/sessions/warm";
pub const SESSION_CONFIG_PATH: &str = "/api/sessions/{sessionId}/config";
pub const SESSION_ATTACH_PATH: &str = "/api/sessions/{sessionId}/attach";
pub const SKILLS_PATH: &str = "/api/skills";
pub const SKILL_PATH: &str = "/api/skills/{skillId}";
pub const SKILL_IMPORTS_PATH: &str = "/api/skill-imports";
pub const SKILL_IMPORT_PATH: &str = "/api/skill-imports/{sessionId}";
pub const SKILL_IMPORT_COMMIT_PATH: &str = "/api/skill-imports/{sessionId}/commit";
pub const AGENTS_PATH: &str = "/api/agents";
pub const AGENT_PATH: &str = "/api/agents/{agentId}";
pub const AGENT_IMPORT_PREPARE_PATH: &str = "/api/agent-imports/prepare";
pub const AGENT_IMPORT_COMMIT_PATH: &str = "/api/agent-imports/commit";
pub const FILE_SYSTEM_DIRECTORY_PATH: &str = "/api/file-system/directory";
pub const WORKSPACE_DIRECTORY_PATH: &str = "/api/tasks/{taskId}/files/list";
pub const WORKSPACE_FILE_PATH: &str = "/api/tasks/{taskId}/files/read";
pub const WORKSPACE_SEARCH_PATH: &str = "/api/tasks/{taskId}/files/search";
pub const WORKSPACE_WATCH_PATH: &str = "/api/tasks/{taskId}/files/watch";
pub const PROJECT_DIRECTORY_PATH: &str = "/api/projects/{projectId}/files/list";
pub const PROJECT_FILE_PATH: &str = "/api/projects/{projectId}/files/read";
pub const PROJECT_SEARCH_PATH: &str = "/api/projects/{projectId}/files/search";
pub const PROJECT_WATCH_PATH: &str = "/api/projects/{projectId}/files/watch";
pub const GIT_IDENTITY_PATH: &str = "/api/git/identity";
pub const WORKFLOWS_PATH: &str = "/api/workflows";
pub const WORKFLOW_PATH: &str = "/api/workflows/{workflowId}";
pub const WORKFLOW_DRAFT_PATH: &str = "/api/workflows/{workflowId}/draft";
pub const WORKFLOW_PUBLISH_PATH: &str = "/api/workflows/{workflowId}/publish";
pub const WORKFLOW_ROLLBACK_PATH: &str = "/api/workflows/{workflowId}/rollback";
pub const WORKFLOW_ACTIVATE_PATH: &str = "/api/workflows/{workflowId}/activate";
pub const WORKFLOW_VERSIONS_PATH: &str = "/api/workflows/{workflowId}/versions";
pub const WORKFLOW_VERSION_PATH: &str = "/api/workflows/{workflowId}/versions/{version}";
pub const WORKFLOW_RUNS_PATH: &str = "/api/workflow-runs";
pub const WORKFLOW_RUN_PATH: &str = "/api/workflow-runs/{runId}";
pub const WORKFLOW_RUN_NODES_PATH: &str = "/api/workflow-runs/{runId}/nodes";
pub const WORKFLOW_SNAPSHOT_PATH: &str = "/api/workflow-snapshots/{snapshotId}";
