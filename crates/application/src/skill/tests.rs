use super::{
    CreateSkillHandler, DeleteSkillHandler, GetSkillHandler, SkillIdGenerator, SkillRepository,
    SkillStorage, SkillStorageError, UpdateSkillHandler,
};
use crate::skill::storage::{CreateHandle, DeleteHandle, SwapHandle, TransactionJournal};
use crate::{ApplicationError, Clock, RepositoryError};
use ora_contracts::{CreateSkillRequest, DeleteSkillRequest, GetSkillRequest, UpdateSkillRequest};
use ora_domain::{AuditFields, Namespace, Skill, SkillId};
use ora_skill_package::manifest::{render_manifest, render_minimal_manifest};
use ora_utils::path::StrictRelativePath;
use pretty_assertions::assert_eq;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[test]
fn creates_trimmed_skill_with_generated_id_and_private_audit_fields() {
    let repository = Rc::new(FakeSkillRepository::default());
    let storage = Rc::new(FakeSkillStorage::default());
    let response = CreateSkillHandler::new(
        repository.clone(),
        storage.clone(),
        FixedSkillIdGenerator,
        FixedClock(10),
    )
    .handle(CreateSkillRequest {
        name: " review ".to_string(),
        description: "Reviews changes".to_string(),
        content: Some("# Skill body".to_string()),
    })
    .unwrap();

    assert_eq!(response.skill.id, "skill-1");
    assert_eq!(response.skill.name, "review");
    assert_eq!(
        repository.skills.borrow().clone(),
        vec![skill("skill-1", "review", "Reviews changes", 10, 10, false)]
    );
    assert_eq!(
        storage.manifest("review"),
        Some(render_manifest("review", "Reviews changes", "# Skill body").into_bytes())
    );
}

#[test]
fn updates_by_id_and_preserves_identity_and_creation_time() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 10, 20, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::with_manifest("review", "Reviews"));
    let response = UpdateSkillHandler::new(repository.clone(), storage.clone(), FixedClock(30))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: " code-review ".to_string(),
            description: "Reviews code".to_string(),
            content: Some("# Updated body".to_string()),
        })
        .unwrap();

    assert_eq!(response.skill.id, "skill-1");
    assert_eq!(
        repository.skills.borrow().clone(),
        vec![skill(
            "skill-1",
            "code-review",
            "Reviews code",
            10,
            30,
            false
        )]
    );
    assert!(!storage.formal_exists("review"));
    assert!(storage.formal_exists("code-review"));
    assert_eq!(
        storage.manifest("code-review"),
        Some(render_manifest("code-review", "Reviews code", "# Updated body").into_bytes())
    );
}

#[test]
fn preserves_or_clears_skill_body_without_losing_unknown_front_matter() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 10, 20, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::default());
    storage.formal.borrow_mut().insert(
        "review".to_string(),
        b"---\nname: review\ndescription: Reviews\ndepth: 3\n---\n# Existing body\n".to_vec(),
    );

    UpdateSkillHandler::new(repository.clone(), storage.clone(), FixedClock(30))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: "review".to_string(),
            description: "Updated".to_string(),
            content: None,
        })
        .unwrap();
    let preserved = String::from_utf8(storage.manifest("review").unwrap()).unwrap();
    assert!(preserved.contains("depth: 3"));
    assert!(preserved.contains("# Existing body"));

    UpdateSkillHandler::new(repository, storage.clone(), FixedClock(40))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: "review".to_string(),
            description: "Updated".to_string(),
            content: Some(String::new()),
        })
        .unwrap();
    let cleared = String::from_utf8(storage.manifest("review").unwrap()).unwrap();
    assert!(cleared.contains("depth: 3"));
    assert!(cleared.ends_with("---\n"));
    assert!(!cleared.contains("# Existing body"));
}
#[test]
fn reports_blank_name_not_found_and_repository_errors() {
    let blank = CreateSkillHandler::new(
        Rc::new(FakeSkillRepository::default()),
        Rc::new(FakeSkillStorage::default()),
        FixedSkillIdGenerator,
        FixedClock(1),
    )
    .handle(CreateSkillRequest {
        name: " ".to_string(),
        description: "Invalid".to_string(),
        content: None,
    })
    .unwrap_err();
    let missing = GetSkillHandler::new(
        Rc::new(FakeSkillRepository::default()),
        Rc::new(FakeSkillStorage::default()),
    )
    .handle(GetSkillRequest {
        skill_id: "missing".to_string(),
    })
    .unwrap_err();
    let failing = Rc::new(FakeSkillRepository::default());
    failing.fail_next(RepositoryError::new(std::io::Error::other("unavailable")));
    let repository_error = GetSkillHandler::new(failing, Rc::new(FakeSkillStorage::default()))
        .handle(GetSkillRequest {
            skill_id: "skill-1".to_string(),
        })
        .unwrap_err();

    assert_eq!(blank, ApplicationError::SkillNameBlank);
    assert_eq!(
        missing,
        ApplicationError::SkillNotFound {
            skill_id: "missing".to_string()
        }
    );
    assert_eq!(
        repository_error,
        ApplicationError::SkillRepository {
            source: RepositoryError::new(std::io::Error::other("unavailable"))
        }
    );
}

#[test]
fn rejects_non_slug_names_and_case_insensitive_conflicts() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::default());
    let handler = CreateSkillHandler::new(
        repository.clone(),
        storage.clone(),
        FixedSkillIdGenerator,
        FixedClock(2),
    );

    assert_eq!(
        handler
            .handle(CreateSkillRequest {
                name: "bad/name".to_string(),
                description: "Invalid".to_string(),
                content: None,
            })
            .unwrap_err(),
        ApplicationError::SkillNameInvalid {
            name: "bad/name".to_string()
        }
    );
    assert_eq!(
        handler
            .handle(CreateSkillRequest {
                name: "REVIEW".to_string(),
                description: "Duplicate".to_string(),
                content: None,
            })
            .unwrap_err(),
        ApplicationError::SkillNameConflict {
            namespace: "local".to_string(),
            name: "REVIEW".to_string()
        }
    );
    assert_eq!(
        handler
            .handle(CreateSkillRequest {
                name: "review".to_string(),
                description: "Too long".to_string(),
                content: None,
            })
            .unwrap_err(),
        ApplicationError::SkillNameConflict {
            namespace: "local".to_string(),
            name: "review".to_string()
        }
    );
}

#[test]
fn rejects_blank_and_oversized_descriptions() {
    let handler = CreateSkillHandler::new(
        Rc::new(FakeSkillRepository::default()),
        Rc::new(FakeSkillStorage::default()),
        FixedSkillIdGenerator,
        FixedClock(1),
    );

    assert_eq!(
        handler
            .handle(CreateSkillRequest {
                name: "review".to_string(),
                description: "   ".to_string(),
                content: None,
            })
            .unwrap_err(),
        ApplicationError::SkillDescriptionBlank
    );
    let oversized = "x".repeat(4097);
    assert_eq!(
        handler
            .handle(CreateSkillRequest {
                name: "review".to_string(),
                description: oversized,
                content: None,
            })
            .unwrap_err(),
        ApplicationError::SkillDescriptionTooLarge
    );
}

#[test]
fn soft_delete_hides_a_skill_by_id() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::with_manifest("review", "Reviews"));
    DeleteSkillHandler::new(repository.clone(), storage.clone(), FixedClock(2))
        .handle(DeleteSkillRequest {
            skill_id: "skill-1".to_string(),
        })
        .unwrap();

    assert_eq!(
        GetSkillHandler::new(repository, storage.clone()).handle(GetSkillRequest {
            skill_id: "skill-1".to_string()
        }),
        Err(ApplicationError::SkillNotFound {
            skill_id: "skill-1".to_string()
        })
    );
    assert!(!storage.formal_exists("review"));
}

#[test]
fn rolls_back_formal_directory_when_repository_persist_fails() {
    let repository = Rc::new(FakeSkillRepository::default());
    let storage = Rc::new(FakeSkillStorage::default());
    repository.fail_next(RepositoryError::new(std::io::Error::other("write failed")));

    let result = CreateSkillHandler::new(
        repository.clone(),
        storage.clone(),
        FixedSkillIdGenerator,
        FixedClock(1),
    )
    .handle(CreateSkillRequest {
        name: "review".to_string(),
        description: "Reviews".to_string(),
        content: None,
    });

    assert!(matches!(
        result,
        Err(ApplicationError::SkillRepository { .. })
    ));
    assert!(!storage.formal_exists("review"));
}

#[test]
fn surfaces_storage_failures_without_half_staging() {
    let repository = Rc::new(FakeSkillRepository::default());
    let storage = Rc::new(FakeSkillStorage::default());
    storage.fail_next();

    let result = CreateSkillHandler::new(
        repository.clone(),
        storage.clone(),
        FixedSkillIdGenerator,
        FixedClock(1),
    )
    .handle(CreateSkillRequest {
        name: "review".to_string(),
        description: "Reviews".to_string(),
        content: None,
    });

    assert!(matches!(result, Err(ApplicationError::SkillStorage { .. })));
    assert!(!storage.formal_exists("review"));
}

#[derive(Default)]
struct FakeSkillRepository {
    skills: RefCell<Vec<Skill>>,
    next_error: RefCell<Option<RepositoryError>>,
}

impl FakeSkillRepository {
    fn with_skills(skills: Vec<Skill>) -> Self {
        Self {
            skills: RefCell::new(skills),
            next_error: RefCell::new(None),
        }
    }
    fn fail_next(&self, error: RepositoryError) {
        self.next_error.replace(Some(error));
    }
    fn take_error(&self) -> Result<(), RepositoryError> {
        self.next_error.borrow_mut().take().map_or(Ok(()), Err)
    }
}

impl SkillRepository for Rc<FakeSkillRepository> {
    fn create_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        self.take_error()?;
        self.skills.borrow_mut().push(skill.clone());
        Ok(skill)
    }
    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, RepositoryError> {
        self.take_error()?;
        Ok(self
            .skills
            .borrow()
            .iter()
            .find(|skill| skill.id == *skill_id && !skill.audit_fields.is_deleted)
            .cloned())
    }
    fn find_skill_by_name(
        &self,
        namespace: &Namespace,
        name: &str,
    ) -> Result<Option<Skill>, RepositoryError> {
        self.take_error()?;
        Ok(self
            .skills
            .borrow()
            .iter()
            .find(|skill| {
                skill.namespace == *namespace
                    && !skill.audit_fields.is_deleted
                    && skill.name.eq_ignore_ascii_case(name)
            })
            .cloned())
    }
    fn list_skills(&self) -> Result<Vec<Skill>, RepositoryError> {
        self.take_error()?;
        Ok(self
            .skills
            .borrow()
            .iter()
            .filter(|skill| !skill.audit_fields.is_deleted)
            .cloned()
            .collect())
    }
    fn update_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        self.take_error()?;
        let mut skills = self.skills.borrow_mut();
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
        self.take_error()?;
        if let Some(skill) = self
            .skills
            .borrow_mut()
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

/// In-memory fake tracking formal manifests to exercise the atomic storage port.
#[derive(Default)]
struct FakeSkillStorage {
    formal: RefCell<HashMap<String, Vec<u8>>>,
    staged: RefCell<HashMap<PathBuf, Vec<u8>>>,
    next_path: Cell<usize>,
    fail: Cell<bool>,
}

impl FakeSkillStorage {
    fn with_manifest(name: &str, description: &str) -> Self {
        let storage = Self::default();
        storage.formal.borrow_mut().insert(
            name.to_string(),
            render_minimal_manifest(name, description).into_bytes(),
        );
        storage
    }
    fn manifest(&self, name: &str) -> Option<Vec<u8>> {
        self.formal.borrow().get(name).cloned()
    }
    fn fail_next(&self) {
        self.fail.replace(true);
    }
    fn take_fail(&self) -> Result<(), SkillStorageError> {
        if self.fail.replace(false) {
            Err(SkillStorageError::OperationFailed {
                message: "fake storage failure".to_string(),
            })
        } else {
            Ok(())
        }
    }
    fn next_staging(&self) -> PathBuf {
        let path = PathBuf::from(format!("/staging/{}", self.next_path.get()));
        self.next_path.set(self.next_path.get() + 1);
        path
    }
}

impl SkillStorage for Rc<FakeSkillStorage> {
    fn create_staging(&self) -> Result<PathBuf, SkillStorageError> {
        self.take_fail()?;
        let path = self.next_staging();
        self.staged.borrow_mut().insert(path.clone(), Vec::new());
        Ok(path)
    }
    fn stage_existing(&self, name: &str, staging: &Path) -> Result<(), SkillStorageError> {
        self.take_fail()?;
        let content = self.formal.borrow().get(name).cloned().ok_or_else(|| {
            SkillStorageError::FormalDirectoryMissing {
                name: name.to_string(),
            }
        })?;
        self.staged
            .borrow_mut()
            .insert(staging.to_path_buf(), content);
        Ok(())
    }
    fn write_file(
        &self,
        _staging: &Path,
        _relative: &StrictRelativePath,
        _bytes: &[u8],
    ) -> Result<(), SkillStorageError> {
        self.take_fail()
    }
    fn copy_file(
        &self,
        _staging: &Path,
        _relative: &StrictRelativePath,
        _source: &Path,
    ) -> Result<(), SkillStorageError> {
        self.take_fail()
    }
    fn write_manifest(&self, staging: &Path, content: &[u8]) -> Result<(), SkillStorageError> {
        self.take_fail()?;
        self.staged
            .borrow_mut()
            .insert(staging.to_path_buf(), content.to_vec());
        Ok(())
    }
    fn commit_create(&self, name: &str, staging: &Path) -> Result<CreateHandle, SkillStorageError> {
        self.take_fail()?;
        if self.formal.borrow().contains_key(name) {
            return Err(SkillStorageError::FormalDirectoryExists {
                name: name.to_string(),
            });
        }
        let content = self.staged.borrow_mut().remove(staging).unwrap_or_default();
        self.formal.borrow_mut().insert(name.to_string(), content);
        Ok(CreateHandle {
            name: name.to_string(),
            staging: staging.to_path_buf(),
            journal: PathBuf::from("/journal"),
        })
    }
    fn rollback_create(&self, handle: &CreateHandle) -> Result<(), SkillStorageError> {
        self.formal.borrow_mut().remove(&handle.name);
        self.staged.borrow_mut().remove(&handle.staging);
        Ok(())
    }
    fn finish_create(&self, _handle: &CreateHandle) -> Result<(), SkillStorageError> {
        Ok(())
    }
    fn commit_swap(
        &self,
        name: &str,
        from_name: &str,
        staging: &Path,
    ) -> Result<SwapHandle, SkillStorageError> {
        self.take_fail()?;
        if !self.formal.borrow().contains_key(from_name) {
            return Err(SkillStorageError::FormalDirectoryMissing {
                name: from_name.to_string(),
            });
        }
        if name != from_name && self.formal.borrow().contains_key(name) {
            return Err(SkillStorageError::FormalDirectoryExists {
                name: name.to_string(),
            });
        }
        let content = self.staged.borrow_mut().remove(staging).unwrap_or_default();
        let mut formal = self.formal.borrow_mut();
        if name != from_name {
            formal.remove(from_name);
        }
        formal.insert(name.to_string(), content);
        Ok(SwapHandle {
            name: name.to_string(),
            from_name: from_name.to_string(),
            staging: staging.to_path_buf(),
            backup: PathBuf::from("/backup"),
            journal: PathBuf::from("/journal"),
        })
    }
    fn rollback_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError> {
        let mut formal = self.formal.borrow_mut();
        formal.remove(&handle.name);
        if !formal.contains_key(&handle.from_name) {
            formal.insert(handle.from_name.clone(), Vec::new());
        }
        Ok(())
    }
    fn finish_swap(&self, _handle: &SwapHandle) -> Result<(), SkillStorageError> {
        Ok(())
    }
    fn commit_delete(&self, name: &str) -> Result<DeleteHandle, SkillStorageError> {
        self.take_fail()?;
        if !self.formal.borrow().contains_key(name) {
            return Err(SkillStorageError::FormalDirectoryMissing {
                name: name.to_string(),
            });
        }
        self.formal.borrow_mut().remove(name);
        Ok(DeleteHandle {
            name: name.to_string(),
            backup: PathBuf::from("/backup"),
            journal: PathBuf::from("/journal"),
        })
    }
    fn rollback_delete(&self, handle: &DeleteHandle) -> Result<(), SkillStorageError> {
        self.formal
            .borrow_mut()
            .entry(handle.name.clone())
            .or_default();
        Ok(())
    }
    fn finish_delete(&self, _handle: &DeleteHandle) -> Result<(), SkillStorageError> {
        Ok(())
    }
    fn formal_exists(&self, name: &str) -> bool {
        self.formal.borrow().contains_key(name)
    }
    fn read_manifest(&self, name: &str) -> Result<Option<Vec<u8>>, SkillStorageError> {
        Ok(self.formal.borrow().get(name).cloned())
    }
    fn list_formal_names(&self) -> Result<Vec<String>, SkillStorageError> {
        Ok(self.formal.borrow().keys().cloned().collect())
    }
    fn remove_temp(&self, _path: &Path) -> Result<(), SkillStorageError> {
        Ok(())
    }
    fn restore_backup(&self, _backup: &Path, _name: &str) -> Result<(), SkillStorageError> {
        Ok(())
    }
    fn remove_dir(&self, _path: &Path) -> Result<(), SkillStorageError> {
        Ok(())
    }
    fn list_journals(&self) -> Result<Vec<TransactionJournal>, SkillStorageError> {
        Ok(Vec::new())
    }
    fn remove_journal(&self, _journal: &TransactionJournal) -> Result<(), SkillStorageError> {
        Ok(())
    }
}

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
        Namespace::local(),
        name,
        description,
        AuditFields::new(created_at, updated_at, is_deleted),
    )
    .unwrap()
}

struct FixedSkillIdGenerator;
impl SkillIdGenerator for FixedSkillIdGenerator {
    fn generate_skill_id(&self) -> SkillId {
        SkillId::new("skill-1")
    }
}
struct FixedClock(i64);
impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.0
    }
}
