//! Integration coverage for Workspace-scoped Effect convergence.

mod tests {
    use crate::setup::DesktopTestSetup;
    use ora_backend::Backend;
    use ora_contracts::{
        AgentStatus, CommitSkillImportRequest, CreateProjectRequest, DeleteSkillRequest,
        GetAgentRuntimeStatusRequest, GetEffectTargetStatusRequest, GetSkillImportSessionRequest,
        ListSkillsRequest, ListWorkspacesRequest, PrepareSkillImportRequest, SkillImportProgress,
        SkillImportResult, SkillImportResultStatus, SkillImportSessionStatus, SkillImportSource,
    };
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const AGENT_NAMESPACE: &str = "official";
    const AGENT_NAME: &str = "ora-space.opencode";
    // Keep the deadline below the worker's 30-second periodic scan so this test still proves the
    // direct wake path, while allowing slower CI process scheduling and plugin IPC.
    const EFFECT_TIMEOUT: Duration = Duration::from_secs(15);
    const POLL_INTERVAL: Duration = Duration::from_millis(10);

    /// Verifies imported Skills promptly converge into an OpenCode Workspace and disappear after
    /// deletion without waiting for the Effect worker's periodic scan.
    #[test]
    fn imported_skill_converges_into_workspace_and_is_removed_after_deletion()
    -> Result<(), Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let mut backend_paths = setup.backend_paths().clone();
        backend_paths.deno_path = env!("CARGO_BIN_EXE_fake-agent").into();
        let backend = Backend::open(backend_paths)?;
        let agent_ref = format!("{AGENT_NAMESPACE}/{AGENT_NAME}");
        wait_until("fake OpenCode agent did not become ready", || {
            backend
                .agent_runtime()
                .status(GetAgentRuntimeStatusRequest {})
                .is_ok_and(|response| {
                    response.statuses.iter().any(|runtime| {
                        runtime.agent_ref == agent_ref && runtime.status == AgentStatus::Ready
                    })
                })
        })?;

        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.projects().create(CreateProjectRequest {
            name: "Effect E2E".to_string(),
            main_workspace_path: workspace.to_string_lossy().into_owned(),
        })?;

        let import_source = setup.root().join("import").join("review");
        fs::create_dir_all(&import_source)?;
        fs::write(
            import_source.join("SKILL.md"),
            "---\nname: review\ndescription: Reviews changes\n---\n# Review\n",
        )?;
        let prepared = backend.skills().prepare_import(PrepareSkillImportRequest {
            source: SkillImportSource::Folder {
                path: import_source.to_string_lossy().into_owned(),
            },
        })?;
        assert_eq!(prepared.session.candidates.len(), 1);
        let candidate_id = prepared.session.candidates[0].candidate_id.clone();
        let session_id = prepared.session.session_id;
        backend.skills().commit_import(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: Vec::new(),
        })?;
        wait_until("Skill import did not complete", || {
            backend
                .skills()
                .get_import(GetSkillImportSessionRequest {
                    session_id: session_id.clone(),
                })
                .is_ok_and(|response| {
                    response.session.status == SkillImportSessionStatus::Completed
                })
        })?;
        let completed = backend
            .skills()
            .get_import(GetSkillImportSessionRequest { session_id })?
            .session;
        assert_eq!(
            completed.progress,
            SkillImportProgress {
                total: 1,
                processed: 1,
                results: vec![SkillImportResult {
                    candidate_id,
                    name: "review".to_string(),
                    status: SkillImportResultStatus::Imported,
                    error_code: None,
                }],
            }
        );
        let skills = backend.skills().list(ListSkillsRequest {})?.skills;
        assert_eq!(skills.len(), 1);

        let materialized_skill = workspace.join(".opencode").join("skills").join("review");
        wait_until("imported Skill was not promptly materialized", || {
            materialized_skill.join("SKILL.md").is_file()
        })?;
        let workspace_id = backend
            .workspaces()
            .list(ListWorkspacesRequest {})?
            .workspaces
            .into_iter()
            .next()
            .ok_or("fixture workspace missing")?
            .id;
        let status = backend
            .effects()
            .target_status(GetEffectTargetStatusRequest::WorkspaceAgent {
                workspace_id,
                agent_plugin_id: agent_ref,
            })?
            .status
            .ok_or("materialization must have a persisted target status")?;
        let by_id = backend
            .effects()
            .target_status(GetEffectTargetStatusRequest::Target {
                target_id: status.target_id.clone(),
            })?
            .status
            .ok_or("target id must resolve the same status")?;
        assert_eq!(by_id.target_id, status.target_id);
        backend.skills().delete(DeleteSkillRequest {
            skill_id: skills[0].id.clone(),
        })?;
        wait_until("deleted Skill was not promptly removed", || {
            !materialized_skill.exists()
        })?;

        Ok(())
    }

    /// Installs the package metadata that makes the E2E fake process discoverable as OpenCode.
    fn install_fake_opencode_plugin(home_directory: &Path) -> io::Result<()> {
        let package_root = home_directory
            .join("plugins")
            .join("installed")
            .join(AGENT_NAMESPACE)
            .join(AGENT_NAME)
            .join("1.0.0");
        fs::create_dir_all(&package_root)?;
        fs::write(package_root.join("main.js"), "export {};\n")?;
        fs::write(
            package_root.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"{AGENT_NAME}\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"OpenCode E2E agent\"\n"
            ),
        )
    }

    /// Polls an asynchronous external observation until it becomes true or the prompt deadline
    /// proves the Effect worker was not notified.
    fn wait_until(message: &str, mut condition: impl FnMut() -> bool) -> io::Result<()> {
        let deadline = Instant::now() + EFFECT_TIMEOUT;
        while Instant::now() < deadline {
            if condition() {
                return Ok(());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, message))
    }
}
