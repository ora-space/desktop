use crate::{Backend, test_backend::backend_paths};
use ora_contracts::{CreateSkillRequest, UpdateSkillRequest};
use pretty_assertions::assert_eq;
use std::fs;
use tempfile::TempDir;

/// Verifies an update rewrites only the manifest and preserves other package files.
#[test]
fn update_preserves_other_package_files() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let app_data_directory = temporary.path().join("app-data");
        let home_directory = temporary.path().join("ora-home");
        let skills_root = app_data_directory.join("atoms").join("skills");
        let backend = Backend::open(backend_paths(&app_data_directory, &home_directory))
            .expect("open shared backend");

        let skill = backend
            .skills()
            .create(CreateSkillRequest {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                content: None,
            })
            .expect("create skill")
            .skill;
        // A user-added package file must survive an ordinary update.
        fs::create_dir_all(skills_root.join("review")).expect("create package directory");
        fs::write(skills_root.join("review").join("helper.sh"), "echo hi")
            .expect("write helper file");

        let updated = backend
            .skills()
            .update(UpdateSkillRequest {
                skill_id: skill.id,
                name: "review".to_string(),
                description: "Reviews pull requests".to_string(),
                content: None,
            })
            .expect("update skill")
            .skill;
        assert_eq!(updated.description, "Reviews pull requests");
        assert!(skills_root.join("review").join("helper.sh").is_file());
        let manifest =
            fs::read_to_string(skills_root.join("review").join("SKILL.md")).expect("read manifest");
        assert!(manifest.contains("description: Reviews pull requests"));
        assert!(!home_directory.join("atoms").exists());
    });
}

/// The export mapping survives deletion while ordinary selectors stop showing the Skill.
#[test]
fn list_keeps_deleted_local_names_for_workflow_export() {
    ora_logging::with_trace_logging(|| {
        let temp = TempDir::new().unwrap();
        let backend = Backend::open(backend_paths(
            &temp.path().join("app"),
            &temp.path().join("home"),
        ))
        .unwrap();
        let skill = backend
            .skills()
            .create(CreateSkillRequest {
                name: "review".to_string(),
                description: "test".to_string(),
                content: None,
            })
            .unwrap()
            .skill;
        backend
            .skills()
            .delete(ora_contracts::DeleteSkillRequest {
                skill_id: skill.id.clone(),
            })
            .unwrap();
        let response = backend
            .skills()
            .list(ora_contracts::ListSkillsRequest {})
            .unwrap();
        assert_eq!(response.skills, vec![]);
        assert_eq!(response.retired_skill_ids, Some(vec![skill.id.clone()]));
        assert_eq!(
            response.local_skill_names_for_export,
            Some(std::collections::BTreeMap::from([(skill.id, skill.name)]))
        );
    });
}
