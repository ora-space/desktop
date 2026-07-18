use super::import::UploadedSkillFile;
use super::{
    DeleteSkillHandler, ImportSkillHandler, ReconcileSkillStorageHandler, SkillIdGenerator,
    SkillImportCommitError, SkillImportUnitOfWork, SkillPackageStore, SkillPackageStoreError,
    SkillRepository, SkillRepositoryError,
};
use crate::{ApplicationError, Clock};
use ora_contracts::{CreateSkillResponse, DeleteSkillRequest, Skill as ContractSkill};
use ora_domain::{AuditFields, Skill, SkillId};
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const MANIFEST: &str = "---\nname: grilling\ndescription: Grill the user\n---\nbody\n";

#[test]
fn imports_skill_and_promotes_staging_into_committed_directory() {
    let store = Rc::new(FakeSkillPackageStore::default());
    let unit_of_work = FakeUnitOfWork::default();
    let response = handler(store.clone(), unit_of_work.clone())
        .handle(vec![
            uploaded("SKILL.md", MANIFEST.as_bytes()),
            uploaded("refs/util.py", b"print('hi')"),
        ])
        .unwrap();

    assert_eq!(
        response,
        CreateSkillResponse {
            skill: ContractSkill {
                id: "skill-1".to_string(),
                name: "grilling".to_string(),
                description: "Grill the user".to_string(),
            },
        }
    );
    assert_eq!(
        unit_of_work.committed_rows(),
        vec![skill("skill-1", "grilling", "Grill the user", 7, 7, false)]
    );
    assert_eq!(
        store.committed_files("grilling"),
        Some(package(&[
            ("SKILL.md", MANIFEST.as_bytes()),
            ("refs/util.py", b"print('hi')")
        ]))
    );
    assert_eq!(store.staging_ids(), Vec::<String>::new());
}

#[test]
fn rejects_upload_without_manifest_and_discards_staging() {
    let store = Rc::new(FakeSkillPackageStore::default());
    let error = handler(store.clone(), FakeUnitOfWork::default())
        .handle(vec![uploaded("refs/util.py", b"noop")])
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillImportInvalid {
            reason: "skill upload is missing a SKILL.md manifest at its root".to_string(),
        }
    );
    assert_eq!(store.staging_ids(), Vec::<String>::new());
    assert_eq!(store.committed_names(), Vec::<String>::new());
}

#[test]
fn rejects_unsafe_manifest_name() {
    let store = Rc::new(FakeSkillPackageStore::default());
    let error = handler(store, FakeUnitOfWork::default())
        .handle(vec![uploaded(
            "SKILL.md",
            b"---\nname: review / guide\ndescription: x\n---\n",
        )])
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillImportInvalid {
            reason: "SKILL.md name `review / guide` is not a valid single-segment directory name"
                .to_string(),
        }
    );
}

#[test]
fn rejects_empty_unsafe_and_duplicate_uploads_before_staging() {
    let store = Rc::new(FakeSkillPackageStore::default());
    let empty = handler(store.clone(), FakeUnitOfWork::default())
        .handle(Vec::new())
        .unwrap_err();
    let unsafe_path = handler(store.clone(), FakeUnitOfWork::default())
        .handle(vec![uploaded("../escape", b"x")])
        .unwrap_err();
    let duplicate = handler(store.clone(), FakeUnitOfWork::default())
        .handle(vec![
            uploaded("SKILL.md", MANIFEST.as_bytes()),
            uploaded("SKILL.md", MANIFEST.as_bytes()),
        ])
        .unwrap_err();

    assert_eq!(
        empty,
        ApplicationError::SkillImportInvalid {
            reason: "skill upload contained no files".to_string(),
        }
    );
    assert_eq!(
        unsafe_path,
        ApplicationError::SkillImportInvalid {
            reason: "unsafe upload path `../escape`".to_string(),
        }
    );
    assert_eq!(
        duplicate,
        ApplicationError::SkillImportInvalid {
            reason: "duplicate upload path `SKILL.md`".to_string(),
        }
    );
    // None of the rejected uploads reached the staging phase.
    assert_eq!(store.staging_ids(), Vec::<String>::new());
}

#[test]
fn rejects_upload_exceeding_the_file_limit() {
    let store = Rc::new(FakeSkillPackageStore::default());
    let files = (0..1001)
        .map(|index| uploaded(&format!("file-{index}.txt"), b"x"))
        .collect();
    let error = handler(store, FakeUnitOfWork::default())
        .handle(files)
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillImportInvalid {
            reason: "skill upload exceeds the 1000-file limit".to_string(),
        }
    );
}

#[test]
fn reports_conflict_when_committed_directory_exists() {
    let store = Rc::new(FakeSkillPackageStore::default());
    store.seed_committed("grilling");
    let error = handler(store.clone(), FakeUnitOfWork::default())
        .handle(vec![uploaded("SKILL.md", MANIFEST.as_bytes())])
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillFolderConflict {
            name: "grilling".to_string(),
        }
    );
    assert_eq!(store.staging_ids(), Vec::<String>::new());
}

#[test]
fn rolls_back_row_and_discards_staging_when_promote_fails() {
    let store = Rc::new(FakeSkillPackageStore::default());
    store.fail_promote();
    let unit_of_work = FakeUnitOfWork::default();
    let error = handler(store.clone(), unit_of_work.clone())
        .handle(vec![uploaded("SKILL.md", MANIFEST.as_bytes())])
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillPackageStorage {
            message: "promote failed".to_string(),
        }
    );
    assert_eq!(unit_of_work.committed_rows(), Vec::<Skill>::new());
    assert_eq!(store.staging_ids(), Vec::<String>::new());
    assert_eq!(store.committed_names(), Vec::<String>::new());
}

#[test]
fn removes_promoted_directory_when_commit_fails() {
    let store = Rc::new(FakeSkillPackageStore::default());
    let unit_of_work = FakeUnitOfWork::default();
    unit_of_work.fail_commit();
    let error = handler(store.clone(), unit_of_work.clone())
        .handle(vec![uploaded("SKILL.md", MANIFEST.as_bytes())])
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillRepository {
            message: "commit failed".to_string(),
        }
    );
    assert_eq!(unit_of_work.committed_rows(), Vec::<Skill>::new());
    // The promote succeeded, so compensation must remove the committed directory again.
    assert_eq!(store.committed_names(), Vec::<String>::new());
}

#[test]
fn reconcile_clears_staging_and_orphan_directories_but_keeps_visible_skills() {
    let store = Rc::new(FakeSkillPackageStore::default());
    store.seed_staging("abandoned");
    store.seed_committed("keep");
    store.seed_committed("orphan");
    let repository = Rc::new(FakeSkillCatalog::with_skills(vec![skill(
        "skill-keep",
        "keep",
        "Kept skill",
        1,
        1,
        false,
    )]));

    ReconcileSkillStorageHandler::new(store.clone(), repository)
        .handle()
        .unwrap();

    assert_eq!(store.staging_ids(), Vec::<String>::new());
    assert_eq!(store.committed_names(), vec!["keep".to_string()]);
}

#[test]
fn delete_soft_deletes_the_row_and_removes_the_committed_folder() {
    let store = Rc::new(FakeSkillPackageStore::default());
    store.seed_committed("grilling");
    let repository = Rc::new(FakeSkillCatalog::with_skills(vec![skill(
        "skill-1", "grilling", "Grill", 1, 1, false,
    )]));

    DeleteSkillHandler::new(repository.clone(), store.clone(), FixedClock(9))
        .handle(DeleteSkillRequest {
            skill_id: "skill-1".to_string(),
        })
        .unwrap();

    assert_eq!(repository.list_skills().unwrap(), Vec::<Skill>::new());
    assert_eq!(store.committed_names(), Vec::<String>::new());
}

/// Builds an import handler over the shared fakes with a fixed identifier and clock.
fn handler(
    store: Rc<FakeSkillPackageStore>,
    unit_of_work: FakeUnitOfWork,
) -> ImportSkillHandler<Rc<FakeSkillPackageStore>, FakeUnitOfWork, FixedSkillIdGenerator, FixedClock>
{
    ImportSkillHandler::new(store, unit_of_work, FixedSkillIdGenerator, FixedClock(7))
}

/// Builds one uploaded file from a relative path and byte payload.
fn uploaded(relative_path: &str, bytes: &[u8]) -> UploadedSkillFile {
    UploadedSkillFile {
        relative_path: relative_path.to_string(),
        bytes: bytes.to_vec(),
    }
}

/// Builds a committed package map from relative paths so tests can deep-compare on-disk contents.
fn package(entries: &[(&str, &[u8])]) -> BTreeMap<PathBuf, Vec<u8>> {
    entries
        .iter()
        .map(|(path, bytes)| (PathBuf::from(path), bytes.to_vec()))
        .collect()
}

/// Builds a domain skill fixture for deep-equality assertions.
fn skill(
    id: &str,
    name: &str,
    description: &str,
    created_at: i64,
    updated_at: i64,
    is_deleted: bool,
) -> Skill {
    Skill::new(
        SkillId::new(id),
        name,
        description,
        AuditFields::new(created_at, updated_at, is_deleted),
    )
    .unwrap()
}

/// In-memory skill package store modeling staging and committed directories for handler tests.
#[derive(Default)]
struct FakeSkillPackageStore {
    state: RefCell<FakeStoreState>,
}

#[derive(Default)]
struct FakeStoreState {
    staging: BTreeMap<String, BTreeMap<PathBuf, Vec<u8>>>,
    committed: BTreeMap<String, BTreeMap<PathBuf, Vec<u8>>>,
    fail_promote: bool,
}

impl FakeSkillPackageStore {
    fn fail_promote(&self) {
        self.state.borrow_mut().fail_promote = true;
    }
    fn seed_staging(&self, id: &str) {
        self.state
            .borrow_mut()
            .staging
            .insert(format!("{id}.tmp"), BTreeMap::new());
    }
    fn seed_committed(&self, name: &str) {
        self.state
            .borrow_mut()
            .committed
            .insert(name.to_string(), BTreeMap::new());
    }
    fn staging_ids(&self) -> Vec<String> {
        self.state.borrow().staging.keys().cloned().collect()
    }
    fn committed_names(&self) -> Vec<String> {
        self.state.borrow().committed.keys().cloned().collect()
    }
    fn committed_files(&self, name: &str) -> Option<BTreeMap<PathBuf, Vec<u8>>> {
        self.state.borrow().committed.get(name).cloned()
    }
}

impl SkillPackageStore for Rc<FakeSkillPackageStore> {
    fn create_staging(&self, skill_id: &SkillId) -> Result<(), SkillPackageStoreError> {
        self.state
            .borrow_mut()
            .staging
            .insert(staging_key(skill_id), BTreeMap::new());
        Ok(())
    }
    fn write_file(
        &self,
        skill_id: &SkillId,
        relative_path: &Path,
        bytes: &[u8],
    ) -> Result<(), SkillPackageStoreError> {
        self.state
            .borrow_mut()
            .staging
            .entry(staging_key(skill_id))
            .or_default()
            .insert(relative_path.to_path_buf(), bytes.to_vec());
        Ok(())
    }
    fn read_manifest(&self, skill_id: &SkillId) -> Result<Option<String>, SkillPackageStoreError> {
        Ok(self
            .state
            .borrow()
            .staging
            .get(&staging_key(skill_id))
            .and_then(|files| files.get(Path::new("SKILL.md")))
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned()))
    }
    fn committed_exists(&self, name: &str) -> Result<bool, SkillPackageStoreError> {
        Ok(self.state.borrow().committed.contains_key(name))
    }
    fn promote(&self, skill_id: &SkillId, name: &str) -> Result<(), SkillPackageStoreError> {
        let mut state = self.state.borrow_mut();
        if state.fail_promote {
            return Err(SkillPackageStoreError::OperationFailed(
                "promote failed".to_string(),
            ));
        }
        let files = state
            .staging
            .remove(&staging_key(skill_id))
            .unwrap_or_default();
        state.committed.insert(name.to_string(), files);
        Ok(())
    }
    fn discard_staging(&self, skill_id: &SkillId) -> Result<(), SkillPackageStoreError> {
        self.state
            .borrow_mut()
            .staging
            .remove(&staging_key(skill_id));
        Ok(())
    }
    fn remove_committed(&self, name: &str) -> Result<(), SkillPackageStoreError> {
        self.state.borrow_mut().committed.remove(name);
        Ok(())
    }
    fn remove_all_staging(&self) -> Result<(), SkillPackageStoreError> {
        self.state.borrow_mut().staging.clear();
        Ok(())
    }
    fn list_committed_names(&self) -> Result<Vec<String>, SkillPackageStoreError> {
        Ok(self.state.borrow().committed.keys().cloned().collect())
    }
}

/// Derives the staging map key matching the store's `<id>.tmp` directory naming.
fn staging_key(skill_id: &SkillId) -> String {
    format!("{skill_id}.tmp")
}

/// In-memory import unit of work recording committed rows and simulating failure points.
#[derive(Clone, Default)]
struct FakeUnitOfWork {
    inner: Rc<FakeUnitOfWorkState>,
}

#[derive(Default)]
struct FakeUnitOfWorkState {
    committed: RefCell<Vec<Skill>>,
    fail_commit: RefCell<bool>,
}

impl FakeUnitOfWork {
    fn fail_commit(&self) {
        *self.inner.fail_commit.borrow_mut() = true;
    }
    fn committed_rows(&self) -> Vec<Skill> {
        self.inner.committed.borrow().clone()
    }
}

impl SkillImportUnitOfWork for FakeUnitOfWork {
    fn insert_then<OnInserted>(
        &self,
        skill: Skill,
        on_inserted: OnInserted,
    ) -> Result<(), SkillImportCommitError>
    where
        OnInserted: FnOnce() -> Result<(), String>,
    {
        // Mirror the real ordering: insert, run the promote callback, then commit. A recorded row
        // stands in for a committed row, so failures leave `committed` untouched.
        match on_inserted() {
            Ok(()) => {
                if *self.inner.fail_commit.borrow() {
                    return Err(SkillImportCommitError::CommitAfterPromote {
                        message: "commit failed".to_string(),
                    });
                }
                self.inner.committed.borrow_mut().push(skill);
                Ok(())
            }
            Err(message) => Err(SkillImportCommitError::Promote { message }),
        }
    }
}

/// Minimal skill catalog fake covering the lookup and soft-delete paths the tests exercise.
struct FakeSkillCatalog {
    skills: RefCell<Vec<Skill>>,
}

impl FakeSkillCatalog {
    fn with_skills(skills: Vec<Skill>) -> Self {
        Self {
            skills: RefCell::new(skills),
        }
    }
}

impl SkillRepository for Rc<FakeSkillCatalog> {
    fn create_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError> {
        self.skills.borrow_mut().push(skill.clone());
        Ok(skill)
    }
    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, SkillRepositoryError> {
        Ok(self
            .skills
            .borrow()
            .iter()
            .find(|skill| skill.id == *skill_id && !skill.audit_fields.is_deleted)
            .cloned())
    }
    fn list_skills(&self) -> Result<Vec<Skill>, SkillRepositoryError> {
        Ok(self
            .skills
            .borrow()
            .iter()
            .filter(|skill| !skill.audit_fields.is_deleted)
            .cloned()
            .collect())
    }
    fn update_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError> {
        Ok(skill)
    }
    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, SkillRepositoryError> {
        match self
            .skills
            .borrow_mut()
            .iter_mut()
            .find(|skill| skill.id == *skill_id && !skill.audit_fields.is_deleted)
        {
            Some(skill) => {
                skill.audit_fields.updated_at = deleted_at;
                skill.audit_fields.is_deleted = true;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Emits a fixed identifier so import tests can assert the generated skill id deterministically.
struct FixedSkillIdGenerator;
impl SkillIdGenerator for FixedSkillIdGenerator {
    fn generate_skill_id(&self) -> SkillId {
        SkillId::new("skill-1")
    }
}

/// Emits a fixed timestamp for deterministic audit fields.
struct FixedClock(i64);
impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.0
    }
}
