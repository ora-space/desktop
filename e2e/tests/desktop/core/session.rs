//! Integration coverage for lazy session creation and plugin-owned model discovery.

mod tests {
    use crate::setup::DesktopTestSetup;
    use agent_client_protocol_schema::v1::{SessionConfigKind, SessionConfigOption};
    use ora_backend::Backend;
    use ora_contracts::{
        AgentStatus, CreateProjectRequest, GetAgentRuntimeStatusRequest, ListAgentModelsRequest,
        ListSessionsRequest, ListWorkspacesRequest, StartSessionRequest, WorkspaceKind,
    };
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    const AGENT_NAMESPACE: &str = "official";
    const AGENT_NAME: &str = "ora-space.opencode";
    const READY_TIMEOUT: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(10);
    /// Journal the fake agent appends one line to per `agent/list_models` call.
    const DISCOVERY_JOURNAL: &str = "list_models_calls.txt";

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
            agent_ref: agent_ref(),
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
                agent_ref: agent_ref(),
                workspace_id: workspace_id.clone(),
            }))?
            .models
            .into_iter()
            .find(|model| !model.default)
            .ok_or("the fake agent offers a model other than its default")?
            .id;

        let started = runtime.block_on(backend.start_session(StartSessionRequest {
            workspace_id: workspace_id.clone(),
            agent_ref: agent_ref(),
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

    /// Names the agent the way the runtime reports it: an agent is its whole canonical plugin id,
    /// so the namespace the package is installed under is part of the identity, not a prefix the
    /// caller may drop.
    fn agent_ref() -> String {
        format!("{AGENT_NAMESPACE}/{AGENT_NAME}")
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
        let expected = agent_ref();
        wait_until("fake OpenCode agent did not become ready", || {
            backend
                .get_agent_runtime_status(GetAgentRuntimeStatusRequest {})
                .is_ok_and(|response| {
                    response.statuses.iter().any(|runtime| {
                        runtime.agent_ref == expected && runtime.status == AgentStatus::Ready
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
            .join(AGENT_NAMESPACE)
            .join(AGENT_NAME)
            .join("1.0.0");
        fs::create_dir_all(&package_root)?;
        fs::write(package_root.join("main.js"), "export {};\n")?;
        // The installed tree is the authority for a package's namespace, so the manifest only
        // spells the identifier half; the directory it is written under supplies the rest.
        fs::write(
            package_root.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"{AGENT_NAME}\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"OpenCode E2E agent\"\n"
            ),
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
