use std::collections::HashSet;
use std::fs;
use std::path::Path;

use ora_domain::{
    SCHEDULED_ARTIFACT_DIRECTORY, ScheduledArtifactKind, ScheduledArtifactRejection,
    ScheduledArtifactRejectionReason, ScheduledRunArtifact, ScheduledRunId,
};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::ScheduledArtifactStore;

fn fixture() -> (TempDir, ScheduledArtifactStore, ScheduledRunId) {
    // The artifact store deliberately rejects symlinked ancestors, and the system temporary
    // root is not guaranteed to be free of them, so keep these fixtures under the checkout
    // instead of relaxing the production safety boundary.
    let temporary = tempfile::tempdir_in(std::env::current_dir().expect("current directory"))
        .expect("create temp directory");
    let store = ScheduledArtifactStore::new(temporary.path().join("store"));
    (temporary, store, ScheduledRunId::new("run-1"))
}

fn artifact_source(workspace: &std::path::Path) -> std::path::PathBuf {
    workspace.join(SCHEDULED_ARTIFACT_DIRECTORY)
}

/// Creates a directory link without following it; Windows junctions need no privilege.
#[cfg(unix)]
fn link_directory(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("create directory symlink");
}

/// Creates a directory junction through mklink, which is a cmd builtin.
#[cfg(windows)]
fn link_directory(target: &Path, link: &Path) {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .status()
        .expect("run mklink");
    assert!(status.success(), "mklink /J failed");
}

#[test]
fn collects_whitelisted_files_recursively_and_copies_them_out_of_the_worktree() {
    let (temporary, store, run_id) = fixture();
    let workspace = temporary.path().join("worktree");
    let source = artifact_source(&workspace);
    fs::create_dir_all(source.join("nested")).expect("create artifact directory");
    fs::write(source.join("report.md"), "# report").expect("write markdown");
    fs::write(source.join("nested").join("result.json"), "{}").expect("write json");

    let collected = store
        .collect(&run_id, &workspace, 42)
        .expect("collect artifacts");

    assert_eq!(
        collected
            .artifacts
            .iter()
            .map(|artifact| (
                artifact.relative_path.as_str(),
                artifact.kind,
                artifact.size_bytes,
                artifact.registered_at,
            ))
            .collect::<Vec<_>>(),
        vec![
            ("nested/result.json", ScheduledArtifactKind::Json, 2, 42),
            ("report.md", ScheduledArtifactKind::Markdown, 8, 42),
        ]
    );
    assert!(collected.rejections.is_empty());
    assert_eq!(
        store
            .read_artifact(&run_id, &collected.artifacts[0])
            .expect("read artifact"),
        b"{}"
    );
    assert_eq!(fs::read(source.join("report.md")).unwrap(), b"# report");
}

#[test]
fn reports_unsupported_types_without_copying_them() {
    let (temporary, store, run_id) = fixture();
    let workspace = temporary.path().join("worktree");
    let source = artifact_source(&workspace);
    fs::create_dir_all(&source).expect("create artifact directory");
    fs::write(source.join("secret.pdf"), "pdf").expect("write unsupported file");

    let collected = store
        .collect(&run_id, &workspace, 7)
        .expect("collect artifacts");

    assert!(collected.artifacts.is_empty());
    assert_eq!(
        collected.rejections,
        vec![ScheduledArtifactRejection {
            name: "secret.pdf".to_string(),
            reason: ScheduledArtifactRejectionReason::UnsupportedType,
        }]
    );
}

#[cfg(unix)]
#[test]
fn reports_windows_alternate_data_stream_names_as_unsafe() {
    let (temporary, store, run_id) = fixture();
    let workspace = temporary.path().join("worktree");
    let source = artifact_source(&workspace);
    fs::create_dir_all(&source).expect("create artifact directory");
    fs::write(source.join("stream.md:payload"), "ads").expect("write ADS-shaped name");

    let collected = store
        .collect(&run_id, &workspace, 7)
        .expect("collect artifacts");

    assert!(collected.artifacts.is_empty());
    assert_eq!(
        collected.rejections,
        vec![ScheduledArtifactRejection {
            name: "stream.md:payload".to_string(),
            reason: ScheduledArtifactRejectionReason::PathUnsafe,
        }]
    );
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_and_case_only_duplicates() {
    use std::os::unix::fs::symlink;

    let (temporary, store, run_id) = fixture();
    let workspace = temporary.path().join("worktree");
    let source = artifact_source(&workspace);
    fs::create_dir_all(&source).expect("create artifact directory");
    let outside = temporary.path().join("outside.txt");
    fs::write(&outside, "outside").expect("write outside file");
    symlink(&outside, source.join("linked.md")).expect("create symlink");
    fs::write(source.join("Report.md"), "one").expect("write first case");
    fs::write(source.join("report.md"), "two").expect("write second case");

    // A case-insensitive Unix filesystem folds both spellings onto one file; the duplicate
    // rejection can only be observed where the two directory entries are actually distinct.
    let distinct_case_variants = fs::read_dir(&source)
        .expect("read artifact directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .count()
        == 2;

    let collected = store
        .collect(&run_id, &workspace, 9)
        .expect("collect artifacts");

    assert_eq!(collected.artifacts.len(), 1);
    let accepted = collected.artifacts[0].relative_path.as_str();
    assert!(matches!(accepted, "Report.md" | "report.md"));
    let rejections = collected
        .rejections
        .iter()
        .map(|rejection| (rejection.name.as_str(), rejection.reason))
        .collect::<Vec<_>>();
    assert!(
        rejections.contains(&("linked.md", ScheduledArtifactRejectionReason::Symlink)),
        "the symlink is always refused: {rejections:?}"
    );
    assert!(
        !rejections.iter().any(|(name, _)| *name == accepted),
        "the accepted artifact must not also be listed as rejected: {rejections:?}"
    );
    if distinct_case_variants {
        assert_eq!(rejections.len(), 2);
        assert_eq!(
            rejections
                .iter()
                .filter(|(_, reason)| *reason == ScheduledArtifactRejectionReason::PathUnsafe)
                .count(),
            1
        );
    } else {
        assert_eq!(
            rejections,
            vec![("linked.md", ScheduledArtifactRejectionReason::Symlink)]
        );
    }
}

#[cfg(unix)]
#[test]
fn rejects_a_file_that_shares_its_inode_with_an_outside_name() {
    let (temporary, store, run_id) = fixture();
    let workspace = temporary.path().join("worktree");
    let source = artifact_source(&workspace);
    fs::create_dir_all(&source).expect("create artifact directory");
    let outside = temporary.path().join("outside.md");
    fs::write(&outside, "shared").expect("write outside file");
    fs::hard_link(&outside, source.join("linked.md")).expect("create hard link");

    let collected = store
        .collect(&run_id, &workspace, 13)
        .expect("collect artifacts");

    assert!(collected.artifacts.is_empty());
    assert_eq!(
        collected.rejections,
        vec![ScheduledArtifactRejection {
            name: "linked.md".to_string(),
            reason: ScheduledArtifactRejectionReason::HardLink,
        }]
    );
}

#[test]
fn rejects_files_over_the_single_artifact_bound() {
    let (temporary, store, run_id) = fixture();
    let workspace = temporary.path().join("worktree");
    let source = artifact_source(&workspace);
    fs::create_dir_all(&source).expect("create artifact directory");
    let oversized = vec![b'a'; (ora_domain::MAX_SCHEDULED_ARTIFACT_BYTES + 1) as usize];
    fs::write(source.join("big.md"), oversized).expect("write oversized file");

    let collected = store
        .collect(&run_id, &workspace, 10)
        .expect("collect artifacts");

    assert!(collected.artifacts.is_empty());
    assert_eq!(
        collected.rejections,
        vec![ScheduledArtifactRejection {
            name: "big.md".to_string(),
            reason: ScheduledArtifactRejectionReason::TooLarge,
        }]
    );
}

#[test]
fn reconcile_removes_unregistered_files_and_reports_missing_or_resized_rows() {
    let (temporary, store, run_id) = fixture();
    let workspace = temporary.path().join("worktree");
    let source = artifact_source(&workspace);
    fs::create_dir_all(&source).expect("create artifact directory");
    fs::write(source.join("kept.md"), "kept").expect("write kept artifact");
    fs::write(source.join("gone.json"), "{}").expect("write gone artifact");
    let collected = store
        .collect(&run_id, &workspace, 11)
        .expect("collect artifacts");
    let run_directory = store.root().join(run_id.as_ref());
    fs::write(run_directory.join(".partial-stray"), "stray").expect("write stray file");
    fs::remove_file(run_directory.join("gone.json")).expect("remove artifact file");
    fs::write(run_directory.join("kept.md"), "kept but longer").expect("resize artifact");

    let mismatched = store
        .reconcile_run(&run_id, &collected.artifacts)
        .expect("reconcile artifacts");

    assert_eq!(
        mismatched,
        vec!["gone.json".to_string(), "kept.md".to_string()]
    );
    assert!(!run_directory.join(".partial-stray").exists());
    // Both registered files failed verification: the missing file and the resized file are
    // removed so the manifest and storage never point at a half-written artifact.
    assert_eq!(
        fs::read_dir(&run_directory)
            .expect("read run directory")
            .count(),
        0
    );
}

#[test]
fn delete_run_refuses_a_linked_storage_root() {
    let (temporary, store, run_id) = fixture();
    let outside = temporary.path().join("outside");
    let outside_run = outside.join(run_id.as_ref());
    fs::create_dir_all(&outside_run).expect("create outside run directory");
    let sentinel = outside_run.join("sentinel.md");
    fs::write(&sentinel, "keep").expect("write sentinel");
    link_directory(&outside, store.root());

    assert!(store.delete_run(&run_id).is_err());
    assert_eq!(fs::read(&sentinel).expect("sentinel survives"), b"keep");
}

#[test]
fn orphan_sweep_refuses_a_linked_storage_root() {
    let (temporary, store, _) = fixture();
    let outside = temporary.path().join("outside");
    let orphan_run = outside.join("run-2");
    fs::create_dir_all(&orphan_run).expect("create orphan run directory");
    let sentinel = orphan_run.join("sentinel.md");
    fs::write(&sentinel, "keep").expect("write sentinel");
    link_directory(&outside, store.root());

    assert!(
        store
            .remove_run_directories_not_in(&HashSet::new())
            .is_err()
    );
    assert_eq!(fs::read(&sentinel).expect("sentinel survives"), b"keep");
}

#[test]
fn read_artifact_refuses_a_linked_storage_root() {
    let (temporary, store, run_id) = fixture();
    let outside = temporary.path().join("outside");
    let outside_run = outside.join(run_id.as_ref());
    fs::create_dir_all(&outside_run).expect("create outside run directory");
    let outside_file = outside_run.join("report.md");
    fs::write(&outside_file, "outside").expect("write outside artifact");
    link_directory(&outside, store.root());

    let registered = ScheduledRunArtifact {
        relative_path: "report.md".to_string(),
        kind: ScheduledArtifactKind::Markdown,
        size_bytes: 7,
        registered_at: 1,
    };

    assert!(store.read_artifact(&run_id, &registered).is_err());
    assert_eq!(
        fs::read(&outside_file).expect("outside file survives"),
        b"outside"
    );
}

#[test]
fn reconcile_removes_a_file_with_no_registered_row() {
    let (_temporary, store, run_id) = fixture();
    let run_directory = store.root().join(run_id.as_ref());
    fs::create_dir_all(&run_directory).expect("create run directory");
    fs::write(run_directory.join("orphan.md"), "orphan").expect("write orphan");
    let registered = ScheduledRunArtifact {
        relative_path: "other.md".to_string(),
        kind: ScheduledArtifactKind::Markdown,
        size_bytes: 1,
        registered_at: 1,
    };

    let mismatched = store
        .reconcile_run(&run_id, std::slice::from_ref(&registered))
        .expect("reconcile artifacts");

    assert_eq!(mismatched, vec!["other.md".to_string()]);
    assert!(!run_directory.join("orphan.md").exists());
}

#[cfg(unix)]
#[test]
fn rejects_an_artifact_directory_that_is_a_symlink() {
    let (temporary, store, run_id) = fixture();
    let workspace = temporary.path().join("worktree");
    fs::create_dir_all(&workspace).expect("create workspace");
    let outside = temporary.path().join("outside");
    fs::create_dir_all(&outside).expect("create outside");
    std::os::unix::fs::symlink(&outside, artifact_source(&workspace))
        .expect("create symlinked artifact directory");

    let collected = store
        .collect(&run_id, &workspace, 12)
        .expect("collect artifacts");

    assert_eq!(
        collected.rejections,
        vec![ScheduledArtifactRejection {
            name: SCHEDULED_ARTIFACT_DIRECTORY.to_string(),
            reason: ScheduledArtifactRejectionReason::Symlink,
        }]
    );
}
