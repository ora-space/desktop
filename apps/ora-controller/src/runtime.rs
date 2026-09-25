use super::*;
use serde::{Deserialize, Serialize};
use std::{future::Future, io, sync::Arc, time::Duration};
use tokio::sync::watch;

/// Which authority persists coordination for this deployment. Chosen once at deployment time: a
/// running Controller never switches adapters, and neither adapter is a fallback for the other,
/// because two authorities would leave nobody able to say which record is the fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Persistence {
    /// Local single-node deployments: the SQLite database and its lease live in `home_directory`.
    Sqlite,
    /// Cloud deployments: every durable operation is a call to the Cloud internal control contract
    /// at `endpoint` (a gRPC URI); no database is opened locally. Work accepted by Cloud is
    /// dispatched to the single configured Node. While a `Watch` stream is live, claims follow its
    /// signals and every lease renewal; `claim_interval_ms` is the claim cadence while no stream
    /// is live.
    Cloud {
        endpoint: String,
        claim_interval_ms: u64,
    },
}

/// Shared deployment configuration for the standalone executable and embedded HTTP composition.
/// `home_directory` is the process-private state root in both modes (API socket, and in SQLite
/// mode the database); it never holds cloud-authoritative records.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub home_directory: PathBuf,
    pub persistence: Persistence,
    pub protected_state_directories: Vec<PathBuf>,
    pub controller_id: ControllerId,
    pub nodes: Vec<NodeTarget>,
    pub session: SessionConfig,
    pub reconnect_ms: u64,
    pub timezone: String,
}

/// Owns deployment and the reconnect lifetime; callers supply their own process shutdown signal.
/// The store type is fixed at construction: one deployment runs exactly one persistence adapter.
pub struct ControllerRuntime<S: CoordinationStore> {
    handle: ControllerHandle<S>,
    config: RuntimeConfig,
}

/// Narrow application access to the durable store; the store itself decides how its work is executed.
pub struct ControllerHandle<S: CoordinationStore> {
    store: S,
    nodes: Arc<Vec<NodeId>>,
}

impl<S: CoordinationStore> Clone for ControllerHandle<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            nodes: self.nodes.clone(),
        }
    }
}

impl ControllerRuntime<SqliteStore> {
    /// Validates deployment before opening local state, preserving protected roots and exclusive ownership.
    /// Only SQLite persistence opens here; a cloud deployment is a different adapter, not a fallback.
    pub fn open(config: RuntimeConfig) -> Result<Self, Error> {
        match &config.persistence {
            Persistence::Sqlite => {}
            Persistence::Cloud { .. } => {
                return Err(Error::Configuration(
                    "persistence.kind is cloud; open the cloud adapter instead of sqlite".into(),
                ));
            }
        }
        Self::validate(&config)?;
        let store = SqliteStore::open(&config.home_directory, config.controller_id.clone())?;
        Ok(Self::with_store(config, store))
    }
}

impl ControllerRuntime<CloudStore> {
    /// Validates deployment and binds the Cloud adapter. No local database, lease or directory is
    /// created: `home_directory` stays the process-private root for the API socket and nothing else.
    pub fn open(config: RuntimeConfig) -> Result<Self, Error> {
        if config.persistence == Persistence::Sqlite {
            return Err(Error::Configuration(
                "persistence.kind is sqlite; open the sqlite adapter instead of cloud".into(),
            ));
        }
        Self::validate(&config)?;
        let store = CloudStore::open(&config)?;
        Ok(Self::with_store(config, store))
    }
}

impl<S: CoordinationStore> ControllerRuntime<S> {
    /// Deployment checks shared by every adapter, all before any state is touched.
    fn validate(config: &RuntimeConfig) -> Result<(), Error> {
        if !config.home_directory.is_absolute()
            || config.reconnect_ms == 0
            || config.session.query_interval_ms == 0
            || config.session.io_timeout_ms == 0
        {
            return Err(Error::InvalidStorage);
        }
        for (index, node) in config.nodes.iter().enumerate() {
            let endpoint_valid = match &node.endpoint {
                NodeEndpoint::Ipc { path } => path.is_absolute(),
                NodeEndpoint::WebSocket(endpoint) => endpoint.validate().is_ok(),
            };
            if node.node_id.as_str().trim().is_empty()
                || !endpoint_valid
                || config.nodes[..index]
                    .iter()
                    .any(|other| other.node_id == node.node_id || other.endpoint == node.endpoint)
            {
                return Err(Error::Conflict);
            }
        }
        let home = ora_utils::path::canonicalize_longest_existing_prefix(&config.home_directory);
        for root in config
            .protected_state_directories
            .iter()
            .map(PathBuf::as_path)
            .chain(
                // Only local sockets live on this filesystem; remote endpoints own no local state.
                config.nodes.iter().filter_map(|node| match &node.endpoint {
                    NodeEndpoint::Ipc { path } => path.parent(),
                    NodeEndpoint::WebSocket(_) => None,
                }),
            )
        {
            if !root.is_absolute() {
                return Err(Error::InvalidStorage);
            }
            let root = ora_utils::path::canonicalize_longest_existing_prefix(root);
            if home.starts_with(&root) || root.starts_with(&home) {
                return Err(Error::InvalidStorage);
            }
        }
        Ok(())
    }

    /// Binds validated deployment to an already opened store; adapters validate their own state.
    fn with_store(config: RuntimeConfig, store: S) -> Self {
        let nodes = Arc::new(
            config
                .nodes
                .iter()
                .map(|node| node.node_id.clone())
                .collect(),
        );
        Self {
            handle: ControllerHandle { store, nodes },
            config,
        }
    }

    /// Supplies application access without exposing the store, mutex or reconnect implementation.
    pub fn handle(&self) -> ControllerHandle<S> {
        self.handle.clone()
    }

    /// Reconnects configured Nodes and runs the adapter's authority coordination until shutdown.
    /// Sessions are aborted first so nothing writes under a lease the adapter is about to release.
    pub async fn run(&self, shutdown: impl Future<Output = ()>) -> io::Result<()> {
        let mut sessions = tokio::task::JoinSet::new();
        for target in self.config.nodes.clone() {
            let store = self.handle.store.clone();
            let settings = self.config.session.clone();
            let delay = Duration::from_millis(self.config.reconnect_ms);
            sessions.spawn(async move {
                loop {
                    if let Err(error) = run_session(&store, &target, &settings).await { ora_logging::ora_warn!(node_id = %target.node_id.as_str(), error = %error, "Controller connection unavailable; original execution responsibility retained"); }
                    tokio::time::sleep(delay).await;
                }
            });
        }
        let (stop_serving, serving_stopping) = watch::channel(false);
        let store = self.handle.store.clone();
        let mut serving = tokio::spawn(async move {
            store
                .serve(async move {
                    let mut stopping = serving_stopping;
                    let _ = stopping.changed().await;
                })
                .await
        });
        let mut serving_done = false;
        ora_logging::ora_info!("Controller recovery started");
        let result = tokio::select! {
            _ = shutdown => Ok(()),
            result = sessions.join_next(), if !sessions.is_empty() => Err(io::Error::other(format!("Controller session task stopped: {result:?}"))),
            result = &mut serving => {
                serving_done = true;
                Err(io::Error::other(format!("Controller authority coordination stopped: {result:?}")))
            }
        };
        sessions.abort_all();
        while sessions.join_next().await.is_some() {}
        let _ = stop_serving.send(true);
        if !serving_done {
            // Releasing a remote lease is bounded; a hung authority must not hold up shutdown.
            match tokio::time::timeout(Duration::from_secs(/*secs*/ 5), &mut serving).await {
                Ok(_) => {}
                Err(_) => serving.abort(),
            }
        }
        result
    }
}

impl<S: CloneIntake> ControllerHandle<S> {
    /// Accepts only a deployment-configured target before any Node dispatch observes the operation.
    pub async fn accept_clone(
        &self,
        request: RequestId,
        spec: CloneExecutionSpec,
    ) -> Result<CloneRepositoryMessage, Error> {
        if !self.nodes.contains(&spec.node_id) {
            return Err(Error::Conflict);
        }
        self.store.accept_request(request, spec).await
    }

    /// Returns accepted operations, including pending responsibility while Nodes are disconnected.
    pub async fn operations(&self) -> Result<Vec<CloneOperation>, Error> {
        self.store.operations().await
    }

    /// Reads one operation without confusing missing identity with an unknown terminal result.
    pub async fn operation(&self, execution: ExecutionId) -> Result<Option<CloneOperation>, Error> {
        self.store.operation(&execution).await
    }
}
