use super::{
    CreateSkillHandler, DeleteSkillHandler, GetSkillHandler, SkillIdGenerator, SkillPackageStore,
    SkillPackageStoreError, SkillRepository, SkillRepositoryError, UpdateSkillHandler,
};
use crate::{ApplicationError, Clock};
use ora_contracts::{CreateSkillRequest, DeleteSkillRequest, GetSkillRequest, UpdateSkillRequest};
use ora_domain::{AuditFields, Skill, SkillId};
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

#[test]
fn creates_trimmed_skill_with_generated_id_and_private_audit_fields() {
    let repository = Rc::new(FakeSkillRepository::default());
    let response =
        CreateSkillHandler::new(repository.clone(), FixedSkillIdGenerator, FixedClock(10))
            .handle(CreateSkillRequest {
                name: " review ".to_string(),
                description: "Reviews changes".to_string(),
            })
            .unwrap();

    assert_eq!(response.skill.id, "skill-1");
    assert_eq!(response.skill.name, "review");
    assert_eq!(
        repository.skills.borrow().clone(),
        vec![skill("skill-1", "review", "Reviews changes", 10, 10, false)]
    );
}

#[test]
fn updates_by_id_and_preserves_identity_and_creation_time() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 10, 20, false,
    )]));
    let response = UpdateSkillHandler::new(repository.clone(), FixedClock(30))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: " code review ".to_string(),
            description: "Reviews code".to_string(),
        })
        .unwrap();

    assert_eq!(response.skill.id, "skill-1");
    assert_eq!(
        repository.skills.borrow().clone(),
        vec![skill(
            "skill-1",
            "code review",
            "Reviews code",
            10,
            30,
            false
        )]
    );
}

#[test]
fn reports_blank_name_not_found_and_repository_errors() {
    let blank = CreateSkillHandler::new(
        Rc::new(FakeSkillRepository::default()),
        FixedSkillIdGenerator,
        FixedClock(1),
    )
    .handle(CreateSkillRequest {
        name: " ".to_string(),
        description: "Invalid".to_string(),
    })
    .unwrap_err();
    let missing = GetSkillHandler::new(Rc::new(FakeSkillRepository::default()))
        .handle(GetSkillRequest {
            skill_id: "missing".to_string(),
        })
        .unwrap_err();
    let failing = Rc::new(FakeSkillRepository::default());
    failing.fail_next(SkillRepositoryError::OperationFailed(
        "unavailable".to_string(),
    ));
    let repository_error = GetSkillHandler::new(failing)
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
            message: "unavailable".to_string()
        }
    );
}

#[test]
fn soft_delete_hides_a_skill_by_id() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    DeleteSkillHandler::new(repository.clone(), NoopSkillPackageStore, FixedClock(2))
        .handle(DeleteSkillRequest {
            skill_id: "skill-1".to_string(),
        })
        .unwrap();

    assert_eq!(
        GetSkillHandler::new(repository).handle(GetSkillRequest {
            skill_id: "skill-1".to_string()
        }),
        Err(ApplicationError::SkillNotFound {
            skill_id: "skill-1".to_string()
        })
    );
}

#[derive(Default)]
struct FakeSkillRepository {
    skills: RefCell<Vec<Skill>>,
    next_error: RefCell<Option<SkillRepositoryError>>,
}

impl FakeSkillRepository {
    fn with_skills(skills: Vec<Skill>) -> Self {
        Self {
            skills: RefCell::new(skills),
            next_error: RefCell::new(None),
        }
    }
    fn fail_next(&self, error: SkillRepositoryError) {
        self.next_error.replace(Some(error));
    }
    fn take_error(&self) -> Result<(), SkillRepositoryError> {
        self.next_error.borrow_mut().take().map_or(Ok(()), Err)
    }
}

impl SkillRepository for Rc<FakeSkillRepository> {
    fn create_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError> {
        self.take_error()?;
        self.skills.borrow_mut().push(skill.clone());
        Ok(skill)
    }
    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, SkillRepositoryError> {
        self.take_error()?;
        Ok(self
            .skills
            .borrow()
            .iter()
            .find(|skill| skill.id == *skill_id && !skill.audit_fields.is_deleted)
            .cloned())
    }
    fn list_skills(&self) -> Result<Vec<Skill>, SkillRepositoryError> {
        self.take_error()?;
        Ok(self
            .skills
            .borrow()
            .iter()
            .filter(|skill| !skill.audit_fields.is_deleted)
            .cloned()
            .collect())
    }
    fn update_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError> {
        self.take_error()?;
        let mut skills = self.skills.borrow_mut();
        if let Some(existing) = skills
            .iter_mut()
            .find(|existing| existing.id == skill.id && !existing.audit_fields.is_deleted)
        {
            *existing = skill.clone();
            Ok(skill)
        } else {
            Err(SkillRepositoryError::OperationFailed(
                "skill missing".to_string(),
            ))
        }
    }
    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, SkillRepositoryError> {
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

/// Skill package store that performs no filesystem work for CRUD tests that ignore on-disk folders.
struct NoopSkillPackageStore;
impl SkillPackageStore for NoopSkillPackageStore {
    fn create_staging(&self, _skill_id: &SkillId) -> Result<(), SkillPackageStoreError> {
        Ok(())
    }
    fn write_file(
        &self,
        _skill_id: &SkillId,
        _relative_path: &Path,
        _bytes: &[u8],
    ) -> Result<(), SkillPackageStoreError> {
        Ok(())
    }
    fn read_manifest(&self, _skill_id: &SkillId) -> Result<Option<String>, SkillPackageStoreError> {
        Ok(None)
    }
    fn committed_exists(&self, _name: &str) -> Result<bool, SkillPackageStoreError> {
        Ok(false)
    }
    fn promote(&self, _skill_id: &SkillId, _name: &str) -> Result<(), SkillPackageStoreError> {
        Ok(())
    }
    fn discard_staging(&self, _skill_id: &SkillId) -> Result<(), SkillPackageStoreError> {
        Ok(())
    }
    fn remove_committed(&self, _name: &str) -> Result<(), SkillPackageStoreError> {
        Ok(())
    }
    fn remove_all_staging(&self) -> Result<(), SkillPackageStoreError> {
        Ok(())
    }
    fn list_committed_names(&self) -> Result<Vec<String>, SkillPackageStoreError> {
        Ok(Vec::new())
    }
}
