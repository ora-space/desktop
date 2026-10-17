//! Integration coverage for Workspace-scoped Effect convergence.

mod tests {
    use crate::setup::DesktopTestSetup;
    use ora_backend::Backend;
    use ora_contracts::{
        AgentStatus, CommitSkillImportRequest, CreateProjectRequest, DeleteSkillRequest,
        EffectConditionImpactDto, EffectTargetPhaseDto, EffectTargetStatusDto,
        GetAgentRuntimeStatusRequest, GetEffectTargetStatusRequest, GetSkillImportSessionRequest,
        ListSkillsRequest, ListWorkspacesRequest, PluginDataDisposition, PrepareSkillImportRequest,
        RecoverEffectTargetRequest, ScanPluginsRequest, SkillImportProgress, SkillImportResult,
        SkillImportResultStatus, SkillImportSessionStatus, SkillImportSource,
        UninstallPluginRequest,
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
        install_fake_agent_package(home_directory, AGENT_NAMESPACE, AGENT_NAME, "1.0.0")
    }

    /// Installs the same fake agent package under one arbitrary source namespace so a second
    /// plugin id declares the identical Effect Resource, exactly like a Marketplace build of one
    /// locally installed agent (issue #6).
    fn install_fake_agent_package(
        home_directory: &Path,
        namespace: &str,
        name: &str,
        version: &str,
    ) -> io::Result<()> {
        let package_root = home_directory
            .join("plugins")
            .join("installed")
            .join(namespace)
            .join(name)
            .join(version);
        fs::create_dir_all(&package_root)?;
        fs::write(package_root.join("main.js"), "export {};\n")?;
        fs::write(
            package_root.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"{name}\"\nkind = \"agent\"\nversion = \"{version}\"\ndescription = \"OpenCode E2E agent\"\n"
            ),
        )
    }

    /// Polls an asynchronous external observation until the prompt deadline proves the
    /// Effect worker was not notified.
    fn wait_until(message: &str, condition: impl FnMut() -> bool) -> io::Result<()> {
        wait_until_within(EFFECT_TIMEOUT, message, condition)
    }

    /// Polls with an explicit budget for waits that open a second backend under the
    /// full suite's parallel load, where plugin startup can outlast the wake-path budget.
    fn wait_until_within(
        budget: Duration,
        message: &str,
        mut condition: impl FnMut() -> bool,
    ) -> io::Result<()> {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            if condition() {
                return Ok(());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, message))
    }

    /// Verifies the issue #6 handover end to end: uninstalling a pure-consumer agent plugin
    /// (the Skill is a local import, so the plugin contributes no catalog Source) and installing
    /// the same-capability agent from another source must hand over the shared Skill directory
    /// Resource — the retired Target converges away and the replacement Target becomes ready,
    /// instead of both Targets retrying forever on an immutable projection conflict.
    #[tokio::test]
    async fn uninstall_and_reinstall_from_another_source_hands_over_the_shared_skill_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let mut backend_paths = setup.backend_paths().clone();
        backend_paths.deno_path = env!("CARGO_BIN_EXE_fake-agent").into();
        let backend = Backend::open(backend_paths)?;
        let local_agent = format!("{AGENT_NAMESPACE}/{AGENT_NAME}");
        wait_until("fake OpenCode agent did not become ready", || {
            backend
                .agent_runtime()
                .status(GetAgentRuntimeStatusRequest {})
                .is_ok_and(|response| {
                    response.statuses.iter().any(|runtime| {
                        runtime.agent_ref == local_agent && runtime.status == AgentStatus::Ready
                    })
                })
        })?;

        // One Workspace with one locally imported Skill: the Skill stays Desired through any
        // agent plugin change because it belongs to the workspace catalog, not to a plugin.
        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.projects().create(CreateProjectRequest {
            name: "Effect handover E2E".to_string(),
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
        let session_id = prepared.session.session_id;
        backend.skills().commit_import(CommitSkillImportRequest {
            session_id: session_id.clone(),
            decisions: Vec::new(),
        })?;
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
        wait_until("local agent Target never became ready", || {
            backend
                .effects()
                .target_status(GetEffectTargetStatusRequest::WorkspaceAgent {
                    workspace_id: workspace_id.clone(),
                    agent_plugin_id: local_agent.clone(),
                })
                .is_ok_and(|response| {
                    response.status.is_some_and(|status| {
                        (status.phase == ora_contracts::EffectTargetPhaseDto::Current
                            || status.phase
                                == ora_contracts::EffectTargetPhaseDto::CurrentWithIssues)
                            && status.ready_generation >= status.desired_generation
                    })
                })
        })?;

        // Uninstall the local build, then install the same capability from another source
        // before waiting on convergence — the exact interleaving reported in issue #6.
        backend
            .plugins()
            .uninstall(UninstallPluginRequest {
                plugin_id: local_agent.clone(),
                data_disposition: PluginDataDisposition::Delete,
                hook_execution_acknowledged: false,
            })
            .await?;
        let market_namespace = "ora-space-marketplace.6905518b";
        install_fake_agent_package(
            &setup.backend_paths().home_directory,
            market_namespace,
            AGENT_NAME,
            "1.0.0",
        )?;
        backend.plugins().scan(ScanPluginsRequest {}).await?;
        let market_agent = format!("{market_namespace}/{AGENT_NAME}");
        wait_until("marketplace agent did not become ready", || {
            backend
                .agent_runtime()
                .status(GetAgentRuntimeStatusRequest {})
                .is_ok_and(|response| {
                    response.statuses.iter().any(|runtime| {
                        runtime.agent_ref == market_agent && runtime.status == AgentStatus::Ready
                    })
                })
        })?;

        // The marketplace Target must converge onto the same Skill directory Resource and the
        // workspace must become sendable again (ready evidence at the desired generation).
        wait_until(
            "marketplace Target never converged on the shared Skill directory (issue #6)",
            || {
                backend
                    .effects()
                    .target_status(GetEffectTargetStatusRequest::WorkspaceAgent {
                        workspace_id: workspace_id.clone(),
                        agent_plugin_id: market_agent.clone(),
                    })
                    .is_ok_and(|response| {
                        response.status.is_some_and(|status| {
                            (status.phase == ora_contracts::EffectTargetPhaseDto::Current
                                || status.phase
                                    == ora_contracts::EffectTargetPhaseDto::CurrentWithIssues)
                                && status.ready_generation >= status.desired_generation
                        })
                    })
            },
        )?;
        // The WorkspaceAgent status API only reports active Target rows, so this proves the
        // retired agent keeps no active row; deletion of the retiring row itself is asserted by
        // the repository-level handover regression test.
        let retired =
            backend
                .effects()
                .target_status(GetEffectTargetStatusRequest::WorkspaceAgent {
                    workspace_id: workspace_id.clone(),
                    agent_plugin_id: local_agent.clone(),
                })?;
        assert_eq!(
            retired.status, None,
            "no active Target row remains for the retired agent"
        );
        // The handover must not lose the materialized Skill the workspace still desires.
        assert!(
            materialized_skill.join("SKILL.md").is_file(),
            "the shared Skill directory keeps its materialized content"
        );

        Ok(())
    }
    /// Rewinds the workspace's converged Effect journal to the durable leftover a
    /// pre-0.4.0 finalize failure produced, including the still-owed request whose
    /// claim lease has expired.
    fn rewind_journal_to_applied_leftover(database: &Path) -> io::Result<()> {
        let connection = rusqlite::Connection::open(database)
            .map_err(|error| io::Error::other(error.to_string()))?;
        connection
            .execute_batch(
                r#"
                UPDATE effect_operations SET phase = 'applied', finalized_at = NULL;
                UPDATE effect_reconcile_attempts SET phase = 'activated' WHERE phase = 'finalized';
                INSERT INTO effect_reconcile_requests (
                    target_id, requested_generation, state, wake_reasons_json, retry_count,
                    claim_token, claim_worker, lease_until, requested_at, updated_at
                )
                SELECT attempt.target_id, scope.generation, 'claimed', '[]', 3,
                       1, 'worker-e2e', 1, 1, 1
                FROM effect_reconcile_attempts attempt
                JOIN effect_targets target ON target.id = attempt.target_id
                JOIN effect_scopes scope ON scope.id = target.scope_id
                WHERE attempt.phase = 'activated';
                "#,
            )
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(())
    }

    /// Verifies issue #177 end to end: a legacy Applied operation log left by an earlier
    /// version is quarantined at restart with no way out, and the controlled recovery
    /// command proves the journal, closes it, and restores the send gate — without
    /// disturbing the materialized Skill or ordinary convergence.
    #[tokio::test]
    async fn quarantined_target_recovery_restores_the_send_gate()
    -> Result<(), Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let mut backend_paths = setup.backend_paths().clone();
        backend_paths.deno_path = env!("CARGO_BIN_EXE_fake-agent").into();
        let backend = Backend::open(backend_paths.clone())?;
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

        // One Workspace whose imported Skill converges and materializes promptly.
        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.projects().create(CreateProjectRequest {
            name: "Effect recovery E2E".to_string(),
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
        backend.skills().commit_import(CommitSkillImportRequest {
            session_id: prepared.session.session_id,
            decisions: Vec::new(),
        })?;
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

        // The send gate the frontend derives from this status must be ready first.
        wait_until("converged Target did not become ready", || {
            backend
                .effects()
                .target_status(GetEffectTargetStatusRequest::WorkspaceAgent {
                    workspace_id: workspace_id.clone(),
                    agent_plugin_id: agent_ref.clone(),
                })
                .is_ok_and(|response| send_gate_ready(&response.status))
        })?;
        let target_id = backend
            .effects()
            .target_status(GetEffectTargetStatusRequest::WorkspaceAgent {
                workspace_id,
                agent_plugin_id: agent_ref.clone(),
            })?
            .status
            .ok_or("converged Target must have a status")?
            .target_id;

        // A pre-0.4.0 finalize failure left an Applied operation log behind; the next
        // process start quarantines it (the exact reporting user's upgrade path).
        rewind_journal_to_applied_leftover(&backend_paths.app_data_directory.join("ora.sqlite3"))?;
        let restarted = Backend::open(backend_paths)?;
        wait_until("restarted agent did not become ready", || {
            restarted
                .agent_runtime()
                .status(GetAgentRuntimeStatusRequest {})
                .is_ok_and(|response| {
                    response.statuses.iter().any(|runtime| {
                        runtime.agent_ref == agent_ref && runtime.status == AgentStatus::Ready
                    })
                })
        })?;
        wait_until("restart did not quarantine the leftover journal", || {
            restarted
                .effects()
                .target_status(GetEffectTargetStatusRequest::Target {
                    target_id: target_id.clone(),
                })
                .is_ok_and(|response| {
                    response.status.as_ref().is_some_and(|status| {
                        status.phase == EffectTargetPhaseDto::RecoveryRequired
                    })
                })
        })?;

        // The controlled recovery exit proves the journal and restores scheduling.
        let recovery = restarted
            .effects()
            .recover_target(RecoverEffectTargetRequest {
                target_id: target_id.clone(),
            })?;
        assert_eq!(
            (recovery.recovered, recovery.finalized_operations),
            (true, 1),
            "the proven Applied journal finalizes instead of staying quarantined"
        );

        // The send gate lifts: current phase, readiness at the Desired generation, and no
        // blocking manual Condition for the frontend to poll on. Two backends now share
        // this database, so this wait uses a budget that tolerates full-suite parallel load.
        wait_until_within(
            Duration::from_secs(45),
            "recovered Target did not converge back to ready",
            || {
                restarted
                    .effects()
                    .target_status(GetEffectTargetStatusRequest::Target {
                        target_id: target_id.clone(),
                    })
                    .is_ok_and(|response| send_gate_ready(&response.status))
            },
        )?;
        assert!(
            materialized_skill.join("SKILL.md").is_file(),
            "recovery must not disturb the materialized Skill"
        );
        let manifest = fs::read_to_string(materialized_skill.join("SKILL.md"))?;
        assert!(manifest.contains("# Review"));

        Ok(())
    }

    /// Mirrors the frontend send gate: ready only at a Current phase whose readiness
    /// watermark covers the Desired generation without any blocking Condition.
    fn send_gate_ready(status: &Option<EffectTargetStatusDto>) -> bool {
        let Some(status) = status else {
            return false;
        };
        (status.phase == EffectTargetPhaseDto::Current
            || status.phase == EffectTargetPhaseDto::CurrentWithIssues)
            && status.ready_generation >= status.desired_generation
            && status
                .conditions
                .iter()
                .all(|condition| condition.impact != EffectConditionImpactDto::Blocking)
    }
}
