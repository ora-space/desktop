//! Integration coverage for lazy session creation and plugin-owned model discovery.

mod tests {
    use crate::setup::DesktopTestSetup;
    use agent_client_protocol_schema::v1::{
        ContentBlock, SessionConfigKind, SessionConfigOption, TextContent,
    };
    use ora_backend::{Backend, BackendError, SessionEventStream};
    use ora_contracts::{
        AgentStatus, CreateProjectRequest, GetAgentRuntimeStatusRequest, ListAgentModelsRequest,
        ListSessionsRequest, ListWorkspacesRequest, LoadSessionEvent, LoadSessionRequest,
        PromptSessionEvent, PromptSessionRequest, StartSessionRequest, WorkspaceKind,
    };
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::future::Future;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    const AGENT_REF: &str = "ora-space.opencode";
    const READY_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(10);
    /// Journal the fake agent appends one line to per `agent/list_models` call.
    const DISCOVERY_JOURNAL: &str = "list_models_calls.txt";
    /// Journal the fake agent appends one line to per session-lifecycle ACP call it serves.
    const ACP_JOURNAL: &str = "acp_calls.txt";
    /// Marker that makes the fake agent refuse `session/load`, as one that lost the session would.
    const LOAD_REFUSAL_MARKER: &str = "refuse_session_load";

    /// Bringing an agent connection up must not perform model discovery.
    ///
    /// Discovery may start a one-shot agent process, so leaving it in the connection startup path
    /// made every agent's availability wait on it and turned any discovery failure into a total
    /// connection failure rather than an agent that momentarily lists no models.
    #[test]
    fn agent_startup_performs_no_model_discovery() -> Result<(), Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;

        assert_eq!(discovery_calls(&package_root), Vec::<String>::new());
        drop(backend);
        Ok(())
    }

    /// Discovery is an on-demand plugin call carrying the Workspace's own directory, and it creates
    /// no provider session on Ora's side.
    ///
    /// Both halves matter together: the plugin needs the directory to answer for the right project,
    /// and answering must cost nothing that has to be cleaned up if the user never sends anything.
    #[test]
    fn model_discovery_passes_the_workspace_directory_and_creates_no_session()
    -> Result<(), Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.create_project(CreateProjectRequest {
            name: "Session E2E".to_string(),
            main_workspace_path: workspace.to_string_lossy().into_owned(),
        })?;
        let workspace_id = main_workspace_id(&backend)?;

        let runtime = current_thread_runtime()?;
        let models = runtime.block_on(backend.list_agent_models(ListAgentModelsRequest {
            agent_ref: AGENT_REF.to_string(),
            workspace_id,
        }))?;

        assert!(
            !models.models.is_empty(),
            "the fake agent advertises a catalog through the plugin control channel",
        );
        assert_eq!(
            discovery_calls(&package_root),
            vec![workspace.to_string_lossy().into_owned()],
            "discovery must be asked exactly once, against the Workspace's own directory",
        );
        assert_eq!(
            backend.list_sessions(ListSessionsRequest {})?.sessions,
            Vec::new(),
            "listing models must not leave a session behind",
        );
        Ok(())
    }

    /// Sending is what creates a session, and the pre-session model intent is applied to it.
    ///
    /// Nothing exists to configure before this call, so the model has to travel with it — the
    /// alternative is a second round trip whose failure would leave a session on the wrong model.
    #[test]
    fn starting_a_session_applies_the_selected_model() -> Result<(), Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.create_project(CreateProjectRequest {
            name: "Session E2E".to_string(),
            main_workspace_path: workspace.to_string_lossy().into_owned(),
        })?;
        let workspace_id = main_workspace_id(&backend)?;
        let runtime = current_thread_runtime()?;
        let chosen = runtime
            .block_on(backend.list_agent_models(ListAgentModelsRequest {
                agent_ref: AGENT_REF.to_string(),
                workspace_id: workspace_id.clone(),
            }))?
            .models
            .into_iter()
            .find(|model| !model.default)
            .ok_or("the fake agent offers a model other than its default")?
            .id;

        let started = runtime.block_on(backend.start_session(StartSessionRequest {
            workspace_id: workspace_id.clone(),
            agent_ref: AGENT_REF.to_string(),
            model: Some(chosen.clone()),
        }))?;

        assert_eq!(started.session.workspace_id, workspace_id);
        assert_eq!(
            selected_model(&started.config_options),
            Some(chosen),
            "the response carries the session's own options, showing the intent took effect",
        );
        assert_eq!(
            backend
                .list_sessions(ListSessionsRequest {})?
                .sessions
                .into_iter()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            vec![started.session.id],
            "exactly one session exists, and it is the one just started",
        );
        Ok(())
    }

    /// Reopening a conversation after a restart reads the record; the next message attaches.
    ///
    /// Nothing about a conversation Ora already recorded needs the agent, and requiring one is
    /// what made a session unreadable once its plugin was uninstalled or its CLI stopped starting.
    /// The provider is acquired by the send that actually needs it, one call later.
    #[test]
    fn reopening_a_conversation_attaches_only_when_the_next_message_is_sent()
    -> Result<(), Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let runtime = current_thread_runtime()?;
        let session_id = {
            let backend = open_ready_backend(&setup)?;
            let workspace_id = seed_workspace(&setup, &backend)?;
            runtime
                .block_on(backend.start_session(StartSessionRequest {
                    workspace_id,
                    agent_ref: AGENT_REF.to_string(),
                    model: None,
                }))?
                .session
                .id
        };

        // A fresh Backend over the same data directory is what a restart leaves behind: the row
        // survives, every actor and provider session does not.
        let backend = open_ready_backend(&setup)?;
        runtime.block_on(drain_load(backend.load_session(LoadSessionRequest {
            session_id: session_id.clone(),
        })))?;

        assert_eq!(
            acp_calls(&package_root),
            vec!["session/new fake-session-1".to_string()],
            "reading a conversation must ask the agent for nothing",
        );

        runtime.block_on(drain_prompt(backend.prompt_session(PromptSessionRequest {
            session_id,
            prompt: vec![ContentBlock::Text(TextContent::new("hello"))],
            record_prompt: None,
        })))?;

        assert_eq!(
            acp_calls(&package_root),
            vec![
                "session/new fake-session-1".to_string(),
                "session/load fake-session-1".to_string(),
                "session/prompt fake-session-1".to_string(),
            ],
            "the send is what restores the provider session it needs",
        );
        Ok(())
    }

    /// A session the agent can no longer restore is rebuilt, and the turn runs on the new one.
    ///
    /// Losing the provider session is not losing the conversation: Ora holds the transcript and
    /// the next prompt carries it into a replacement, so the user's message goes through instead
    /// of failing against an identity the agent cannot resolve.
    #[test]
    fn a_session_the_agent_cannot_restore_is_rebuilt_for_the_next_message()
    -> Result<(), Box<dyn std::error::Error>> {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let runtime = current_thread_runtime()?;
        let session_id = {
            let backend = open_ready_backend(&setup)?;
            let workspace_id = seed_workspace(&setup, &backend)?;
            runtime
                .block_on(backend.start_session(StartSessionRequest {
                    workspace_id,
                    agent_ref: AGENT_REF.to_string(),
                    model: None,
                }))?
                .session
                .id
        };
        fs::write(package_root.join(LOAD_REFUSAL_MARKER), "")?;

        let backend = open_ready_backend(&setup)?;
        runtime.block_on(drain_load(backend.load_session(LoadSessionRequest {
            session_id: session_id.clone(),
        })))?;
        runtime.block_on(drain_prompt(backend.prompt_session(PromptSessionRequest {
            session_id,
            prompt: vec![ContentBlock::Text(TextContent::new("hello"))],
            record_prompt: None,
        })))?;

        // The identity repeats because the restarted agent numbers its sessions from one again;
        // the ordering is the evidence: a `session/new` between the refusal and the turn is the
        // replacement being built, and it is what the prompt then runs on.
        assert_eq!(
            acp_calls(&package_root),
            vec![
                "session/new fake-session-1".to_string(),
                "session/load fake-session-1".to_string(),
                "session/new fake-session-1".to_string(),
                "session/prompt fake-session-1".to_string(),
            ],
            "a refused restore is answered with a replacement session that serves the turn",
        );
        Ok(())
    }

    /// Creates the project checkout these session tests run against and returns its Workspace.
    fn seed_workspace(
        setup: &DesktopTestSetup,
        backend: &Backend,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.create_project(CreateProjectRequest {
            name: "Session E2E".to_string(),
            main_workspace_path: workspace.to_string_lossy().into_owned(),
        })?;
        main_workspace_id(backend)
    }

    /// Consumes a finite load stream, surfacing the first failure it carries.
    async fn drain_load(
        opening: impl Future<Output = Result<SessionEventStream<LoadSessionEvent>, BackendError>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut stream = opening.await?;
        while let Some(event) = stream.recv().await {
            event?;
        }
        Ok(())
    }

    /// Consumes one prompt turn, surfacing the first failure it carries.
    async fn drain_prompt(
        sending: impl Future<Output = Result<SessionEventStream<PromptSessionEvent>, BackendError>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut stream = sending.await?;
        while let Some(event) = stream.recv().await {
            event?;
        }
        Ok(())
    }

    /// Returns the session-lifecycle ACP calls the fake agent recorded, in order.
    fn acp_calls(package_root: &Path) -> Vec<String> {
        match fs::read_to_string(package_root.join(ACP_JOURNAL)) {
            Ok(journal) => journal.lines().map(str::to_string).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Reads the value the agent reports as current on its model selector.
    fn selected_model(config_options: &[SessionConfigOption]) -> Option<String> {
        config_options.iter().find_map(|option| match &option.kind {
            SessionConfigKind::Select(select) => Some(select.current_value.0.to_string()),
            _ => None,
        })
    }

    /// Returns the directories the fake plugin recorded for each discovery call it served.
    fn discovery_calls(package_root: &Path) -> Vec<String> {
        match fs::read_to_string(package_root.join(DISCOVERY_JOURNAL)) {
            Ok(journal) => journal.lines().map(str::to_string).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Resolves the main Workspace the project checkout created.
    fn main_workspace_id(backend: &Backend) -> Result<String, Box<dyn std::error::Error>> {
        Ok(backend
            .list_workspaces(ListWorkspacesRequest {})?
            .workspaces
            .into_iter()
            .find(|workspace| workspace.kind == WorkspaceKind::Main)
            .ok_or("project checkout has no main Workspace")?
            .id)
    }

    /// Opens a Backend whose only agent is the E2E fake, and waits for it to report Ready.
    fn open_ready_backend(setup: &DesktopTestSetup) -> Result<Backend, Box<dyn std::error::Error>> {
        let mut backend_paths = setup.backend_paths().clone();
        backend_paths.deno_path = env!("CARGO_BIN_EXE_fake-agent").into();
        let backend = Backend::open(backend_paths)?;
        wait_until("fake OpenCode agent did not become ready", || {
            backend
                .get_agent_runtime_status(GetAgentRuntimeStatusRequest {})
                .is_ok_and(|response| {
                    response.statuses.iter().any(|runtime| {
                        runtime.agent_ref == AGENT_REF && runtime.status == AgentStatus::Ready
                    })
                })
        })?;
        Ok(backend)
    }

    /// Builds the runtime a synchronous test drives async Backend calls on.
    fn current_thread_runtime() -> io::Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    }

    /// Installs the package metadata that makes the E2E fake process discoverable as OpenCode.
    fn install_fake_opencode_plugin(home_directory: &Path) -> io::Result<PathBuf> {
        let package_root = home_directory
            .join("plugins")
            .join("installed")
            .join("official")
            .join(AGENT_REF)
            .join("1.0.0");
        fs::create_dir_all(&package_root)?;
        fs::write(package_root.join("main.js"), "export {};\n")?;
        fs::write(
            package_root.join("orax.toml"),
            "resolver = 1\nidentifier = \"ora-space.opencode\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"OpenCode E2E agent\"\n",
        )?;
        Ok(package_root)
    }

    /// Polls an asynchronous external observation until it becomes true or the deadline passes.
    fn wait_until(message: &str, mut condition: impl FnMut() -> bool) -> io::Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if condition() {
                return Ok(());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, message))
    }
}
