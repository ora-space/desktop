use super::{
    CreateSkillHandler, DeleteSkillHandler, FilesystemSkillStorage, GetSkillHandler,
    ListSkillsHandler, SkillIdGenerator, SkillRepository, SkillStorage, SkillStorageError,
    UpdateSkillHandler,
};
use crate::skill::storage::{CreateHandle, DeleteHandle, SwapHandle, TransactionJournal};
use crate::{ApplicationError, Clock, RepositoryError};
use ora_contracts::{
    CreateSkillRequest, DeleteSkillRequest, GetSkillRequest, ListSkillsRequest, UpdateSkillRequest,
};
use ora_domain::{AuditFields, Namespace, Skill, SkillId};
use ora_skill_package::manifest::{render_manifest, render_minimal_manifest};
use ora_utils::path::StrictRelativePath;
use pretty_assertions::assert_eq;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tempfile::TempDir;

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
fn update_rejects_non_slug_name_without_mutating_catalog_or_package() {
    let original = skill("skill-1", "review", "Reviews", 10, 20, false);
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![original.clone()]));
    let storage = Rc::new(FakeSkillStorage::with_manifest("review", "Reviews"));
    let original_manifest = storage.manifest("review");

    let error = UpdateSkillHandler::new(repository.clone(), storage.clone(), FixedClock(30))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: "bad/name".to_string(),
            description: "Invalid".to_string(),
            content: None,
        })
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillNameInvalid {
            name: "bad/name".to_string()
        }
    );
    assert_eq!(repository.skills.borrow().as_slice(), &[original]);
    assert_eq!(storage.manifest("review"), original_manifest);
    assert!(!storage.formal_exists("bad/name"));
}

#[test]
fn update_advances_timestamp_past_the_previous_database_version() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 10, 20, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::with_manifest("review", "Reviews"));

    UpdateSkillHandler::new(repository.clone(), storage, FixedClock(20))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: "review".to_string(),
            description: "Updated".to_string(),
            content: None,
        })
        .unwrap();

    assert_eq!(repository.skills.borrow()[0].audit_fields.updated_at, 21);
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
    let storage = Rc::new(FakeSkillStorage::with_manifest("review", "Reviews"));
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
fn reports_folder_conflict_when_rename_target_directory_exists() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 10, 20, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::with_manifest("review", "Reviews"));
    storage
        .formal
        .borrow_mut()
        .insert("grilling".to_string(), Vec::new());

    let error = UpdateSkillHandler::new(repository, storage, FixedClock(30))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: "grilling".to_string(),
            description: "Reviews".to_string(),
            content: None,
        })
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillFolderConflict {
            name: "grilling".to_string()
        }
    );
}

#[test]
fn maps_existing_formal_directory_to_folder_conflict() {
    assert_eq!(
        ApplicationError::from_skill_storage_error(SkillStorageError::FormalDirectoryExists {
            name: "grilling".to_string(),
        }),
        ApplicationError::SkillFolderConflict {
            name: "grilling".to_string(),
        }
    );
    assert_eq!(
        ApplicationError::from_skill_storage_error(SkillStorageError::FormalDirectoryMissing {
            name: "grilling".to_string(),
        }),
        ApplicationError::SkillStorageInconsistent {
            name: "grilling".to_string(),
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
fn recreates_a_deleted_unavailable_skill_under_the_same_name() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::default());
    DeleteSkillHandler::new(repository.clone(), storage.clone(), FixedClock(2))
        .handle(DeleteSkillRequest {
            skill_id: "skill-1".to_string(),
        })
        .unwrap();

    let created = CreateSkillHandler::new(
        repository.clone(),
        storage.clone(),
        RecreatedSkillIdGenerator,
        FixedClock(3),
    )
    .handle(CreateSkillRequest {
        name: "review".to_string(),
        description: "Reviews again".to_string(),
        content: None,
    })
    .unwrap();

    assert_eq!(created.skill.id, "skill-2");
    assert_eq!(
        created.skill.availability,
        ora_contracts::SkillAvailability::Available
    );
    assert!(storage.formal_exists("review"));
}

#[test]
fn reports_unavailable_skills_and_restores_them_by_same_name() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![
        skill("skill-1", "review", "Reviews", 1, 1, false),
        skill("skill-2", "kept", "Kept", 1, 1, false),
    ]));
    let storage = Rc::new(FakeSkillStorage::with_manifest("kept", "Kept"));

    let listed = ListSkillsHandler::new(repository.clone(), storage.clone())
        .handle(ListSkillsRequest {})
        .unwrap();
    assert_eq!(
        listed
            .skills
            .iter()
            .map(|skill| (skill.name.as_str(), skill.availability))
            .collect::<Vec<_>>(),
        vec![
            ("review", ora_contracts::SkillAvailability::Unavailable),
            ("kept", ora_contracts::SkillAvailability::Available),
        ]
    );

    let details = GetSkillHandler::new(repository.clone(), storage.clone())
        .handle(GetSkillRequest {
            skill_id: "skill-1".to_string(),
        })
        .unwrap();
    assert_eq!(
        details.skill.availability,
        ora_contracts::SkillAvailability::Unavailable
    );
    assert_eq!(details.skill.content, "");

    let restored = CreateSkillHandler::new(
        repository.clone(),
        storage.clone(),
        FixedSkillIdGenerator,
        FixedClock(4),
    )
    .handle(CreateSkillRequest {
        name: "review".to_string(),
        description: "Restored".to_string(),
        content: None,
    })
    .unwrap();
    assert_eq!(restored.skill.id, "skill-1");
    assert_eq!(
        restored.skill.availability,
        ora_contracts::SkillAvailability::Available
    );
    assert!(storage.formal_exists("review"));
    assert_eq!(
        repository
            .find_skill(&ora_domain::SkillId::new("skill-1"))
            .unwrap()
            .unwrap()
            .description,
        "Restored"
    );

    DeleteSkillHandler::new(
        Rc::new(FakeSkillRepository::with_skills(vec![skill(
            "skill-3", "ghost", "Ghost", 1, 1, false,
        )])),
        Rc::new(FakeSkillStorage::default()),
        FixedClock(5),
    )
    .handle(DeleteSkillRequest {
        skill_id: "skill-3".to_string(),
    })
    .unwrap();
}

#[test]
fn treats_unreadable_manifests_as_unavailable_and_restores_them() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::default());
    storage
        .formal
        .borrow_mut()
        .insert("review".to_string(), b"---\nname: [unclosed".to_vec());

    let listed = ListSkillsHandler::new(repository.clone(), storage.clone())
        .handle(ListSkillsRequest {})
        .unwrap();
    assert_eq!(
        listed.skills[0].availability,
        ora_contracts::SkillAvailability::Unavailable
    );

    let details = GetSkillHandler::new(repository.clone(), storage.clone())
        .handle(GetSkillRequest {
            skill_id: "skill-1".to_string(),
        })
        .unwrap();
    assert_eq!(
        details.skill.availability,
        ora_contracts::SkillAvailability::Unavailable
    );
    assert_eq!(details.skill.content, "");

    let restored = CreateSkillHandler::new(
        repository,
        storage.clone(),
        FixedSkillIdGenerator,
        FixedClock(4),
    )
    .handle(CreateSkillRequest {
        name: "review".to_string(),
        description: "Restored".to_string(),
        content: None,
    })
    .unwrap();
    assert_eq!(restored.skill.id, "skill-1");
    assert_eq!(
        restored.skill.availability,
        ora_contracts::SkillAvailability::Available
    );
    assert_eq!(
        storage.manifest("review"),
        Some(render_minimal_manifest("review", "Restored").into_bytes())
    );
}

#[test]
fn restore_does_not_replace_an_untracked_complete_package() {
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    let storage = Rc::new(FakeSkillStorage::with_manifest("stray", "Untracked"));
    let original = storage.manifest("stray");

    let error = UpdateSkillHandler::new(repository, storage.clone(), FixedClock(4))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: "stray".to_string(),
            description: "Restored".to_string(),
            content: None,
        })
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::SkillNameConflict {
            namespace: "local".to_string(),
            name: "stray".to_string()
        }
    );
    assert_eq!(storage.manifest("stray"), original);
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
fn rolls_back_an_untracked_package_when_creating_over_it_fails() {
    let repository = Rc::new(FakeSkillRepository::default());
    let storage = Rc::new(FakeSkillStorage::with_manifest("review", "Untracked"));
    let original = storage.manifest("review");
    repository.fail_next(RepositoryError::new(std::io::Error::other("write failed")));

    let result = CreateSkillHandler::new(
        repository,
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
    assert_eq!(storage.manifest("review"), original);
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

#[test]
fn same_name_restore_preserves_residual_sibling_files() {
    let temp = TempDir::new().unwrap();
    let skills_root = temp.path().join("skills");
    create_residual_package(&skills_root, "review");
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    let storage = FilesystemSkillStorage::new(skills_root.clone());

    CreateSkillHandler::new(repository, storage, FixedSkillIdGenerator, FixedClock(2))
        .handle(CreateSkillRequest {
            name: "review".to_string(),
            description: "Restored".to_string(),
            content: None,
        })
        .unwrap();

    assert_eq!(
        fs::read_to_string(skills_root.join("review/scripts/helper.js")).unwrap(),
        "preserve me"
    );
}

#[test]
fn unavailable_rename_moves_the_whole_residual_package() {
    let temp = TempDir::new().unwrap();
    let skills_root = temp.path().join("skills");
    create_residual_package(&skills_root, "review");
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    let storage = FilesystemSkillStorage::new(skills_root.clone());

    UpdateSkillHandler::new(repository.clone(), storage, FixedClock(2))
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: "renamed".to_string(),
            description: "Restored".to_string(),
            content: None,
        })
        .unwrap();

    assert!(!skills_root.join("review").exists());
    assert_eq!(
        fs::read_to_string(skills_root.join("renamed/scripts/helper.js")).unwrap(),
        "preserve me"
    );
    assert_eq!(repository.skills.borrow()[0].name, "renamed");
}

#[test]
fn unavailable_rename_never_overwrites_an_existing_target_directory() {
    for (target_name, target_manifest) in [
        (
            "usable-target",
            render_minimal_manifest("usable-target", "Usable"),
        ),
        ("broken-target", "---\nname: [unterminated".to_string()),
    ] {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        create_residual_package(&skills_root, "review");
        let target = skills_root.join(target_name);
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), &target_manifest).unwrap();
        fs::write(target.join("owner.txt"), "keep target").unwrap();
        let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
            "skill-1", "review", "Reviews", 1, 1, false,
        )]));

        let error = UpdateSkillHandler::new(
            repository.clone(),
            FilesystemSkillStorage::new(skills_root.clone()),
            FixedClock(2),
        )
        .handle(UpdateSkillRequest {
            skill_id: "skill-1".to_string(),
            name: target_name.to_string(),
            description: "Restored".to_string(),
            content: None,
        })
        .unwrap_err();

        assert_eq!(
            error,
            ApplicationError::SkillNameConflict {
                namespace: "local".to_string(),
                name: target_name.to_string(),
            }
        );
        assert_eq!(repository.skills.borrow()[0].name, "review");
        assert!(skills_root.join("review").exists());
        assert_eq!(
            fs::read_to_string(target.join("owner.txt")).unwrap(),
            "keep target"
        );
    }
}

#[test]
fn unavailable_rename_rolls_back_package_when_repository_update_fails() {
    let temp = TempDir::new().unwrap();
    let skills_root = temp.path().join("skills");
    create_residual_package(&skills_root, "review");
    let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
        "skill-1", "review", "Reviews", 1, 1, false,
    )]));
    repository.fail_next_update(RepositoryError::new(std::io::Error::other("write failed")));

    let result = UpdateSkillHandler::new(
        repository.clone(),
        FilesystemSkillStorage::new(skills_root.clone()),
        FixedClock(2),
    )
    .handle(UpdateSkillRequest {
        skill_id: "skill-1".to_string(),
        name: "renamed".to_string(),
        description: "Restored".to_string(),
        content: None,
    });

    assert!(matches!(
        result,
        Err(ApplicationError::SkillRepository { .. })
    ));
    assert_eq!(repository.skills.borrow()[0].name, "review");
    assert!(skills_root.join("review").exists());
    assert!(!skills_root.join("renamed").exists());
    assert_eq!(
        fs::read_to_string(skills_root.join("review/scripts/helper.js")).unwrap(),
        "preserve me"
    );
}

#[test]
fn unavailable_rename_leaves_catalog_and_old_package_on_staging_or_swap_failure() {
    for fail_swap in [false, true] {
        let repository = Rc::new(FakeSkillRepository::with_skills(vec![skill(
            "skill-1", "review", "Reviews", 1, 1, false,
        )]));
        let storage = Rc::new(FakeSkillStorage::default());
        storage
            .formal
            .borrow_mut()
            .insert("review".to_string(), b"---\nname: [unterminated".to_vec());
        if fail_swap {
            storage.fail_next_swap();
        } else {
            storage.fail_next();
        }

        let result = UpdateSkillHandler::new(repository.clone(), storage.clone(), FixedClock(2))
            .handle(UpdateSkillRequest {
                skill_id: "skill-1".to_string(),
                name: "renamed".to_string(),
                description: "Restored".to_string(),
                content: None,
            });

        assert!(matches!(result, Err(ApplicationError::SkillStorage { .. })));
        assert_eq!(repository.skills.borrow()[0].name, "review");
        assert!(storage.formal_exists("review"));
        assert!(!storage.formal_exists("renamed"));
    }
}

fn create_residual_package(skills_root: &Path, name: &str) {
    let package = skills_root.join(name);
    fs::create_dir_all(package.join("scripts")).unwrap();
    fs::write(package.join("SKILL.md"), "---\nname: [unterminated").unwrap();
    fs::write(package.join("scripts/helper.js"), "preserve me").unwrap();
}

#[derive(Default)]
struct FakeSkillRepository {
    skills: RefCell<Vec<Skill>>,
    next_error: RefCell<Option<RepositoryError>>,
    next_update_error: RefCell<Option<RepositoryError>>,
}

impl FakeSkillRepository {
    fn with_skills(skills: Vec<Skill>) -> Self {
        Self {
            skills: RefCell::new(skills),
            next_error: RefCell::new(None),
            next_update_error: RefCell::new(None),
        }
    }
    fn fail_next(&self, error: RepositoryError) {
        self.next_error.replace(Some(error));
    }
    fn fail_next_update(&self, error: RepositoryError) {
        self.next_update_error.replace(Some(error));
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
        if let Some(error) = self.next_update_error.borrow_mut().take() {
            return Err(error);
        }
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
    backups: RefCell<HashMap<PathBuf, (String, Vec<u8>)>>,
    next_path: Cell<usize>,
    fail: Cell<bool>,
    fail_swap: Cell<bool>,
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
    fn fail_next_swap(&self) {
        self.fail_swap.replace(true);
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
    fn commit_create(
        &self,
        name: &str,
        _skill_id: &SkillId,
        staging: &Path,
    ) -> Result<CreateHandle, SkillStorageError> {
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
        _skill_id: &SkillId,
        _previous_updated_at: Option<i64>,
        staging: &Path,
    ) -> Result<SwapHandle, SkillStorageError> {
        self.take_fail()?;
        if self.fail_swap.replace(false) {
            return Err(SkillStorageError::OperationFailed {
                message: "fake swap failure".to_string(),
            });
        }
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
        let previous = self
            .formal
            .borrow()
            .get(from_name)
            .cloned()
            .unwrap_or_default();
        let backup = PathBuf::from(format!("/backup/{}", self.next_path.get()));
        self.next_path.set(self.next_path.get() + 1);
        self.backups
            .borrow_mut()
            .insert(backup.clone(), (from_name.to_string(), previous));
        let mut formal = self.formal.borrow_mut();
        if name != from_name {
            formal.remove(from_name);
        }
        formal.insert(name.to_string(), content);
        Ok(SwapHandle {
            name: name.to_string(),
            from_name: from_name.to_string(),
            staging: staging.to_path_buf(),
            backup,
            journal: PathBuf::from("/journal"),
        })
    }
    fn rollback_swap(&self, handle: &SwapHandle) -> Result<(), SkillStorageError> {
        let mut formal = self.formal.borrow_mut();
        formal.remove(&handle.name);
        if let Some((from_name, content)) = self.backups.borrow_mut().remove(&handle.backup) {
            formal.insert(from_name, content);
        }
        self.staged.borrow_mut().remove(&handle.staging);
        Ok(())
    }
    fn finish_swap(&self, _handle: &SwapHandle) -> Result<(), SkillStorageError> {
        Ok(())
    }
    fn commit_delete(
        &self,
        name: &str,
        _skill_id: &SkillId,
    ) -> Result<DeleteHandle, SkillStorageError> {
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
    fn remove_formal(&self, name: &str) -> Result<(), SkillStorageError> {
        self.formal.borrow_mut().remove(name);
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
struct RecreatedSkillIdGenerator;
impl SkillIdGenerator for RecreatedSkillIdGenerator {
    fn generate_skill_id(&self) -> SkillId {
        SkillId::new("skill-2")
    }
}
struct FixedClock(i64);
impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.0
    }
}
