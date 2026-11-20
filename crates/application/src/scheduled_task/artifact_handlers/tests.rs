use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Mutex};

use ora_domain::{
    ScheduledArtifactKind, ScheduledArtifactRejection, ScheduledArtifactRejectionReason,
    ScheduledRunArtifact, ScheduledRunId, ScheduledTaskId,
};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::{
    CollectScheduledRunArtifactsHandler, CopyScheduledRunArtifactHandler,
    ListScheduledRunArtifactsHandler, ReadScheduledRunArtifactHandler,
    SweepScheduledRunArtifactsHandler,
};
use crate::scheduled_task::{ScheduledArtifactRunRecord, ScheduledRunArtifactRepository};
use crate::{ApplicationError, ScheduledArtifactStore};

#[derive(Default)]
struct FakeState {
    runs: BTreeMap<String, ScheduledArtifactRunRecord>,
    artifacts: BTreeMap<String, Vec<ScheduledRunArtifact>>,
    rejections: BTreeMap<String, Vec<ScheduledArtifactRejection>>,
}

#[derive(Clone, Default)]
struct FakeArtifactRepository {
    state: Arc<Mutex<FakeState>>,
}

impl FakeArtifactRepository {
    fn insert(
        &self,
        record: ScheduledArtifactRunRecord,
        artifacts: Vec<ScheduledRunArtifact>,
        rejections: Vec<ScheduledArtifactRejection>,
    ) {
        let run_id = record.run_id.to_string();
        let mut state = self.state.lock().unwrap();
        state.runs.insert(run_id.clone(), record);
        state.artifacts.insert(run_id.clone(), artifacts);
        state.rejections.insert(run_id, rejections);
    }

    fn artifacts(&self, run_id: &str) -> Vec<ScheduledRunArtifact> {
        self.state
            .lock()
            .unwrap()
            .artifacts
            .get(run_id)
            .cloned()
            .unwrap_or_default()
    }

    fn has_rows(&self, run_id: &str) -> bool {
        let state = self.state.lock().unwrap();
        state
            .artifacts
            .get(run_id)
            .is_some_and(|artifacts| !artifacts.is_empty())
            || state
                .rejections
                .get(run_id)
                .is_some_and(|rejections| !rejections.is_empty())
    }
}

impl ScheduledRunArtifactRepository for FakeArtifactRepository {
    fn record_scheduled_run_artifacts(
        &self,
        run_id: &ScheduledRunId,
        artifacts: &[ScheduledRunArtifact],
        rejections: &[ScheduledArtifactRejection],
        _recorded_at: i64,
    ) -> Result<(), crate::RepositoryError> {
        let mut state = self.state.lock().unwrap();
        state
            .artifacts
            .insert(run_id.to_string(), artifacts.to_vec());
        state
            .rejections
            .insert(run_id.to_string(), rejections.to_vec());
        Ok(())
    }

    fn list_scheduled_run_artifacts(
        &self,
        run_id: &ScheduledRunId,
    ) -> Result<Vec<ScheduledRunArtifact>, crate::RepositoryError> {
        Ok(self.artifacts(run_id.as_ref()))
    }

    fn list_scheduled_run_artifact_rejections(
        &self,
        run_id: &ScheduledRunId,
    ) -> Result<Vec<ScheduledArtifactRejection>, crate::RepositoryError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .rejections
            .get(run_id.as_ref())
            .cloned()
            .unwrap_or_default())
    }

    fn find_scheduled_run_artifact(
        &self,
        run_id: &ScheduledRunId,
        relative_path: &str,
    ) -> Result<Option<ScheduledRunArtifact>, crate::RepositoryError> {
        Ok(self
            .artifacts(run_id.as_ref())
            .into_iter()
            .find(|artifact| artifact.relative_path == relative_path))
    }

    fn list_scheduled_artifact_runs(
        &self,
    ) -> Result<Vec<ScheduledArtifactRunRecord>, crate::RepositoryError> {
        let state = self.state.lock().unwrap();
        let mut records = state
            .runs
            .values()
            .filter(|record| {
                state
                    .artifacts
                    .get(record.run_id.as_ref())
                    .is_some_and(|artifacts| !artifacts.is_empty())
                    || state
                        .rejections
                        .get(record.run_id.as_ref())
                        .is_some_and(|rejections| !rejections.is_empty())
            })
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| (record.updated_at, record.run_id.clone()));
        Ok(records)
    }

    fn find_scheduled_artifact_run(
        &self,
        run_id: &ScheduledRunId,
    ) -> Result<Option<ScheduledArtifactRunRecord>, crate::RepositoryError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .runs
            .get(run_id.as_ref())
            .cloned())
    }

    fn delete_scheduled_run_artifact(
        &self,
        run_id: &ScheduledRunId,
        relative_path: &str,
    ) -> Result<bool, crate::RepositoryError> {
        let mut state = self.state.lock().unwrap();
        let Some(artifacts) = state.artifacts.get_mut(run_id.as_ref()) else {
            return Ok(false);
        };
        let before = artifacts.len();
        artifacts.retain(|artifact| artifact.relative_path != relative_path);
        Ok(artifacts.len() != before)
    }

    fn delete_scheduled_run_artifact_rows(
        &self,
        run_id: &ScheduledRunId,
    ) -> Result<usize, crate::RepositoryError> {
        let mut state = self.state.lock().unwrap();
        let artifacts = state
            .artifacts
            .remove(run_id.as_ref())
            .map_or(0, |rows| rows.len());
        let rejections = state
            .rejections
            .remove(run_id.as_ref())
            .map_or(0, |rows| rows.len());
        Ok(artifacts + rejections)
    }
}

fn fixture() -> (TempDir, ScheduledArtifactStore, String) {
    // The artifact store deliberately rejects symlinked ancestors, and the system temporary
    // root is not guaranteed to be free of them, so keep these fixtures under the checkout
    // instead of relaxing the production safety boundary.
    let temporary = tempfile::tempdir_in(std::env::current_dir().expect("current directory"))
        .expect("create temp directory");
    let store = ScheduledArtifactStore::new(temporary.path().join("store"));
    let workspace = temporary.path().join("worktree");
    (temporary, store, workspace.to_string_lossy().into_owned())
}

fn record(
    task: &str,
    run: &str,
    updated_at: i64,
    retains_worktree: bool,
) -> ScheduledArtifactRunRecord {
    ScheduledArtifactRunRecord {
        run_id: ScheduledRunId::new(run),
        task_id: Some(ScheduledTaskId::new(task)),
        updated_at: Some(updated_at),
        run_missing: false,
        task_deleted: false,
        retains_worktree,
    }
}

fn artifact(name: &str, size: u64) -> ScheduledRunArtifact {
    ScheduledRunArtifact {
        relative_path: name.to_string(),
        kind: ScheduledArtifactKind::Markdown,
        size_bytes: size,
        registered_at: 1,
    }
}

fn write_artifact_file(store: &ScheduledArtifactStore, run: &str, name: &str) {
    let run_directory = store.root().join(run);
    fs::create_dir_all(&run_directory).expect("create run directory");
    fs::write(run_directory.join(name), "x").expect("write stored artifact");
}

#[test]
fn sweep_removes_orphan_rows_and_soft_deleted_task_storage() {
    let (temporary, store, workspace) = fixture();
    let source = std::path::Path::new(&workspace).join(".ora-scheduled-artifacts");
    fs::create_dir_all(&source).expect("create worktree artifact directory");
    fs::write(source.join("report.md"), "report").expect("write artifact");
    let repository = FakeArtifactRepository::default();
    let collect =
        CollectScheduledRunArtifactsHandler::new(Arc::new(repository.clone()), store.clone());
    collect
        .handle(
            &ScheduledRunId::new("run-orphan"),
            std::path::Path::new(&workspace),
            1,
        )
        .expect("collect orphan artifacts");
    collect
        .handle(
            &ScheduledRunId::new("run-deleted"),
            std::path::Path::new(&workspace),
            1,
        )
        .expect("collect deleted-task artifacts");

    let mut orphan = record("task-1", "run-orphan", 1, false);
    orphan.run_missing = true;
    orphan.task_id = None;
    let mut deleted = record("task-2", "run-deleted", 2, false);
    deleted.task_deleted = true;
    repository.insert(orphan, vec![artifact("report.md", 6)], Vec::new());
    repository.insert(deleted, vec![artifact("report.md", 6)], Vec::new());

    SweepScheduledRunArtifactsHandler::new(Arc::new(repository.clone()), store.clone())
        .handle()
        .expect("sweep artifacts");

    assert!(!store.root().join("run-orphan").exists());
    assert!(!store.root().join("run-deleted").exists());
    assert!(!repository.has_rows("run-orphan"));
    assert!(!repository.has_rows("run-deleted"));
    drop(temporary);
}

#[test]
fn sweep_keeps_the_newest_ten_unlinked_runs_and_exempts_linked_runs() {
    let (temporary, store, _workspace) = fixture();
    let repository = FakeArtifactRepository::default();
    // The oldest run still retains its worktree, so it is exempt and not counted.
    let mut linked = record("task-1", "run-linked", 0, true);
    linked.updated_at = Some(0);
    repository.insert(linked, vec![artifact("linked.md", 1)], Vec::new());
    write_artifact_file(&store, "run-linked", "linked.md");
    for index in 1..=12 {
        let run = format!("run-{index:02}");
        repository.insert(
            record("task-1", &run, index, false),
            vec![artifact("report.md", 1)],
            Vec::new(),
        );
        write_artifact_file(&store, &run, "report.md");
    }

    SweepScheduledRunArtifactsHandler::new(Arc::new(repository.clone()), store.clone())
        .handle()
        .expect("sweep artifacts");

    assert!(repository.has_rows("run-linked"));
    assert!(!repository.has_rows("run-01"));
    assert!(!repository.has_rows("run-02"));
    assert!(repository.has_rows("run-03"));
    assert!(repository.has_rows("run-12"));
    assert_eq!(
        (3..=12)
            .filter(|index| repository.has_rows(&format!("run-{index:02}")))
            .count(),
        10
    );
    drop(temporary);
}

#[test]
fn sweep_deletes_rows_whose_file_vanished_and_files_without_rows() {
    let (temporary, store, _workspace) = fixture();
    let run_id = ScheduledRunId::new("run-1");
    let run_directory = store.root().join(run_id.as_ref());
    fs::create_dir_all(&run_directory).expect("create run directory");
    fs::write(run_directory.join("stray.md"), "stray").expect("write stray");
    let repository = FakeArtifactRepository::default();
    repository.insert(
        record("task-1", run_id.as_ref(), 1, false),
        vec![artifact("missing.md", 3)],
        Vec::new(),
    );

    SweepScheduledRunArtifactsHandler::new(Arc::new(repository.clone()), store.clone())
        .handle()
        .expect("sweep artifacts");

    assert!(!repository.has_rows(run_id.as_ref()));
    assert!(!run_directory.join("stray.md").exists());
    drop(temporary);
}

#[test]
fn list_and_read_reject_pruned_or_soft_deleted_runs() {
    let (_temporary, store, workspace) = fixture();
    let repository = FakeArtifactRepository::default();
    let mut deleted = record("task-1", "run-1", 1, false);
    deleted.task_deleted = true;
    repository.insert(
        deleted,
        vec![artifact("report.md", 1)],
        vec![ScheduledArtifactRejection {
            name: "secret.pdf".to_string(),
            reason: ScheduledArtifactRejectionReason::UnsupportedType,
        }],
    );
    let list = ListScheduledRunArtifactsHandler::new(Arc::new(repository.clone()));
    let read = ReadScheduledRunArtifactHandler::new(Arc::new(repository.clone()), store.clone());

    assert_eq!(
        list.handle(&ScheduledRunId::new("run-1"))
            .expect_err("reject deleted task artifacts"),
        ApplicationError::ScheduledRunNotFound {
            run_id: "run-1".to_string(),
        }
    );
    assert_eq!(
        read.handle(&ScheduledRunId::new("run-1"), "report.md")
            .expect_err("reject deleted task artifact reads"),
        ApplicationError::ScheduledRunNotFound {
            run_id: "run-1".to_string(),
        }
    );
    assert_eq!(
        list.handle(&ScheduledRunId::new("run-missing"))
            .expect_err("reject unknown run artifacts"),
        ApplicationError::ScheduledRunNotFound {
            run_id: "run-missing".to_string(),
        }
    );
    drop(workspace);
}

#[test]
fn copy_exports_an_artifact_after_the_worktree_was_reclaimed() {
    let (temporary, store, workspace) = fixture();
    let source = std::path::Path::new(&workspace).join(".ora-scheduled-artifacts");
    fs::create_dir_all(&source).expect("create artifact directory");
    fs::write(source.join("report.md"), "report").expect("write artifact");
    let repository = FakeArtifactRepository::default();
    let run_id = ScheduledRunId::new("run-1");
    repository.insert(record("task-1", "run-1", 1, false), Vec::new(), Vec::new());
    CollectScheduledRunArtifactsHandler::new(Arc::new(repository.clone()), store.clone())
        .handle(&run_id, std::path::Path::new(&workspace), 1)
        .expect("collect artifacts");
    // The janitor discards an unmodified worktree after collection; the manifest and the
    // stored copy must remain readable without the original directory.
    fs::remove_dir_all(&source).expect("reclaim worktree artifact directory");
    let destination = temporary.path().join("exported.md");

    CopyScheduledRunArtifactHandler::new(Arc::new(repository), store)
        .handle(&run_id, "report.md", &destination)
        .expect("export artifact");

    assert_eq!(fs::read(destination).expect("read export"), b"report");
}
