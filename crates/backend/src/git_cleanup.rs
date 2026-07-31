use gitlancer::CliGitRunner;
use ora_application::{
    GitResourceCleanupOutcome, GitTaskGitResourceCleaner, TaskGitResourceCleaner,
    TaskGitResourceCleanupRequest, branch_name_for_task,
};
use ora_db::GitCleanupTarget;
use ora_logging::{ora_error, ora_info};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use uuid::Uuid;

/// Identifies which aggregate deletion initiated a best-effort Git cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AggregateDeletionKind {
    Project,
    Task,
}

impl AggregateDeletionKind {
    /// Returns the stable operation field shared by cleanup log events.
    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::Project => "delete_project",
            Self::Task => "delete_task",
        }
    }
}

/// Carries one committed database response and its post-commit Git cleanup targets.
pub(crate) struct CommittedAggregateDeletion<Response> {
    pub(crate) response: Response,
    pub(crate) git_cleanup_targets: Vec<GitCleanupTarget>,
}

/// Best-effort removes every validated Ora-owned worktree and branch after database commit.
pub(crate) fn cleanup_git_resources(
    deletion_kind: AggregateDeletionKind,
    targets: &[GitCleanupTarget],
    worktree_root: &Path,
) {
    cleanup_git_resources_with_cleaner(
        deletion_kind,
        targets,
        worktree_root,
        &GitTaskGitResourceCleaner::new(CliGitRunner),
    );
}

/// Runs cleanup through an injected application port so backend policy remains testable.
fn cleanup_git_resources_with_cleaner<Cleaner: TaskGitResourceCleaner>(
    deletion_kind: AggregateDeletionKind,
    targets: &[GitCleanupTarget],
    worktree_root: &Path,
    cleaner: &Cleaner,
) {
    for target in targets {
        let cleanup = catch_unwind(AssertUnwindSafe(|| {
            if Uuid::parse_str(target.task_id.as_ref()).is_err() {
                log_cleanup_failure(
                    deletion_kind,
                    target,
                    "ownership_validation",
                    "task id is not a valid UUID".to_string(),
                );
                return;
            }
            let expected_branch_name = branch_name_for_task(&target.task_id);
            if target.branch_name != expected_branch_name {
                log_cleanup_failure(
                    deletion_kind,
                    target,
                    "ownership_validation",
                    format!(
                        "stored branch does not match expected Ora-owned branch {expected_branch_name}"
                    ),
                );
                return;
            }

            let report = cleaner.cleanup_task_git_resources(TaskGitResourceCleanupRequest {
                repository_root: target.repository_root.clone(),
                expected_worktree_root: worktree_root.join(target.task_id.as_ref()),
                branch_name: target.branch_name.clone(),
            });
            log_cleanup_outcome(deletion_kind, target, "worktree", report.worktree);
            log_cleanup_outcome(deletion_kind, target, "branch", report.branch);
        }));
        if let Err(panic) = cleanup {
            // Panic isolation preserves cleanup attempts for sibling tasks after the
            // database has already committed the entire aggregate deletion.
            let message = panic
                .downcast_ref::<&str>()
                .map(|message| (*message).to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic payload".to_string());
            log_cleanup_failure(
                deletion_kind,
                target,
                "task_resources",
                format!("cleanup panicked: {message}"),
            );
        }
    }
}

/// Emits the structured event appropriate for one independent cleanup stage.
fn log_cleanup_outcome(
    deletion_kind: AggregateDeletionKind,
    target: &GitCleanupTarget,
    stage: &'static str,
    outcome: GitResourceCleanupOutcome,
) {
    match outcome {
        GitResourceCleanupOutcome::Removed => {}
        GitResourceCleanupOutcome::AlreadyAbsent => {
            ora_info!(
                message = "aggregate Git resource already absent",
                operation = deletion_kind.operation(),
                cleanup_stage = stage,
                project_id = target.project_id.to_string(),
                task_id = target.task_id.to_string(),
                branch_name = target.branch_name.clone()
            );
        }
        GitResourceCleanupOutcome::Failed { message } => {
            log_cleanup_failure(deletion_kind, target, stage, message);
        }
    }
}

/// Emits one structured failure without exposing best-effort cleanup through the API response.
fn log_cleanup_failure(
    deletion_kind: AggregateDeletionKind,
    target: &GitCleanupTarget,
    stage: &'static str,
    error: String,
) {
    ora_error!(
        message = "aggregate Git cleanup failed",
        operation = deletion_kind.operation(),
        cleanup_stage = stage,
        project_id = target.project_id.to_string(),
        task_id = target.task_id.to_string(),
        branch_name = target.branch_name.clone(),
        error.message = error
    );
}

#[cfg(test)]
mod tests {
    use super::{AggregateDeletionKind, cleanup_git_resources_with_cleaner};
    use ora_application::{
        GitResourceCleanupOutcome, TaskGitResourceCleaner, TaskGitResourceCleanupReport,
        TaskGitResourceCleanupRequest,
    };
    use ora_db::GitCleanupTarget;
    use ora_domain::{ProjectId, TaskId};
    use ora_logging::{with_recorded_trace_logging, with_trace_logging};
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::registry::LookupSpan;

    const TASK_ID: &str = "12345678-1234-4234-8234-1234567890ab";

    /// Records validated cleanup requests while returning independent stage failures.
    #[derive(Debug, Default)]
    struct RecordingCleaner {
        requests: RefCell<Vec<TaskGitResourceCleanupRequest>>,
    }

    impl RecordingCleaner {
        /// Returns every cleanup request accepted by the application port.
        fn requests(&self) -> Vec<TaskGitResourceCleanupRequest> {
            self.requests.borrow().clone()
        }
    }

    /// Records every target while panicking for one selected branch.
    #[derive(Debug)]
    struct PanickingCleaner {
        panicking_branch: String,
        requests: RefCell<Vec<TaskGitResourceCleanupRequest>>,
    }

    impl PanickingCleaner {
        /// Builds a cleaner that exposes whether sibling targets continue after its panic.
        fn new(panicking_branch: &str) -> Self {
            Self {
                panicking_branch: panicking_branch.to_string(),
                requests: RefCell::new(Vec::new()),
            }
        }

        /// Returns every branch whose target reached the cleaner.
        fn requested_branches(&self) -> Vec<String> {
            self.requests
                .borrow()
                .iter()
                .map(|request| request.branch_name.clone())
                .collect()
        }
    }

    impl TaskGitResourceCleaner for PanickingCleaner {
        /// Simulates one unexpected cleaner panic between otherwise successful targets.
        fn cleanup_task_git_resources(
            &self,
            request: TaskGitResourceCleanupRequest,
        ) -> TaskGitResourceCleanupReport {
            let should_panic = request.branch_name == self.panicking_branch;
            self.requests.borrow_mut().push(request);
            assert!(!should_panic, "simulated cleaner panic");
            TaskGitResourceCleanupReport {
                worktree: GitResourceCleanupOutcome::Removed,
                branch: GitResourceCleanupOutcome::Removed,
            }
        }
    }

    impl TaskGitResourceCleaner for RecordingCleaner {
        /// Records requests and simulates a failed worktree with a successful branch removal.
        fn cleanup_task_git_resources(
            &self,
            request: TaskGitResourceCleanupRequest,
        ) -> TaskGitResourceCleanupReport {
            self.requests.borrow_mut().push(request);
            TaskGitResourceCleanupReport {
                worktree: GitResourceCleanupOutcome::Failed {
                    message: "worktree is locked".to_string(),
                },
                branch: GitResourceCleanupOutcome::Removed,
            }
        }
    }

    /// Verifies validated targets include the deterministic task checkout root.
    #[test]
    fn delegates_validated_targets_with_the_expected_worktree_root() {
        with_trace_logging(|| {
            let cleaner = RecordingCleaner::default();

            cleanup_git_resources_with_cleaner(
                AggregateDeletionKind::Task,
                &[cleanup_target("ora/12345678")],
                Path::new("/ora/worktrees"),
                &cleaner,
            );

            assert_eq!(
                cleaner.requests(),
                vec![TaskGitResourceCleanupRequest {
                    repository_root: PathBuf::from("/repo"),
                    expected_worktree_root: PathBuf::from(
                        "/ora/worktrees/12345678-1234-4234-8234-1234567890ab",
                    ),
                    branch_name: "ora/12345678".to_string(),
                }]
            );
        });
    }

    /// Verifies an unexpected persisted branch cannot trigger any destructive Git command.
    #[test]
    fn rejects_cleanup_targets_outside_the_task_branch_namespace() {
        with_trace_logging(|| {
            let cleaner = RecordingCleaner::default();

            cleanup_git_resources_with_cleaner(
                AggregateDeletionKind::Project,
                &[cleanup_target("feature/user-work")],
                Path::new("/ora/worktrees"),
                &cleaner,
            );

            assert_eq!(
                cleaner.requests(),
                Vec::<TaskGitResourceCleanupRequest>::new()
            );
        });
    }

    /// Verifies malformed persisted identifiers cannot escape the configured worktree root.
    #[test]
    fn rejects_cleanup_targets_with_non_uuid_task_ids() {
        with_trace_logging(|| {
            let cleaner = RecordingCleaner::default();
            let mut target = cleanup_target("ora/12345678");
            target.task_id = TaskId::new("12345678/../../other-worktree");

            cleanup_git_resources_with_cleaner(
                AggregateDeletionKind::Task,
                &[target],
                Path::new("/ora/worktrees"),
                &cleaner,
            );

            assert_eq!(
                cleaner.requests(),
                Vec::<TaskGitResourceCleanupRequest>::new()
            );
        });
    }

    /// Verifies one cleaner panic does not skip later targets in a Project deletion.
    #[test]
    fn continues_with_sibling_targets_after_cleaner_panics() {
        let recorder = EventRecorder::default();
        with_recorded_trace_logging(recorder.layer(), || {
            let cleaner = PanickingCleaner::new("ora/87654321");
            let targets = [
                cleanup_target_for("12345678-1234-4234-8234-1234567890ab"),
                cleanup_target_for("87654321-4321-4321-8321-ba0987654321"),
                cleanup_target_for("abcdefab-cdef-4def-8def-abcdefabcdef"),
            ];

            cleanup_git_resources_with_cleaner(
                AggregateDeletionKind::Project,
                &targets,
                Path::new("/ora/worktrees"),
                &cleaner,
            );

            assert_eq!(
                cleaner.requested_branches(),
                vec![
                    "ora/12345678".to_string(),
                    "ora/87654321".to_string(),
                    "ora/abcdefab".to_string(),
                ]
            );
        });
        assert_eq!(
            recorder.events(),
            vec![LoggedEvent {
                level: "ERROR".to_string(),
                target: "ora_backend::git_cleanup".to_string(),
                fields: BTreeMap::from([
                    ("branch_name".to_string(), "ora/87654321".to_string()),
                    ("cleanup_stage".to_string(), "task_resources".to_string()),
                    (
                        "error.message".to_string(),
                        "cleanup panicked: simulated cleaner panic".to_string()
                    ),
                    (
                        "message".to_string(),
                        "aggregate Git cleanup failed".to_string()
                    ),
                    ("method".to_string(), "log_cleanup_failure".to_string()),
                    ("operation".to_string(), "delete_project".to_string()),
                    ("project_id".to_string(), "project-1".to_string()),
                    (
                        "task_id".to_string(),
                        "87654321-4321-4321-8321-ba0987654321".to_string()
                    ),
                ]),
            }]
        );
    }

    /// Builds one target whose task id has the production eight-character branch prefix.
    fn cleanup_target(branch_name: &str) -> GitCleanupTarget {
        GitCleanupTarget {
            project_id: ProjectId::new("project-1"),
            task_id: TaskId::new(TASK_ID),
            repository_root: PathBuf::from("/repo"),
            branch_name: branch_name.to_string(),
        }
    }

    /// Builds one internally consistent target for multi-target cleanup tests.
    fn cleanup_target_for(task_id: &str) -> GitCleanupTarget {
        let task_id = TaskId::new(task_id);
        GitCleanupTarget {
            project_id: ProjectId::new("project-1"),
            branch_name: ora_application::branch_name_for_task(&task_id),
            task_id,
            repository_root: PathBuf::from("/repo"),
        }
    }

    /// Captures one emitted event in a comparison-friendly structure.
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct LoggedEvent {
        level: String,
        target: String,
        fields: BTreeMap<String, String>,
    }

    /// Records cleanup events under a test-scoped TRACE subscriber.
    #[derive(Clone, Debug, Default)]
    struct EventRecorder {
        events: Arc<Mutex<Vec<LoggedEvent>>>,
    }

    impl EventRecorder {
        /// Builds the layer attached to the scoped test subscriber.
        fn layer(&self) -> RecordingLayer {
            RecordingLayer {
                events: self.events.clone(),
            }
        }

        /// Returns every captured cleanup event in emission order.
        fn events(&self) -> Vec<LoggedEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    /// Pushes structured cleanup events into shared test memory.
    #[derive(Clone, Debug)]
    struct RecordingLayer {
        events: Arc<Mutex<Vec<LoggedEvent>>>,
    }

    impl<Subscriber> Layer<Subscriber> for RecordingLayer
    where
        Subscriber: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        /// Converts each event into a stable structure for deep equality assertions.
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Subscriber>) {
            let mut visitor = EventFieldVisitor::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(LoggedEvent {
                level: event.metadata().level().to_string(),
                target: event.metadata().target().to_string(),
                fields: visitor.fields,
            });
        }
    }

    /// Records tracing fields as strings so structured semantics remain easy to compare.
    #[derive(Debug, Default)]
    struct EventFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl tracing::field::Visit for EventFieldVisitor {
        /// Preserves string fields exactly as cleanup logging emitted them.
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        /// Falls back to debug formatting for fields without a string visitor hook.
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.fields.insert(
                field.name().to_string(),
                format!("{value:?}").trim_matches('"').to_string(),
            );
        }
    }
}
