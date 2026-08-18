use super::{
    NoopSkillImportProgressPublisher, SkillImportConfig, SkillImportError, SkillImportIdGenerator,
    SkillImportService,
};
use crate::skill::{
    BACKUP_DIR_NAME, DeleteSkillHandler, FilesystemSkillStorage, JOURNAL_DIR_NAME, STAGING_DIR_NAME,
};
use crate::{Clock, RepositoryError, SkillRepository};
use ora_contracts::{
    CommitSkillImportRequest, DeleteSkillRequest, GetSkillImportSessionRequest,
    PrepareSkillImportRequest, SkillImportConflictDecision, SkillImportDecision,
    SkillImportSession, SkillImportSessionStatus, SkillImportSource,
};
use ora_domain::{AuditFields, Namespace, Skill, SkillId};
use pretty_assertions::assert_eq;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Writes one minimal SKILL.md manifest with the given name and description.
fn write_manifest(dir: &Path, relative: &str, name: &str, description: &str) {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("---\nname: {name}\ndescription: {description}\n---\nBody.\n"),
    )
    .unwrap();
}

/// Writes one ordinary file under a skill folder.
fn write_file(dir: &Path, relative: &str, content: &str) {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Builds a `.zip` archive from a source folder.
fn build_zip(source: &Path, destination: &Path) {
    let file = fs::File::create(destination).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for entry in walk_files(source) {
        let relative = entry
            .strip_prefix(source)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if entry.is_dir() {
            writer
                .add_directory(format!("{relative}/"), options)
                .unwrap();
        } else {
            writer.start_file(relative, options).unwrap();
            writer.write_all(&fs::read(&entry).unwrap()).unwrap();
        }
    }
    writer.finish().unwrap();
}

/// Walks a source tree returning files and directories in deterministic order.
fn walk_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        entries.push(path.clone());
        if path.is_dir() {
            entries.extend(walk_files(&path));
        }
    }
    entries
}

/// Builds a `.tar.gz` archive from a source folder.
fn build_tar_gz(source: &Path, destination: &Path) {
    let file = fs::File::create(destination).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for entry in walk_files(source) {
        let relative = entry
            .strip_prefix(source)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if entry.is_dir() {
            builder.append_dir(&relative, ".").unwrap();
        } else {
            let content = fs::read(&entry).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, &relative, content.as_slice())
                .unwrap();
        }
    }
    let encoder = builder.into_inner().unwrap();
    encoder.finish().unwrap();
}

type TestService = SkillImportService<
    Arc<FakeSkillRepository>,
    FilesystemSkillStorage,
    SequentialIdGenerator,
    TestClock,
    NoopSkillImportProgressPublisher,
>;

/// Builds one import service over a fake repository and real filesystem storage.
fn test_service(
    repository: Arc<FakeSkillRepository>,
    temp_dir: &TempDir,
    idle_timeout: Duration,
) -> (TestService, Arc<TestClock>) {
    let clock = Arc::new(TestClock::new(1_000_000));
    let config = SkillImportConfig {
        temp_root: temp_dir.path().join("os-temp"),
        limits: ora_skill_package::Limits::default(),
        idle_timeout,
        max_lifetime: Duration::from_secs(60),
        result_retention: Duration::from_secs(60),
        preparation_timeout: Duration::from_secs(30),
    };
    let service = SkillImportService::new(
        repository,
        FilesystemSkillStorage::new(temp_dir.path().join("atoms").join("skills")),
        SequentialIdGenerator::default(),
        (*clock).clone(),
        NoopSkillImportProgressPublisher,
        config,
    );
    (service, clock)
}

/// Creates the on-disk formal directory that backs one pre-seeded skill.
fn seed_formal_skill(temp_dir: &TempDir, name: &str, description: &str) {
    let dir = temp_dir.path().join("atoms").join("skills").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n"),
    )
    .unwrap();
}

/// Prepares a folder source and returns the prepared session.
fn prepare_folder(service: &TestService, source: &Path) -> SkillImportSession {
    service
        .prepare(PrepareSkillImportRequest {
            source: SkillImportSource::Folder {
                path: source.to_string_lossy().into_owned(),
            },
        })
        .unwrap()
        .session
}

/// Polls a session until the commit finishes or the timeout elapses.
fn wait_for_completion(service: &TestService, session_id: &str) -> SkillImportSession {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let session = service
            .get_session(GetSkillImportSessionRequest {
                session_id: session_id.to_string(),
            })
            .unwrap()
            .session;
        if session.status == SkillImportSessionStatus::Completed {
            return session;
        }
        if Instant::now() > deadline {
            panic!("commit did not complete in time");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn prepares_folder_with_ready_invalid_and_missing_manifest_cases() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_manifest(&source, "skills/alpha/SKILL.md", "alpha", "Alpha skill");
    write_file(&source, "skills/alpha/helper.txt", "x");
    fs::create_dir_all(source.join("skills").join("broken")).unwrap();
    fs::write(
        source.join("skills").join("broken").join("SKILL.md"),
        "not valid front matter",
    )
    .unwrap();

    let repository = FakeSkillRepository::new();
    let (service, _) = test_service(repository, &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);

    assert_eq!(session.status, SkillImportSessionStatus::Prepared);
    assert_eq!(session.candidates.len(), 2);

    let alpha = session
        .candidates
        .iter()
        .find(|candidate| candidate.name == "alpha")
        .unwrap();
    assert_eq!(
        alpha.status,
        ora_contracts::SkillImportCandidateStatus::Ready
    );
    assert_eq!(alpha.source_path, "skills/alpha/SKILL.md");
    assert_eq!(alpha.file_count, 2);
    let manifest_len =
        format!("---\nname: alpha\ndescription: Alpha skill\n---\nBody.\n").len() as u64;
    assert_eq!(alpha.total_size, manifest_len + 1);

    let broken = session
        .candidates
        .iter()
        .find(|candidate| candidate.name.is_empty())
        .unwrap();
    assert_eq!(
        broken.status,
        ora_contracts::SkillImportCandidateStatus::Invalid
    );
    assert_eq!(broken.error_code.as_deref(), Some("name_missing"));
}

#[test]
fn prepares_zip_and_tar_gz_archives() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_manifest(&source, "skills/alpha/SKILL.md", "alpha", "Alpha");
    write_manifest(&source, "skills/beta/SKILL.md", "beta", "Beta");

    let zip_path = temp.path().join("skills.zip");
    build_zip(&source, &zip_path);
    let repository = FakeSkillRepository::new();
    let (service, _) = test_service(repository, &temp, Duration::from_secs(30));
    let session = service
        .prepare(PrepareSkillImportRequest {
            source: SkillImportSource::Archive {
                path: zip_path.to_string_lossy().into_owned(),
                file_name: "skills.zip".to_string(),
            },
        })
        .unwrap()
        .session;
    assert_eq!(session.candidates.len(), 2);

    let tar_path = temp.path().join("skills.tar.gz");
    build_tar_gz(&source, &tar_path);
    let repository = FakeSkillRepository::new();
    let (service, _) = test_service(repository, &temp, Duration::from_secs(30));
    let session = service
        .prepare(PrepareSkillImportRequest {
            source: SkillImportSource::Archive {
                path: tar_path.to_string_lossy().into_owned(),
                file_name: "skills.tar.gz".to_string(),
            },
        })
        .unwrap()
        .session;
    assert_eq!(session.candidates.len(), 2);
}

#[test]
fn reports_manifest_not_found_and_unsupported_archives() {
    let temp = TempDir::new().unwrap();
    let empty = temp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    write_file(&empty, "readme.txt", "no skills here");

    let repository = FakeSkillRepository::new();
    let (service, _) = test_service(repository, &temp, Duration::from_secs(30));
    let error = service
        .prepare(PrepareSkillImportRequest {
            source: SkillImportSource::Folder {
                path: empty.to_string_lossy().into_owned(),
            },
        })
        .unwrap_err();
    assert_eq!(
        error,
        crate::ApplicationError::SkillImport(SkillImportError::SkillManifestNotFound)
    );

    let rar = temp.path().join("skills.rar");
    fs::write(&rar, b"RAR!").unwrap();
    let error = service
        .prepare(PrepareSkillImportRequest {
            source: SkillImportSource::Archive {
                path: rar.to_string_lossy().into_owned(),
                file_name: "skills.rar".to_string(),
            },
        })
        .unwrap_err();
    assert_eq!(
        error,
        crate::ApplicationError::SkillImport(SkillImportError::ArchiveFormatUnsupported)
    );
}

#[test]
fn rejects_duplicate_names_within_one_source() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write_manifest(&source, "a/SKILL.md", "review", "Reviews");
    write_manifest(&source, "b/SKILL.md", "REVIEW", "Reviews again");

    let repository = FakeSkillRepository::new();
    let (service, _) = test_service(repository, &temp, Duration::from_secs(30));
    let error = service
        .prepare(PrepareSkillImportRequest {
            source: SkillImportSource::Folder {
                path: source.to_string_lossy().into_owned(),
            },
        })
        .unwrap_err();

    match error {
        crate::ApplicationError::SkillImport(SkillImportError::DuplicateSkillNames {
            duplicates,
        }) => {
            assert_eq!(duplicates.len(), 1);
            assert_eq!(duplicates[0].name, "review");
            assert_eq!(duplicates[0].source_paths.len(), 2);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn previews_conflicts_with_existing_skills() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    repository.push_skill("skill-1", "review", "Existing review", 100, 100);
    seed_formal_skill(&temp, "review", "Existing review");

    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "review", "New review");
    let (service, _) = test_service(repository, &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);

    let candidate = &session.candidates[0];
    assert_eq!(
        candidate.status,
        ora_contracts::SkillImportCandidateStatus::Conflict
    );
    let existing = candidate.existing_skill.as_ref().unwrap();
    assert_eq!(existing.skill_id, "skill-1");
    assert_eq!(existing.description, "Existing review");
}

#[test]
fn commits_ready_skip_and_overwrite_candidates() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    repository.push_skill("skill-1", "review", "Existing review", 100, 100);
    seed_formal_skill(&temp, "review", "Existing review");

    let source = temp.path().join("source");
    write_manifest(&source, "alpha/SKILL.md", "alpha", "Alpha skill");
    write_file(&source, "alpha/tool.txt", "tool");
    write_manifest(&source, "review/SKILL.md", "review", "New review");
    write_file(&source, "review/new-file.md", "new content");
    write_manifest(&source, "beta/SKILL.md", "beta", "Beta skill");

    let (service, clock) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    let session_id = session.session_id.clone();

    let decisions = session
        .candidates
        .iter()
        .filter(|candidate| candidate.status == ora_contracts::SkillImportCandidateStatus::Conflict)
        .map(|candidate| SkillImportConflictDecision {
            candidate_id: candidate.candidate_id.clone(),
            decision: SkillImportDecision::Overwrite,
        })
        .collect::<Vec<_>>();
    let response = service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions,
        })
        .unwrap();
    assert_eq!(response.status, SkillImportSessionStatus::Committing);

    let completed = wait_for_completion(&service, &session_id);
    assert_eq!(completed.status, SkillImportSessionStatus::Completed);
    assert_eq!(completed.progress.total, 3);
    assert_eq!(completed.progress.processed, 3);

    let by_name = |name: &str| {
        completed
            .progress
            .results
            .iter()
            .find(|result| result.name == name)
            .unwrap()
            .status
            .clone()
    };
    assert_eq!(
        by_name("alpha"),
        ora_contracts::SkillImportResultStatus::Imported
    );
    assert_eq!(
        by_name("review"),
        ora_contracts::SkillImportResultStatus::Overwritten
    );
    assert_eq!(
        by_name("beta"),
        ora_contracts::SkillImportResultStatus::Imported
    );

    // The database now reflects all three skills, with the overwrite keeping its id.
    let skills = repository.snapshot();
    assert_eq!(skills.len(), 3);
    let review = repository
        .find_skill_by_name(&Namespace::local(), "review")
        .unwrap()
        .unwrap();
    assert_eq!(review.id.to_string(), "skill-1");
    assert_eq!(review.description, "New review");
    assert_eq!(review.audit_fields.created_at, 100);
    assert!(review.audit_fields.updated_at > 100);
    let _ = clock;

    // Formal directories and manifests exist on disk.
    let skills_root = temp.path().join("atoms").join("skills");
    for name in ["alpha", "review", "beta"] {
        assert!(skills_root.join(name).join("SKILL.md").is_file());
    }
    let review_manifest = fs::read_to_string(skills_root.join("review").join("SKILL.md")).unwrap();
    assert!(review_manifest.contains("name: review"));
    assert!(skills_root.join("review").join("new-file.md").is_file());
    assert!(!skills_root.join("review").join("SKILL.md.tmp").exists());
    // The old review content is gone (full replacement, no merge).
    assert!(
        !fs::read_to_string(skills_root.join("review").join("SKILL.md"))
            .unwrap()
            .contains("Existing review")
    );
}

#[test]
fn skips_conflict_candidates_on_skip_decision() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    repository.push_skill("skill-1", "review", "Existing review", 100, 100);
    seed_formal_skill(&temp, "review", "Existing review");

    let source = temp.path().join("source");
    write_manifest(&source, "review/SKILL.md", "review", "New review");
    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    let session_id = session.session_id.clone();

    service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: vec![SkillImportConflictDecision {
                candidate_id: session.candidates[0].candidate_id.clone(),
                decision: SkillImportDecision::Skip,
            }],
        })
        .unwrap();
    let completed = wait_for_completion(&service, &session_id);

    assert_eq!(
        completed.progress.results[0].status,
        ora_contracts::SkillImportResultStatus::Skipped
    );
    // The existing skill is untouched.
    assert_eq!(
        repository
            .find_skill_by_name(&Namespace::local(), "review")
            .unwrap()
            .unwrap()
            .description,
        "Existing review"
    );
    assert!(
        temp.path()
            .join("atoms")
            .join("skills")
            .join("review")
            .join("SKILL.md")
            .is_file()
    );
}

/// Guards the namespace split between committed skills and reserved transaction directories.
///
/// A skill named after a reserved directory used to be promoted onto that directory, leaving
/// startup reconciliation to delete the package while sweeping transaction leftovers. The name
/// must now be refused during preparation, before anything reaches the formal tree.
#[test]
fn rejects_skill_names_reserved_by_the_storage_layer() {
    for reserved in [STAGING_DIR_NAME, BACKUP_DIR_NAME, JOURNAL_DIR_NAME] {
        let temp = TempDir::new().unwrap();
        let repository = FakeSkillRepository::new();
        let source = temp.path().join("source");
        write_manifest(&source, "pkg/SKILL.md", reserved, "Collides with reserved");

        let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
        let session = prepare_folder(&service, &source);
        let session_id = session.session_id.clone();

        assert_eq!(session.candidates.len(), 1);
        assert_eq!(
            session.candidates[0].status,
            ora_contracts::SkillImportCandidateStatus::Invalid
        );
        assert_eq!(
            session.candidates[0].error_code.as_deref(),
            Some("name_invalid")
        );

        service
            .commit(CommitSkillImportRequest {
                session_id: session_id.clone(),
                decisions: vec![],
            })
            .unwrap();
        let completed = wait_for_completion(&service, &session_id);

        assert_eq!(
            completed.progress.results[0].status,
            ora_contracts::SkillImportResultStatus::Failed
        );
        // Neither the database nor the reserved directory may be touched by the refused import.
        assert_eq!(
            repository
                .find_skill_by_name(&Namespace::local(), reserved)
                .unwrap(),
            None
        );
        assert_eq!(
            temp.path()
                .join("atoms")
                .join("skills")
                .join(reserved)
                .join("SKILL.md")
                .is_file(),
            false
        );
    }
}

#[test]
fn rejects_skills_exceeding_the_per_skill_file_limit() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "alpha", "Alpha");
    write_file(&source, "extra.txt", "x");

    let clock = Arc::new(TestClock::new(1_000_000));
    let mut config = SkillImportConfig {
        temp_root: temp.path().join("os-temp"),
        limits: ora_skill_package::Limits::default(),
        idle_timeout: Duration::from_secs(30),
        max_lifetime: Duration::from_secs(60),
        result_retention: Duration::from_secs(60),
        preparation_timeout: Duration::from_secs(30),
    };
    config.limits.max_files_per_skill = 1;
    let service = SkillImportService::new(
        repository,
        FilesystemSkillStorage::new(temp.path().join("atoms").join("skills")),
        SequentialIdGenerator::default(),
        (*clock).clone(),
        NoopSkillImportProgressPublisher,
        config,
    );

    let error = service
        .prepare(PrepareSkillImportRequest {
            source: SkillImportSource::Folder {
                path: source.to_string_lossy().into_owned(),
            },
        })
        .unwrap_err();
    assert_eq!(
        error,
        crate::ApplicationError::SkillImport(SkillImportError::TooManyFiles { max_files: 1 })
    );
}

#[test]
fn rejects_more_skills_than_the_session_limit() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    let source = temp.path().join("source");
    write_manifest(&source, "alpha/SKILL.md", "alpha", "Alpha");
    write_manifest(&source, "beta/SKILL.md", "beta", "Beta");

    let clock = Arc::new(TestClock::new(1_000_000));
    let mut config = SkillImportConfig {
        temp_root: temp.path().join("os-temp"),
        limits: ora_skill_package::Limits::default(),
        idle_timeout: Duration::from_secs(30),
        max_lifetime: Duration::from_secs(60),
        result_retention: Duration::from_secs(60),
        preparation_timeout: Duration::from_secs(30),
    };
    config.limits.max_skills = 1;
    let service = SkillImportService::new(
        repository,
        FilesystemSkillStorage::new(temp.path().join("atoms").join("skills")),
        SequentialIdGenerator::default(),
        (*clock).clone(),
        NoopSkillImportProgressPublisher,
        config,
    );

    let error = service
        .prepare(PrepareSkillImportRequest {
            source: SkillImportSource::Folder {
                path: source.to_string_lossy().into_owned(),
            },
        })
        .unwrap_err();
    assert_eq!(
        error,
        crate::ApplicationError::SkillImport(SkillImportError::TooManySkills { max_skills: 1 })
    );
}

#[test]
fn reports_missing_decisions_before_commit() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    repository.push_skill("skill-1", "review", "Existing", 100, 100);
    seed_formal_skill(&temp, "review", "Existing");

    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "review", "New review");
    let (service, _) = test_service(repository, &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);

    let error = service
        .commit(CommitSkillImportRequest {
            session_id: session.session_id.clone(),
            decisions: vec![],
        })
        .unwrap_err();
    match error {
        crate::ApplicationError::SkillImport(SkillImportError::DecisionMissing {
            candidate_ids,
        }) => {
            assert_eq!(candidate_ids.len(), 1);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn marks_stale_conflict_when_target_changes_before_commit() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    repository.push_skill("skill-1", "review", "Existing review", 100, 100);
    seed_formal_skill(&temp, "review", "Existing review");

    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "review", "New review");
    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    let session_id = session.session_id.clone();

    // The target's updated_at changes between preview and commit.
    repository.push_skill("skill-2", "fresh", "Fresh skill", 200, 200);
    let existing = repository
        .find_skill_by_name(&Namespace::local(), "review")
        .unwrap()
        .unwrap();
    repository.replace(existing.id, "review", "Changed before commit", 300);

    let decisions = vec![SkillImportConflictDecision {
        candidate_id: session.candidates[0].candidate_id.clone(),
        decision: SkillImportDecision::Overwrite,
    }];
    service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions,
        })
        .unwrap();

    let completed = wait_for_completion(&service, &session_id);
    assert_eq!(
        completed.progress.results[0].status,
        ora_contracts::SkillImportResultStatus::StaleConflict
    );
    // The stale candidate must not have modified the target.
    assert_eq!(
        repository
            .find_skill_by_name(&Namespace::local(), "review")
            .unwrap()
            .unwrap()
            .description,
        "Changed before commit"
    );
}

#[test]
fn concurrent_same_name_imports_do_not_report_storage_failure() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    let source_a = temp.path().join("source-a");
    let source_b = temp.path().join("source-b");
    write_manifest(&source_a, "SKILL.md", "grilling", "First");
    write_manifest(&source_b, "SKILL.md", "grilling", "Second");

    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session_a = prepare_folder(&service, &source_a);
    let session_b = prepare_folder(&service, &source_b);
    assert_eq!(
        session_a.candidates[0].status,
        ora_contracts::SkillImportCandidateStatus::Ready
    );
    assert_eq!(
        session_b.candidates[0].status,
        ora_contracts::SkillImportCandidateStatus::Ready
    );

    service
        .commit(CommitSkillImportRequest {
            session_id: session_a.session_id.clone(),
            decisions: vec![],
        })
        .unwrap();
    service
        .commit(CommitSkillImportRequest {
            session_id: session_b.session_id.clone(),
            decisions: vec![],
        })
        .unwrap();

    let completed_a = wait_for_completion(&service, &session_a.session_id);
    let completed_b = wait_for_completion(&service, &session_b.session_id);
    let statuses = [
        completed_a.progress.results[0].status.clone(),
        completed_b.progress.results[0].status.clone(),
    ];
    let imported = statuses
        .iter()
        .filter(|status| **status == ora_contracts::SkillImportResultStatus::Imported)
        .count();
    let stale = statuses
        .iter()
        .filter(|status| **status == ora_contracts::SkillImportResultStatus::StaleConflict)
        .count();

    assert_eq!((imported, stale), (1, 1));
    assert_eq!(repository.snapshot().len(), 1);
    assert!(
        completed_a.progress.results[0].error_code.is_none()
            && completed_b.progress.results[0].error_code.is_none()
    );
}

#[test]
fn continues_after_individual_candidate_failure() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    let source = temp.path().join("source");
    write_manifest(&source, "alpha/SKILL.md", "alpha", "Alpha");
    write_manifest(&source, "beta/SKILL.md", "beta", "Beta");

    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    let session_id = session.session_id.clone();
    // Alpha commits first (sorted by source path) and its DB insert fails once.
    repository.fail_next_create();

    service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: vec![],
        })
        .unwrap();
    let completed = wait_for_completion(&service, &session_id);

    assert_eq!(completed.progress.total, 2);
    assert_eq!(completed.progress.processed, 2);
    let alpha = completed
        .progress
        .results
        .iter()
        .find(|result| result.name == "alpha")
        .unwrap();
    assert_eq!(alpha.status, ora_contracts::SkillImportResultStatus::Failed);
    assert_eq!(alpha.error_code.as_deref(), Some("skill_repository_error"));
    let beta = completed
        .progress
        .results
        .iter()
        .find(|result| result.name == "beta")
        .unwrap();
    assert_eq!(
        beta.status,
        ora_contracts::SkillImportResultStatus::Imported
    );
    // Alpha's failed transaction left no formal directory behind.
    assert!(
        !temp
            .path()
            .join("atoms")
            .join("skills")
            .join("alpha")
            .exists()
    );
}

#[test]
fn imports_over_an_untracked_package_when_the_name_is_unclaimed() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    seed_formal_skill(&temp, "review", "Untracked review");

    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "review", "Imported review");
    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    assert_eq!(
        session.candidates[0].status,
        ora_contracts::SkillImportCandidateStatus::Ready
    );

    service
        .commit(CommitSkillImportRequest {
            session_id: session.session_id.clone(),
            decisions: vec![],
        })
        .unwrap();
    let completed = wait_for_completion(&service, &session.session_id);
    let result = &completed.progress.results[0];
    assert_eq!(
        result.status,
        ora_contracts::SkillImportResultStatus::Imported
    );
    assert_eq!(repository.snapshot().len(), 1);
    assert!(
        fs::read_to_string(
            temp.path()
                .join("atoms")
                .join("skills")
                .join("review")
                .join("SKILL.md")
        )
        .unwrap()
        .contains("Imported review")
    );
}

#[test]
fn importing_over_an_unavailable_row_preserves_residual_sibling_files() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    repository.push_skill("skill-1", "review", "Reviews", 100, 100);
    let formal = temp.path().join("atoms").join("skills").join("review");
    fs::create_dir_all(formal.join("scripts")).unwrap();
    fs::write(formal.join("SKILL.md"), "---\nname: [unterminated").unwrap();
    fs::write(formal.join("scripts/helper.js"), "preserve me").unwrap();

    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "review", "Imported review");
    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    assert_eq!(
        session.candidates[0].status,
        ora_contracts::SkillImportCandidateStatus::Ready
    );

    service
        .commit(CommitSkillImportRequest {
            session_id: session.session_id.clone(),
            decisions: vec![],
        })
        .unwrap();
    let completed = wait_for_completion(&service, &session.session_id);

    assert_eq!(
        completed.progress.results[0].status,
        ora_contracts::SkillImportResultStatus::Imported
    );
    assert_eq!(repository.snapshot()[0].id.to_string(), "skill-1");
    assert_eq!(
        fs::read_to_string(formal.join("scripts/helper.js")).unwrap(),
        "preserve me"
    );
    assert!(
        fs::read_to_string(formal.join("SKILL.md"))
            .unwrap()
            .contains("Imported review")
    );
}

#[test]
fn imports_the_same_name_after_deleting_an_unavailable_skill() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    repository.push_skill("skill-1", "review", "Reviews", 100, 100);
    let leftover = temp.path().join("atoms").join("skills").join("review");
    fs::create_dir_all(&leftover).unwrap();
    fs::write(leftover.join("notes.md"), "stale leftover").unwrap();

    DeleteSkillHandler::new(
        repository.clone(),
        FilesystemSkillStorage::new(temp.path().join("atoms").join("skills")),
        TestClock::new(200),
    )
    .handle(DeleteSkillRequest {
        skill_id: "skill-1".to_string(),
    })
    .unwrap();
    assert!(repository.snapshot().is_empty());

    let source = temp.path().join("source");
    write_manifest(&source, "SKILL.md", "review", "Imported review");
    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    assert_eq!(
        session.candidates[0].status,
        ora_contracts::SkillImportCandidateStatus::Ready
    );

    service
        .commit(CommitSkillImportRequest {
            session_id: session.session_id.clone(),
            decisions: vec![],
        })
        .unwrap();
    let completed = wait_for_completion(&service, &session.session_id);
    assert_eq!(
        completed.progress.results[0].status,
        ora_contracts::SkillImportResultStatus::Imported
    );
    assert_eq!(repository.snapshot().len(), 1);
    assert_ne!(repository.snapshot()[0].id.to_string(), "skill-1");
    assert!(
        fs::read_to_string(
            temp.path()
                .join("atoms")
                .join("skills")
                .join("review")
                .join("SKILL.md")
        )
        .unwrap()
        .contains("Imported review")
    );
}

#[test]
fn commit_is_idempotent_and_rejects_different_decisions() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    repository.push_skill("skill-1", "review", "Existing", 100, 100);
    seed_formal_skill(&temp, "review", "Existing");

    let source = temp.path().join("source");
    write_manifest(&source, "alpha/SKILL.md", "alpha", "Alpha");
    write_manifest(&source, "review/SKILL.md", "review", "New review");

    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    let session_id = session.session_id.clone();
    let decisions = vec![SkillImportConflictDecision {
        candidate_id: session.candidates[1].candidate_id.clone(),
        decision: SkillImportDecision::Overwrite,
    }];

    service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: decisions.clone(),
        })
        .unwrap();
    let completed = wait_for_completion(&service, &session_id);
    let results = completed.progress.results.len();

    // Same decisions replay the stored results without writing again.
    let replay = service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: decisions.clone(),
        })
        .unwrap();
    assert_eq!(replay.status, SkillImportSessionStatus::Completed);
    assert_eq!(replay.progress.results.len(), results);
    assert_eq!(repository.snapshot().len(), 2);

    // Different decisions are rejected.
    let error = service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: vec![SkillImportConflictDecision {
                candidate_id: session.candidates[1].candidate_id.clone(),
                decision: SkillImportDecision::Skip,
            }],
        })
        .unwrap_err();
    assert_eq!(
        error,
        crate::ApplicationError::SkillImport(SkillImportError::AlreadyCommitted)
    );
}

#[test]
fn cancels_prepared_sessions_and_blocks_cancel_during_commit() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    let source = temp.path().join("source");
    write_manifest(&source, "alpha/SKILL.md", "alpha", "Alpha");

    let (service, _) = test_service(repository.clone(), &temp, Duration::from_secs(30));
    let session = prepare_folder(&service, &source);
    let session_id = session.session_id.clone();

    let cancelled = service
        .cancel(ora_contracts::CancelSkillImportRequest {
            session_id: session_id.clone(),
        })
        .unwrap();
    assert!(cancelled.cancelled);

    let commit_error = service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: vec![],
        })
        .unwrap_err();
    assert_eq!(
        commit_error,
        crate::ApplicationError::SkillImport(SkillImportError::SessionCancelled)
    );

    // A committing session rejects cancellation.
    let session = prepare_folder(&service, &source);
    service
        .commit(CommitSkillImportRequest {
            session_id: session.session_id.clone(),
            decisions: vec![],
        })
        .unwrap();
    let cancel_error = service
        .cancel(ora_contracts::CancelSkillImportRequest {
            session_id: session.session_id.clone(),
        })
        .unwrap_err();
    assert_eq!(
        cancel_error,
        crate::ApplicationError::SkillImport(SkillImportError::CommitInProgress)
    );
    wait_for_completion(&service, &session.session_id);
}

#[test]
fn expires_idle_sessions_on_access() {
    let temp = TempDir::new().unwrap();
    let repository = FakeSkillRepository::new();
    let source = temp.path().join("source");
    write_manifest(&source, "alpha/SKILL.md", "alpha", "Alpha");

    let (service, clock) = test_service(repository, &temp, Duration::from_millis(100));
    let session = prepare_folder(&service, &source);
    let session_id = session.session_id.clone();

    clock.advance(200);
    let error = service
        .get_session(GetSkillImportSessionRequest {
            session_id: session_id.clone(),
        })
        .unwrap_err();
    assert_eq!(
        error,
        crate::ApplicationError::SkillImport(SkillImportError::SessionExpired)
    );

    let commit_error = service
        .commit(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: vec![],
        })
        .unwrap_err();
    assert_eq!(
        commit_error,
        crate::ApplicationError::SkillImport(SkillImportError::SessionNotFound { session_id })
    );
}

/// In-memory skill repository shared across import and commit threads.
struct FakeSkillRepository {
    skills: Mutex<Vec<Skill>>,
    fail_create: std::sync::atomic::AtomicUsize,
}

impl FakeSkillRepository {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            skills: Mutex::new(Vec::new()),
            fail_create: std::sync::atomic::AtomicUsize::new(0),
        })
    }
    fn push_skill(&self, id: &str, name: &str, description: &str, created: i64, updated: i64) {
        let skill = Skill::new(
            SkillId::new(id),
            Namespace::local(),
            name,
            description,
            AuditFields::new(created, updated, false),
        )
        .unwrap();
        self.skills.lock().unwrap().push(skill);
    }
    fn replace(&self, id: SkillId, name: &str, description: &str, updated: i64) {
        let mut skills = self.skills.lock().unwrap();
        let existing = skills
            .iter_mut()
            .find(|skill| skill.id == id && !skill.audit_fields.is_deleted)
            .unwrap();
        existing.name = name.to_string();
        existing.description = description.to_string();
        existing.audit_fields.updated_at = updated;
    }
    fn fail_next_create(&self) {
        self.fail_create.fetch_add(1, Ordering::SeqCst);
    }
    fn snapshot(&self) -> Vec<Skill> {
        self.skills
            .lock()
            .unwrap()
            .iter()
            .filter(|skill| !skill.audit_fields.is_deleted)
            .cloned()
            .collect()
    }
}

impl SkillRepository for Arc<FakeSkillRepository> {
    fn create_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        if self.fail_create.load(Ordering::SeqCst) > 0 {
            self.fail_create.fetch_sub(1, Ordering::SeqCst);
            return Err(RepositoryError::new(std::io::Error::other(
                "injected create failure",
            )));
        }
        self.skills.lock().unwrap().push(skill.clone());
        Ok(skill)
    }
    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, RepositoryError> {
        Ok(self
            .skills
            .lock()
            .unwrap()
            .iter()
            .find(|skill| skill.id == *skill_id && !skill.audit_fields.is_deleted)
            .cloned())
    }
    fn find_skill_by_name(
        &self,
        namespace: &Namespace,
        name: &str,
    ) -> Result<Option<Skill>, RepositoryError> {
        Ok(self
            .skills
            .lock()
            .unwrap()
            .iter()
            .find(|skill| {
                skill.namespace == *namespace
                    && !skill.audit_fields.is_deleted
                    && skill.name.eq_ignore_ascii_case(name)
            })
            .cloned())
    }
    fn list_skills(&self) -> Result<Vec<Skill>, RepositoryError> {
        Ok(self.snapshot())
    }
    fn update_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        let mut skills = self.skills.lock().unwrap();
        if let Some(existing) = skills
            .iter_mut()
            .find(|existing| existing.id == skill.id && !existing.audit_fields.is_deleted)
        {
            *existing = skill.clone();
            Ok(skill)
        } else {
            Err(RepositoryError::new(std::io::Error::other("skill missing")))
        }
    }
    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        if let Some(skill) = self
            .skills
            .lock()
            .unwrap()
            .iter_mut()
            .find(|skill| skill.id == *skill_id && !skill.audit_fields.is_deleted)
        {
            skill.audit_fields.updated_at = deleted_at;
            skill.audit_fields.is_deleted = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Produces deterministic unique identifiers for sessions and candidates.
#[derive(Clone, Default)]
struct SequentialIdGenerator {
    next: Arc<AtomicU64>,
}

impl SkillImportIdGenerator for SequentialIdGenerator {
    fn generate_import_id(&self) -> String {
        let value = self.next.fetch_add(1, Ordering::SeqCst);
        format!("id-{value}")
    }
}

/// A clock tests can advance explicitly.
#[derive(Clone)]
struct TestClock {
    now: Arc<AtomicI64>,
}

impl TestClock {
    fn new(now: i64) -> Self {
        Self {
            now: Arc::new(AtomicI64::new(now)),
        }
    }
    fn advance(&self, milliseconds: i64) {
        self.now.fetch_add(milliseconds, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}
