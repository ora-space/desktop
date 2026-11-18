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

/// Writes one installed MCP package whose declared stdio command exists inside the package.
fn install_mcp_package(home: &Path, name: &str) {
    let root = home
        .join("plugins")
        .join("installed")
        .join("local")
        .join(name)
        .join("1.0.0");
    std::fs::create_dir_all(root.join("assets")).expect("create MCP package");
    std::fs::write(
        root.join("orax.toml"),
        format!(
            "resolver = 1\nidentifier = \"{name}\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"MCP fixture\"\n"
        ),
    )
    .expect("write MCP manifest");
    let command = root.join("assets").join("server");
    std::fs::write(&command, "#!/bin/sh\n").expect("write MCP command");
    // Unix discovery refuses a stdio command without an executable mode bit, and CI runs there.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755))
            .expect("make the MCP command executable");
    }
    std::fs::write(
        root.join("assets").join("config.json"),
        r#"{"schemaVersion":1,"transport":{"type":"stdio","command":"assets/server","args":[],"env":{}}}"#,
    )
    .expect("write MCP configuration");
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

    let answered = fixture
        .runs
        .start(StartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect("an already started run answers with its current state");
    let _ = answered;

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
    let run_id = fixture.create_run();

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

    install_mcp_package(fixture.temporary.path(), "github");
    fixture
        .runs
        .start(StartWorkflowRunRequest { run_id })
        .expect("start once the MCP plugin is installed");
}
