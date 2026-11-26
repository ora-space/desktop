use super::{CountedSkillRepository, SkillRevision};
use crate::skill::SkillApi;
use crate::skill_catalog::DesktopSkillCatalog;
use ora_agent_runtime::SkillCatalogSource as _;
use ora_application::SkillRepository as _;
use ora_contracts::{
    AppEvent, CommitSkillImportRequest, CreateSkillRequest, DeleteSkillRequest,
    GetSkillImportSessionRequest, PrepareSkillImportRequest, SkillImportSessionStatus,
    SkillImportSource, UpdateSkillRequest,
};
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, PluginSkillProjection, RepositoryPool,
    SqliteSkillRepository, default_migration_catalog,
};
use ora_domain::{AuditFields, Namespace, PluginId, Skill, SkillId, WorkspaceId};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::TempDir;

/// One set of Skills, the ways to change it, and what was announced about them.
struct Skills {
    directory: TempDir,
    pool: RepositoryPool,
    revision: SkillRevision,
    /// The revision at the latest announcement, which has to be the one it announces.
    announced: Arc<AtomicU64>,
}

impl Skills {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let pool = DatabaseBootstrapper::new(crate::test_clock::TestClock)
            .bootstrap_repository_pool(
                &DatabaseLocation::path(directory.path().join("ora.sqlite3")),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        let revision = SkillRevision::default();
        let announced = Arc::new(AtomicU64::new(0));
        revision.announce_with({
            let revision = revision.clone();
            let announced = announced.clone();
            move || announced.store(revision.current(), Ordering::Relaxed)
        });
        Self {
            directory,
            pool,
            revision,
            announced,
        }
    }

    fn root(&self) -> PathBuf {
        self.directory.path().join("skills")
    }

    fn api(&self) -> SkillApi {
        SkillApi::new(
            self.pool.clone(),
            self.root(),
            self.revision.clone(),
            crate::clock::SystemClock,
        )
    }

    fn repository(&self) -> CountedSkillRepository {
        CountedSkillRepository::new(
            SqliteSkillRepository::new(self.pool.clone()),
            self.revision.clone(),
        )
    }

    fn catalog(&self) -> DesktopSkillCatalog {
        DesktopSkillCatalog::new(
            self.pool.clone(),
            self.root(),
            self.directory.path(),
            self.revision.clone(),
        )
    }

    /// Returns how far the revision advanced since it was `before`, all of it announced.
    fn advanced_from(&self, before: u64) -> u64 {
        assert_eq!(
            self.announced.load(Ordering::Relaxed),
            self.revision.current()
        );
        self.revision.current() - before
    }
}

/// Asks for one Skill to be created.
fn create(name: &str) -> CreateSkillRequest {
    CreateSkillRequest {
        name: name.to_string(),
        description: "Reviews changes".to_string(),
        content: None,
    }
}

/// Every committed change through the settings is counted once, also one that leaves the catalog's
/// paths, names and descriptions alone or brings back an earlier state.
#[test]
fn each_committed_change_advances_the_revision() {
    ora_logging::with_trace_logging(|| {
        let skills = Skills::new();
        let api = skills.api();
        let mut before = skills.revision.current();
        let mut changed = |step: &str| {
            assert_eq!(skills.advanced_from(before), 1, "{step}");
            before = skills.revision.current();
        };

        let created = api.create(create("review")).unwrap().skill;
        changed("create");
        let update = |name: &str, content: Option<&str>| UpdateSkillRequest {
            skill_id: created.id.clone(),
            name: name.to_string(),
            description: "Reviews changes".to_string(),
            content: content.map(str::to_string),
        };
        api.update(update("review", Some("Other instructions")))
            .unwrap();
        changed("edit in place");
        api.update(update("audit", None)).unwrap();
        changed("rename");
        api.update(update("review", None)).unwrap();
        changed("rename back");
        api.delete(DeleteSkillRequest {
            skill_id: created.id.clone(),
        })
        .unwrap();
        changed("delete");
        api.create(create("review")).unwrap();
        changed("create again");
    });
}

/// What changed nothing is not counted, so nothing is discovered again for it.
#[test]
fn a_change_that_does_not_take_effect_leaves_the_revision_alone() {
    ora_logging::with_trace_logging(|| {
        let skills = Skills::new();
        let api = skills.api();
        api.create(create("review")).unwrap();
        let before = skills.revision.current();

        assert!(api.create(create("review")).is_err());
        assert!(
            api.update(UpdateSkillRequest {
                skill_id: "missing".to_string(),
                name: "audit".to_string(),
                description: "Audits".to_string(),
                content: None,
            })
            .is_err()
        );
        assert!(
            api.delete(DeleteSkillRequest {
                skill_id: "missing".to_string(),
            })
            .is_err()
        );
        let repository = skills.repository();
        assert!(
            !repository
                .soft_delete_skill(&SkillId::new("missing"), 1)
                .unwrap()
        );
        let stranger = Skill::new(
            SkillId::new("missing"),
            Namespace::local(),
            "audit",
            "Audits",
            AuditFields::new(1, 1, /*is_deleted*/ false),
        )
        .unwrap();
        assert!(repository.update_skill(stranger).is_err());
        // Settles every plugin that contributes no Skills, which is most of them.
        let plugin = PluginId::new("official", "agent");
        let plugin = plugin.unwrap();
        repository.remove_plugin_skills(&plugin, 1).unwrap();
        repository
            .replace_plugin_skills(&plugin, "1.0.0", &[], 1)
            .unwrap();
        api.list(ora_contracts::ListSkillsRequest {}).unwrap();

        assert_eq!(skills.advanced_from(before), 0);
    });
}

/// A created Skill is counted once its row is written, so nobody told about it can miss it, and not
/// at all when the write fails.
#[test]
fn a_create_is_counted_once_it_is_written() {
    ora_logging::with_trace_logging(|| {
        let skills = Skills::new();
        let rows = SqliteSkillRepository::new(skills.pool.clone());
        let revision = SkillRevision::default();
        let written_when_told = Arc::new(std::sync::Mutex::new(Vec::new()));
        revision.announce_with({
            let rows = rows.clone();
            let written_when_told = written_when_told.clone();
            move || {
                let written = rows
                    .find_skill_by_name(&Namespace::local(), "review")
                    .unwrap()
                    .is_some();
                written_when_told.lock().unwrap().push(written);
            }
        });
        let repository = CountedSkillRepository::new(rows, revision.clone());
        let skill = Skill::new(
            SkillId::new("review"),
            Namespace::local(),
            "review",
            "Reviews changes",
            AuditFields::new(1, 1, /*is_deleted*/ false),
        )
        .unwrap();

        repository.create_skill(skill.clone()).unwrap();
        let created = revision.current();
        // The same row again, which the database refuses.
        assert!(repository.create_skill(skill).is_err());

        assert_eq!(revision.current(), created);
        assert_eq!(*written_when_told.lock().unwrap(), vec![true]);
    });
}

/// An import is committed Skill by Skill.
#[test]
fn an_import_advances_the_revision() {
    ora_logging::with_trace_logging(|| {
        let skills = Skills::new();
        let api = skills.api();
        let source = skills.directory.path().join("source");
        for name in ["audit", "review"] {
            let package = source.join(name);
            fs::create_dir_all(&package).unwrap();
            fs::write(
                package.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: Does it\n---\nInstructions"),
            )
            .unwrap();
        }
        let session = api
            .prepare_import(PrepareSkillImportRequest {
                source: SkillImportSource::Folder {
                    path: source.to_string_lossy().into_owned(),
                },
            })
            .unwrap()
            .session;
        assert_eq!(skills.advanced_from(0), 0, "preparing commits nothing");
        api.commit_import(CommitSkillImportRequest {
            session_id: session.session_id.clone(),
            decisions: Vec::new(),
        })
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let status = api
                .get_import(GetSkillImportSessionRequest {
                    session_id: session.session_id.clone(),
                })
                .unwrap()
                .session
                .status;
            if status == SkillImportSessionStatus::Completed {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{status:?}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(skills.advanced_from(0), 2);
    });
}

/// A plugin's Skills change with its package: installed, upgraded or reinstalled, and uninstalled.
#[test]
fn a_skill_plugin_advances_the_revision() {
    ora_logging::with_trace_logging(|| {
        let skills = Skills::new();
        let repository = skills.repository();
        let plugin = PluginId::new("official", "review").unwrap();
        let projection = PluginSkillProjection {
            name: "audit".to_string(),
            description: "Audits".to_string(),
            package_root: skills.directory.path().join("audit"),
        };
        for step in 1..=2 {
            repository
                .replace_plugin_skills(&plugin, "1.0.0", std::slice::from_ref(&projection), step)
                .unwrap();
            assert_eq!(skills.advanced_from(0), step as u64);
        }
        repository
            .replace_plugin_skills(&plugin, "2.0.0", &[], 3)
            .unwrap();
        assert_eq!(skills.advanced_from(0), 3, "the last one was dropped");
        repository
            .replace_plugin_skills(&plugin, "2.0.1", &[projection], 4)
            .unwrap();
        repository.remove_plugin_skills(&plugin, 5).unwrap();
        assert_eq!(skills.advanced_from(0), 5);
    });
}

/// What becomes available or stops being so without a commit is counted when it is noticed, and a
/// commit is not counted a second time by whoever looks next.
#[test]
fn noticing_another_set_of_available_skills_advances_the_revision() {
    ora_logging::with_trace_logging(|| {
        let skills = Skills::new();
        let api = skills.api();
        api.create(create("review")).unwrap();
        let catalog = skills.catalog();
        let look = || {
            let before = skills.revision.current();
            catalog
                .skill_catalog(&WorkspaceId::new("workspace"))
                .unwrap();
            assert_eq!(catalog.revision(), skills.revision.current());
            skills.advanced_from(before)
        };
        assert_eq!([look(), look()], [0, 0], "the commit was counted");

        let manifest = skills.root().join("review").join("SKILL.md");
        let content = fs::read(&manifest).unwrap();
        fs::remove_file(&manifest).unwrap();
        assert_eq!(look(), 1, "it stopped being available");
        fs::write(&manifest, content).unwrap();
        assert_eq!([look(), look()], [1, 0], "it is available again");
        api.create(create("audit")).unwrap();
        assert_eq!([look(), look()], [0, 0], "the commit was counted");
    });
}

/// A look at the Skills from before a change says nothing about those after it.
#[test]
fn a_look_from_before_a_change_is_ignored() {
    let revision = SkillRevision::default();
    let paths = |name: &str| [PathBuf::from(name)].into_iter().collect();
    revision.observed(0, paths("old"));
    revision.committed();
    revision.observed(0, paths("old"));
    revision.observed(1, paths("new"));
    assert_eq!(revision.current(), 1);

    revision.observed(1, paths("other"));
    assert_eq!(revision.current(), 2);
}

/// A change reaches clients as one invalidation per agent.
#[tokio::test]
async fn a_change_invalidates_what_every_agent_offered() {
    let temporary = TempDir::new().unwrap();
    let home = temporary.path().join("home");
    for agent in ["first", "second"] {
        let package = home
            .join("plugins/installed/official")
            .join(agent)
            .join("1.0.0");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("main.js"), "export {};\n").unwrap();
        fs::write(
            package.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"{agent}\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Example\"\n"
            ),
        )
        .unwrap();
    }
    let package = home.join("plugins/installed/official/review/1.0.0");
    fs::create_dir_all(package.join("assets/audit")).unwrap();
    fs::write(
        package.join("orax.toml"),
        "resolver = 1\nidentifier = \"review\"\nnamespace = \"official\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Example\"\n",
    )
    .unwrap();
    fs::write(
        package.join("assets/audit/SKILL.md"),
        "---\nname: audit\ndescription: Audits\n---\nInstructions",
    )
    .unwrap();
    let backend = crate::Backend::open(crate::test_backend::backend_paths(
        &temporary.path().join("app-data"),
        &home,
    ))
    .unwrap();
    let mut events = backend.app_events().subscribe();

    for change in ["a Skill of the user's", "a plugin's Skills"] {
        if change == "a plugin's Skills" {
            backend
                .plugins()
                .uninstall(ora_contracts::UninstallPluginRequest {
                    plugin_id: "official/review".to_string(),
                    data_disposition: ora_contracts::PluginDataDisposition::Delete,
                    hook_execution_acknowledged: false,
                })
                .await
                .unwrap();
        } else {
            backend.skills().create(create("review")).unwrap();
        }

        let mut invalidated = Vec::new();
        while invalidated.len() < 2 {
            let event = tokio::time::timeout(std::time::Duration::from_secs(30), events.recv());
            if let AppEvent::AgentModelsInvalidated { agent_ref } =
                event.await.unwrap().unwrap().unwrap()
            {
                invalidated.push(agent_ref);
            }
        }
        invalidated.sort();
        assert_eq!(
            invalidated,
            ["official/first", "official/second"],
            "{change}"
        );
    }
}

/// A change whose package already reached the disk when its row could not be written is rolled
/// back, and counted once: discovery in between may have seen the swapped-in package, or no
/// package at all, under the revision that would otherwise stay current. Create is not counted,
/// as discovery only reads packages that have a row.
#[test]
fn a_rollback_after_the_package_reached_the_disk_advances_the_revision() {
    ora_logging::with_trace_logging(|| {
        let skills = Skills::new();
        let api = skills.api();
        let created = api.create(create("review")).unwrap().skill;
        let catalog = skills.catalog();
        let manifest = skills.root().join("review").join("SKILL.md");
        let content = fs::read(&manifest).unwrap();
        // Every row write fails from here on, after the package was swapped in or moved aside.
        rusqlite::Connection::open(skills.directory.path().join("ora.sqlite3"))
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER refuse_updates BEFORE UPDATE ON skills BEGIN SELECT RAISE(ABORT, 'refused'); END;
                 CREATE TRIGGER refuse_inserts BEFORE INSERT ON skills BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
        let update = |name: &str, content: Option<&str>| UpdateSkillRequest {
            skill_id: created.id.clone(),
            name: name.to_string(),
            description: "Reviews changes".to_string(),
            content: content.map(str::to_string),
        };
        let failures: [(&str, &dyn Fn() -> bool, u64); 4] = [
            (
                "edit in place",
                &|| {
                    api.update(update("review", Some("Other instructions")))
                        .is_err()
                },
                1,
            ),
            ("rename", &|| api.update(update("audit", None)).is_err(), 1),
            (
                "delete",
                &|| {
                    api.delete(DeleteSkillRequest {
                        skill_id: created.id.clone(),
                    })
                    .is_err()
                },
                1,
            ),
            ("create", &|| api.create(create("audit")).is_err(), 0),
        ];
        for (change, fails, advanced) in failures {
            catalog
                .skill_catalog(&WorkspaceId::new("workspace"))
                .unwrap();
            let discovered = catalog.revision();
            assert!(fails(), "{change}");
            assert_eq!(skills.advanced_from(discovered), advanced, "{change}");
            catalog
                .skill_catalog(&WorkspaceId::new("workspace"))
                .unwrap();
            assert_eq!(
                catalog.revision() == discovered,
                advanced == 0,
                "{change}: the next discovery's revision"
            );
            assert_eq!(
                fs::read(&manifest).unwrap(),
                content,
                "{change}: the package is back"
            );
            assert!(!skills.root().join("audit").exists(), "{change}");
        }
    });
}
