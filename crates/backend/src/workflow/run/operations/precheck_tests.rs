//! Dependency prechecks on the start, restart, and resume paths.
//!
//! A run freezes its graph and the Skill placements it deploys with when it is created, so the only
//! way to prove that beginning scheduling notices a dependency that disappeared afterwards is to
//! install a real Skill plugin, bind it in a published graph, create the run, uninstall the plugin
//! through the plugin use case, and then start or restart that same run through the public
//! interface.

use super::WorkflowRuns;
use crate::Backend;
use crate::test_backend::backend_paths;
use crate::workflow::run::test_fixture::{SeededWorkflow, seeded_workflow};
use ora_application::{WorkflowRepository, WorkflowRunEngineRepository, WorkflowRunRepository};
use ora_contracts::{
    CancelWorkflowRunRequest, CreateWorkflowRunRequest, ImportPluginRequest, PluginDataDisposition,
    PublicError, RestartWorkflowRunRequest, ResumeWorkflowRunRequest, StartWorkflowRunRequest,
    UninstallPluginRequest, WorkflowMcpNotFoundParams, WorkflowRunLocale, WorkflowRunStatus,
    WorkflowSkillNotFoundParams,
};
use pretty_assertions::assert_eq;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// A published graph whose only agent node binds one Skill under a user-visible title.
const SKILL_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"reviewer","data":{"kind":"agent","title":"Code review","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"official/ora-space.opencode","modelId":"m"},"prompt":"review","skills":[{"skillId":"code_review","enabled":true}]}}}
],"edges":[{"source":"start","target":"reviewer"}]}"#;

/// The same node shape, binding one MCP plugin instead of a Skill.
const MCP_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"reviewer","data":{"kind":"agent","title":"Code review","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"official/ora-space.opencode","modelId":"m"},"prompt":"review","mcps":[{"mcpId":"local/github","enabled":true}]}}}
],"edges":[{"source":"start","target":"reviewer"}]}"#;

/// One backend, one published workflow, and the run use cases every test drives.
struct Fixture {
    temporary: TempDir,
    backend: Backend,
    runs: Arc<WorkflowRuns>,
    seeded: SeededWorkflow,
}

impl Fixture {
    /// Opens a composed backend and seeds a project, workflow, and published snapshot.
    fn open(graph: &str) -> Self {
        let temporary = TempDir::new().expect("run fixture");
        let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
            .expect("backend composition");
        let runs = backend.workflow_runs();
        let seeded = seeded_workflow(&temporary, &runs.pool, graph);
        Self {
            temporary,
            backend,
            runs,
            seeded,
        }
    }

    /// Creates the run the way the run screen does: through the public create use case.
    fn create_run(&self) -> String {
        self.runs
            .create(CreateWorkflowRunRequest {
                workspace_id: self.seeded.workspace_id.clone(),
                workflow_id: self.seeded.workflow_id.clone(),
                locale: WorkflowRunLocale::EnUs,
                snapshot_id: None,
                kickoff_input: None,
                name: None,
                inject_last_failure: None,
            })
            .expect("create run from the published snapshot")
            .run
            .id
            .clone()
    }

    /// Starts a run and leaves it restartable, so later steps do not race the dispatched node.
    async fn start_and_settle(&self, run_id: &str) {
        self.runs
            .start(StartWorkflowRunRequest {
                run_id: run_id.to_string(),
            })
            .expect("start with every dependency installed");
        let cancelled = self
            .runs
            .cancel(CancelWorkflowRunRequest {
                run_id: run_id.to_string(),
            })
            .await
            .expect("cancel the started run");
        assert_eq!(
            cancelled.run.status,
            WorkflowRunStatus::Cancelled,
            "the run must be restartable before the dependency of the next step changes"
        );
    }

    /// Returns the public run and node state before or after a scheduling attempt.
    fn get(&self, run_id: &str) -> ora_contracts::GetWorkflowRunResponse {
        self.runs
            .get(ora_contracts::GetWorkflowRunRequest {
                run_id: run_id.to_string(),
            })
            .unwrap()
    }

    /// Publishes the graph a resume explicitly selects, leaving the existing run on v1.
    fn publish_target(&self, graph: &str) {
        let repository = ora_db::SqliteWorkflowRepository::new(self.runs.pool.clone());
        let workflow = ora_domain::WorkflowId::new(&self.seeded.workflow_id);
        assert!(matches!(
            repository
                .update_draft(&workflow, graph.to_string(), 30)
                .unwrap(),
            ora_application::UpdateDraftResult::Updated(_)
        ));
        assert!(matches!(
            repository
                .publish_snapshot(
                    &workflow,
                    ora_domain::WorkflowSnapshotId::new("snapshot-2"),
                    "v2".into(),
                    31
                )
                .unwrap(),
            ora_application::PublishSnapshotResult::Published(_)
        ));
    }

    /// Installs the Skill plugin the graph binds through the real local-archive import.
    async fn install_skill_plugin(&self, plugin: &str, skill: &str) {
        let archive = skill_plugin_archive(self.temporary.path(), plugin, skill);
        self.backend
            .plugins()
            .import(ImportPluginRequest {
                path: archive.to_string_lossy().into_owned(),
                hook_execution_acknowledged: false,
            })
            .await
            .expect("import the skill plugin");
    }

    /// Removes the installed package and its catalog projection, as the plugin market does.
    async fn uninstall_plugin(&self, plugin_id: &str) {
        self.backend
            .plugins()
            .uninstall(UninstallPluginRequest {
                plugin_id: plugin_id.to_string(),
                data_disposition: PluginDataDisposition::Delete,
                hook_execution_acknowledged: false,
            })
            .await
            .expect("uninstall the plugin");
    }
}

/// Writes one Skill plugin release archive so the import path installs and catalogs its Skill.
fn skill_plugin_archive(directory: &Path, plugin: &str, skill: &str) -> PathBuf {
    let archive = directory.join(format!("{plugin}.orax"));
    let manifest = format!(
        "resolver = 1\nidentifier = \"{plugin}\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Skill fixture\"\n"
    );
    let mut writer = ZipWriter::new(File::create(&archive).expect("create skill archive"));
    let options = SimpleFileOptions::default();
    writer
        .start_file("orax.toml", options)
        .expect("write manifest");
    writer
        .write_all(manifest.as_bytes())
        .expect("write manifest");
    writer
        .start_file(format!("assets/{skill}/SKILL.md"), options)
        .expect("write skill manifest");
    writer
        .write_all(
            format!("---\nname: {skill}\ndescription: fixture skill\n---\n\nReview the change.\n")
                .as_bytes(),
        )
        .expect("write skill manifest");
    writer.finish().expect("finish skill archive");
    archive
}

/// Installs a valid MCP with an unset required setting through the live plugin host.
/// Settings and command readiness belong to session setup, while scheduling requires the package.
async fn install_mcp_package(fixture: &Fixture) {
    let archive = fixture.temporary.path().join("github.orax");
    let mut writer = ZipWriter::new(File::create(&archive).unwrap());
    let options = SimpleFileOptions::default();
    writer.start_file("orax.toml", options).unwrap();
    writer
        .write_all(
            br#"resolver = 1
identifier = "github"
kind = "mcp"
version = "1.0.0"
description = "MCP fixture"
"#,
        )
        .unwrap();
    writer.start_file("assets/config.json", options).unwrap();
    writer.write_all(br#"{"schemaVersion":1,"settings":{"apiKey":{"type":"string","title":"API key","description":"Required credential","required":true}},"transport":{"type":"stdio","command":"assets/server","env":{"API_KEY":{"setting":"apiKey"}}}}"#).unwrap();
    writer
        .start_file("assets/server", options.unix_permissions(0o755))
        .unwrap();
    writer
        .write_all(b"#!/nonexistent-ora-test-interpreter\n")
        .unwrap();
    drop(writer.finish().unwrap());
    let installed = fixture
        .backend
        .plugins()
        .import(ImportPluginRequest {
            path: archive.to_string_lossy().into_owned(),
            hook_execution_acknowledged: false,
        })
        .await
        .expect("import the MCP package");
    assert_eq!(installed.plugin_id, "local/github");
}

/// Uninstalling the Skill plugin a frozen run declares blocks both start and restart, and
/// re-installing it makes the same frozen run startable again.
#[tokio::test]
async fn a_removed_skill_plugin_blocks_start_and_restart() {
    let fixture = Fixture::open(SKILL_GRAPH);
    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    let run_id = fixture.create_run();
    fixture.start_and_settle(&run_id).await;

    fixture.uninstall_plugin("local/review-pack").await;

    let restart_error = fixture
        .runs
        .restart(RestartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect_err("a restart must not bypass the dependency precheck");
    assert_eq!(
        restart_error.public_error(),
        &PublicError::WorkflowSkillNotFound(Box::new(WorkflowSkillNotFoundParams {
            node_id: "reviewer".to_string(),
            node_title: "Code review".to_string(),
            skill_id: "code_review".to_string(),
        }))
    );

    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    fixture
        .runs
        .restart(RestartWorkflowRunRequest { run_id })
        .expect("restart once the Skill plugin is installed again");
}

/// A run that never started is rejected by the same precheck, so `start` cannot be used to run a
/// graph whose Skill plugin is gone.
#[tokio::test]
async fn a_removed_skill_plugin_blocks_a_pending_start() {
    let fixture = Fixture::open(SKILL_GRAPH);
    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    let run_id = fixture.create_run();
    fixture.uninstall_plugin("local/review-pack").await;

    let error = fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect_err("a start must not bypass the dependency precheck");
    assert_eq!(
        error.public_error(),
        &PublicError::WorkflowSkillNotFound(Box::new(WorkflowSkillNotFoundParams {
            node_id: "reviewer".to_string(),
            node_title: "Code review".to_string(),
            skill_id: "code_review".to_string(),
        }))
    );

    // The rejected start left the run untouched, so it is still the same pending run.
    let run = fixture
        .runs
        .get(ora_contracts::GetWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect("load the rejected run")
        .run;
    assert_eq!(run.status, WorkflowRunStatus::Pending);

    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    fixture
        .runs
        .start(StartWorkflowRunRequest { run_id })
        .expect("start once the Skill plugin is installed again");
}

/// Resuming a stopped run re-executes nodes, so it validates the same dependencies a restart does
/// and cannot be used to route around a removed Skill plugin.
#[tokio::test]
async fn a_removed_skill_plugin_blocks_a_resume() {
    let fixture = Fixture::open(SKILL_GRAPH);
    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    let run_id = fixture.create_run();
    fixture.start_and_settle(&run_id).await;
    fixture.uninstall_plugin("local/review-pack").await;

    let error = fixture
        .runs
        .resume_from_failure(ResumeWorkflowRunRequest {
            run_id: run_id.clone(),
            rollback: None,
            snapshot_id: None,
        })
        .expect_err("a resume must not bypass the dependency precheck");
    assert_eq!(
        error.public_error(),
        &PublicError::WorkflowSkillNotFound(Box::new(WorkflowSkillNotFoundParams {
            node_id: "reviewer".to_string(),
            node_title: "Code review".to_string(),
            skill_id: "code_review".to_string(),
        }))
    );

    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    fixture
        .runs
        .resume_from_failure(ResumeWorkflowRunRequest {
            run_id,
            rollback: None,
            snapshot_id: None,
        })
        .expect("resume once the Skill plugin is installed again");
}

/// An idempotent start of an already running run answers with the run instead of turning a
/// dependency that disappeared mid-run into an error.
#[tokio::test]
async fn an_idempotent_start_is_not_turned_into_a_dependency_error() {
    let fixture = Fixture::open(SKILL_GRAPH);
    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    let run_id = fixture.create_run();
    fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect("start with every dependency installed");

    fixture.uninstall_plugin("local/review-pack").await;
    let answered = fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect("an already started run answers with its current state");
    assert_eq!(answered.run.status, WorkflowRunStatus::Running);
    let error = fixture
        .runs
        .restart(RestartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect_err("a running run cannot restart");
    assert_eq!(error.public_error().code(), "workflow_run_not_restartable");

    fixture
        .runs
        .cancel(CancelWorkflowRunRequest { run_id })
        .await
        .expect("cancel the started run so the fixture cannot outlive its runtime");
}

/// The MCP plugin an agent node declares is checked before scheduling, and installing that plugin
/// makes the same pending run startable.
#[tokio::test]
async fn a_missing_mcp_plugin_blocks_start() {
    let fixture = Fixture::open(MCP_GRAPH);
    install_mcp_package(&fixture).await;
    let run_id = fixture.create_run();
    fixture.uninstall_plugin("local/github").await;

    let error = fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect_err("a start must not schedule a node whose MCP plugin is gone");
    assert_eq!(
        error.public_error(),
        &PublicError::WorkflowMcpNotFound(Box::new(WorkflowMcpNotFoundParams {
            node_id: "reviewer".to_string(),
            node_title: "Code review".to_string(),
            mcp_id: "local/github".to_string(),
        }))
    );

    assert_eq!(fixture.get(&run_id).run.status, WorkflowRunStatus::Pending);
    install_mcp_package(&fixture).await;
    fixture
        .runs
        .start(StartWorkflowRunRequest { run_id })
        .expect("start once the MCP plugin is installed");
}

/// Uninstall after cancellation must preserve the run and prevent both routes back to dispatch.
#[tokio::test]
async fn removed_mcp_blocks_restart_and_same_snapshot_resume() {
    let fixture = Fixture::open(MCP_GRAPH);
    install_mcp_package(&fixture).await;
    let run_id = fixture.create_run();
    fixture.start_and_settle(&run_id).await;
    fixture.uninstall_plugin("local/github").await;
    let before = fixture.get(&run_id).run;

    let restart = fixture
        .runs
        .restart(RestartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .unwrap_err();
    assert_eq!(restart.public_error().code(), "workflow_mcp_not_found");
    assert_eq!(fixture.get(&run_id).run, before);
    let resume = fixture
        .runs
        .resume_from_failure(ResumeWorkflowRunRequest {
            run_id: run_id.clone(),
            rollback: None,
            snapshot_id: None,
        })
        .unwrap_err();
    assert_eq!(resume.public_error(), restart.public_error());
    assert_eq!(fixture.get(&run_id).run, before);
}

/// Disabling a binding removes that MCP from the set of scheduling prerequisites.
#[tokio::test]
async fn a_disabled_missing_mcp_allows_start() {
    let graph = MCP_GRAPH.replace(
        r#""mcpId":"local/github","enabled":true"#,
        r#""mcpId":"local/github","enabled":false"#,
    );
    let fixture = Fixture::open(&graph);
    let run_id = fixture.create_run();
    fixture.start_and_settle(&run_id).await;
}

/// A package with the selected id must actually provide a usable MCP.
#[tokio::test]
async fn a_skill_plugin_with_the_selected_mcp_id_blocks_start() {
    let fixture = Fixture::open(MCP_GRAPH);
    fixture.install_skill_plugin("github", "code_review").await;
    let run_id = fixture.create_run();
    let error = fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .unwrap_err();
    assert_eq!(error.public_error().code(), "workflow_mcp_not_found");
    assert_eq!(fixture.get(&run_id).run.status, WorkflowRunStatus::Pending);
}

/// Both kinds of missing target dependency must be refused before a real checkpoint restore.
#[tokio::test]
async fn target_dependencies_are_checked_before_checkpoint_rollback() {
    for (target, expected) in [
        (MCP_GRAPH, "workflow_mcp_not_found"),
        (SKILL_GRAPH, "workflow_skill_not_found"),
    ] {
        let original = MCP_GRAPH.replace(
            r#""mcpId":"local/github","enabled":true"#,
            r#""mcpId":"local/github","enabled":false"#,
        );
        let fixture = Fixture::open(&original);
        let workspace = crate::workflow::run::test_fixture::init_git_workspace(&fixture.temporary);
        let run_id = fixture.create_run();
        fixture.start_and_settle(&run_id).await;
        let before = fixture.get(&run_id);
        let node = before
            .nodes
            .iter()
            .find(|node| node.node_id == "reviewer")
            .unwrap();
        super::super::checkpoint::record_pre_node_checkpoint(
            &ora_db::SqliteWorkflowRunEngineRepository::new(fixture.runs.pool.clone()),
            &workspace,
            &ora_domain::WorkflowRunId::new(&run_id),
            "reviewer",
            &ora_domain::WorkflowNodeRunId::new(&node.id),
            &fixture.seeded.snapshot_id,
            95,
        )
        .unwrap();
        let file = workspace.join("README.md");
        std::fs::write(&file, "unsaved work").unwrap();
        fixture.publish_target(target);
        let request = ResumeWorkflowRunRequest {
            run_id: run_id.clone(),
            rollback: Some(ora_contracts::ResumeRollbackMode::Checkpoint),
            snapshot_id: Some("snapshot-2".to_owned()),
        };
        let error = fixture
            .runs
            .resume_from_failure(request.clone())
            .unwrap_err();
        assert_eq!(error.public_error().code(), expected);
        assert_eq!(fixture.get(&run_id).run, before.run);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "unsaved work");

        // A successful retry proves the selected checkpoint really could overwrite the file.
        if target == MCP_GRAPH {
            install_mcp_package(&fixture).await;
        } else {
            fixture
                .install_skill_plugin("review-pack", "code_review")
                .await;
        }
        let resumed = fixture.runs.resume_from_failure(request).unwrap();
        assert_eq!(resumed.run.snapshot_id, "snapshot-2");
        assert_eq!(
            std::fs::read_to_string(file)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            ["seed"]
        );
    }
}

/// Switching versions freezes catalog-resolved skill bindings for the target snapshot without
/// requiring any Effect declaration or workspace package paths.
#[tokio::test]
async fn switched_receipt_freezes_catalog_skill_bindings_without_workspace_paths() {
    let original = SKILL_GRAPH.replace(
        r#""skillId":"code_review","enabled":true"#,
        r#""skillId":"code_review","enabled":false"#,
    );
    let fixture = Fixture::open(&original);
    let run_id = fixture.create_run();
    fixture.start_and_settle(&run_id).await;
    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    fixture.publish_target(SKILL_GRAPH);
    let resumed = fixture
        .runs
        .resume_from_failure(ResumeWorkflowRunRequest {
            run_id: run_id.clone(),
            rollback: None,
            snapshot_id: Some("snapshot-2".to_owned()),
        })
        .unwrap();
    assert_eq!(resumed.run.snapshot_id, "snapshot-2");
    let row = ora_db::SqliteWorkflowRunRepository::new(fixture.runs.pool.clone())
        .find_run(&ora_domain::WorkflowRunId::new(run_id))
        .unwrap()
        .unwrap();
    let payload: ora_application::WorkflowRunPayload =
        serde_json::from_str(row.payload.as_deref().unwrap()).unwrap();
    let bindings = payload.skill_materialization.bindings_for_node("reviewer");
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].skill_id, "code_review");
    assert_eq!(
        bindings[0].catalog_skill_id.as_deref(),
        Some("plugin:local/review-pack:code_review")
    );
    assert_eq!(bindings[0].invocation_name, "code-review");
}

/// Uninstalling the Skill plugin a run froze must block start, restart, and same-snapshot resume
/// even when a same-named local Skill is created afterwards.
#[tokio::test]
async fn removed_plugin_skill_does_not_fall_back_to_same_named_local_skill() {
    let fixture = Fixture::open(SKILL_GRAPH);
    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    let run_id = fixture.create_run();
    fixture.start_and_settle(&run_id).await;
    fixture.uninstall_plugin("local/review-pack").await;

    fixture
        .backend
        .skills()
        .create(ora_contracts::CreateSkillRequest {
            name: "code_review".to_string(),
            description: "Local impostor".to_string(),
            content: Some("Local instructions".to_string()),
        })
        .expect("create same-named local skill");

    let restart_error = fixture
        .runs
        .restart(RestartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect_err(
            "restart must reject missing frozen plugin skill despite same-named local skill",
        );
    assert_eq!(
        restart_error.public_error(),
        &PublicError::WorkflowSkillNotFound(Box::new(WorkflowSkillNotFoundParams {
            node_id: "reviewer".to_string(),
            node_title: "Code review".to_string(),
            skill_id: "code_review".to_string(),
        }))
    );

    let resume_error = fixture
        .runs
        .resume_from_failure(ResumeWorkflowRunRequest {
            run_id: run_id.clone(),
            rollback: None,
            snapshot_id: None,
        })
        .expect_err(
            "resume must reject missing frozen plugin skill despite same-named local skill",
        );
    assert_eq!(resume_error.public_error(), restart_error.public_error());

    // Re-installing the frozen plugin restores the exact catalog ID and allows restart again.
    fixture
        .install_skill_plugin("review-pack", "code_review")
        .await;
    fixture
        .runs
        .restart(RestartWorkflowRunRequest { run_id })
        .expect("restart succeeds once original plugin skill returns");
}

/// Scheduling accepts an installed, unconfigured MCP and keeps a running start idempotent
/// when that MCP is subsequently removed.
#[tokio::test]
async fn start_with_an_unconfigured_mcp_stays_idempotent_after_uninstall() {
    let fixture = Fixture::open(MCP_GRAPH);
    install_mcp_package(&fixture).await;
    let run_id = fixture.create_run();
    let request = StartWorkflowRunRequest {
        run_id: run_id.clone(),
    };
    assert_eq!(
        fixture.runs.start(request.clone()).unwrap().run.status,
        WorkflowRunStatus::Running
    );
    fixture.uninstall_plugin("local/github").await;
    assert_eq!(
        fixture.runs.start(request).unwrap().run.status,
        WorkflowRunStatus::Running
    );
    let restart = fixture
        .runs
        .restart(RestartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .unwrap_err();
    assert_eq!(
        restart.public_error().code(),
        "workflow_run_not_restartable"
    );
    fixture
        .runs
        .cancel(CancelWorkflowRunRequest { run_id })
        .await
        .unwrap();
}

/// A package whose manifest no longer exposes a valid MCP cannot satisfy a frozen binding.
#[tokio::test]
async fn invalid_mcp_package_blocks_pending_start() {
    let fixture = Fixture::open(MCP_GRAPH);
    install_mcp_package(&fixture).await;
    let run_id = fixture.create_run();
    let config = fixture
        .temporary
        .path()
        .join("plugins")
        .join("installed")
        .join("local")
        .join("github")
        .join("1.0.0")
        .join("assets")
        .join("config.json");
    std::fs::write(config, "invalid configuration").unwrap();
    let error = fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .unwrap_err();
    assert_eq!(error.public_error().code(), "workflow_mcp_not_found");
    assert_eq!(fixture.get(&run_id).run.status, WorkflowRunStatus::Pending);
}

/// A blank title cannot identify the node, so the diagnostic carries the node id instead and
/// nothing but the three identity fields reaches the client.
#[tokio::test]
async fn a_blank_node_title_falls_back_to_the_node_id() {
    let graph = MCP_GRAPH.replace(r#""title":"Code review""#, r#""title":"   ""#);
    let fixture = Fixture::open(&graph);
    install_mcp_package(&fixture).await;
    let run_id = fixture.create_run();
    fixture.uninstall_plugin("local/github").await;

    let error = fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .unwrap_err();
    assert_eq!(
        error.public_error(),
        &PublicError::WorkflowMcpNotFound(Box::new(WorkflowMcpNotFoundParams {
            node_id: "reviewer".to_string(),
            node_title: "reviewer".to_string(),
            mcp_id: "local/github".to_string(),
        }))
    );
    // Configuration values, transport arguments, and environment are not part of the diagnostic.
    assert_eq!(
        serde_json::to_value(error.public_error()).unwrap(),
        serde_json::json!({
            "code": "workflow_mcp_not_found",
            "params": {
                "nodeId": "reviewer",
                "nodeTitle": "reviewer",
                "mcpId": "local/github",
            },
        })
    );
    assert_eq!(fixture.get(&run_id).run.status, WorkflowRunStatus::Pending);
}

/// A run that cannot be resumed keeps its original rejection even when a dependency is also
/// missing, so the user is not sent to reinstall a plugin that would not make it resumable.
#[tokio::test]
async fn a_run_that_cannot_resume_reports_that_before_a_missing_dependency() {
    let fixture = Fixture::open(MCP_GRAPH);
    install_mcp_package(&fixture).await;
    let run_id = fixture.create_run();
    fixture.uninstall_plugin("local/github").await;

    let error = fixture
        .runs
        .resume_from_failure(ResumeWorkflowRunRequest {
            run_id: run_id.clone(),
            rollback: None,
            snapshot_id: None,
        })
        .unwrap_err();
    assert_eq!(error.public_error().code(), "workflow_run_not_resumable");
    assert_eq!(fixture.get(&run_id).run.status, WorkflowRunStatus::Pending);
}

/// Only the old graph is missing the binding, so selecting the fixed target snapshot must switch
/// instead of failing on the snapshot the run is leaving behind.
#[tokio::test]
async fn a_target_snapshot_without_the_missing_binding_still_switches() {
    let fixture = Fixture::open(MCP_GRAPH);
    install_mcp_package(&fixture).await;
    let run_id = fixture.create_run();
    fixture.start_and_settle(&run_id).await;
    fixture.uninstall_plugin("local/github").await;

    // The target graph drops the binding entirely, which repairs the dependency of the switch.
    fixture.publish_target(
        &MCP_GRAPH.replace(r#","mcps":[{"mcpId":"local/github","enabled":true}]"#, ""),
    );
    let resumed = fixture
        .runs
        .resume_from_failure(ResumeWorkflowRunRequest {
            run_id: run_id.clone(),
            rollback: None,
            snapshot_id: Some("snapshot-2".to_owned()),
        })
        .expect("a target snapshot without the removed binding must be usable");
    assert_eq!(resumed.run.snapshot_id, "snapshot-2");
    assert_eq!(resumed.run.status, WorkflowRunStatus::Running);
    fixture
        .runs
        .cancel(CancelWorkflowRunRequest { run_id })
        .await
        .expect("cancel the switched run so the fixture cannot outlive its runtime");
}

/// Checks persisted node runs at the backend boundary. The desktop E2E observes sessions and ACP
/// with an installed agent.
#[tokio::test]
async fn a_rejected_start_creates_no_node_run() {
    let fixture = Fixture::open(MCP_GRAPH);
    install_mcp_package(&fixture).await;
    let run_id = fixture.create_run();
    fixture.uninstall_plugin("local/github").await;

    fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .unwrap_err();

    let detail = fixture.get(&run_id);
    assert_eq!(detail.run.status, WorkflowRunStatus::Pending);
    assert!(
        detail.nodes.is_empty(),
        "a rejected start must not create node-run rows"
    );
    assert!(
        ora_db::SqliteWorkflowRunEngineRepository::new(fixture.runs.pool.clone())
            .list_node_runs(&ora_domain::WorkflowRunId::new(&run_id))
            .unwrap()
            .is_empty(),
        "a rejected start must not create node-run rows"
    );
}
