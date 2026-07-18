use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ora_plugin_protocol::PluginVersion;
use ora_process::{
    ManagedProcessTree, ProcessExit, ProcessSpec, ProcessTreeController, ProcessTreeError,
    ProcessTreeSpawner,
};
use tokio::sync::mpsc;

use crate::{
    LaunchValueResolver, PluginError, ReaderEvent, ResolvedLaunchValue, RuntimeAssetLease,
    StderrDrainSummary, ValidatedLaunchDescriptor, WriterCompletion, WriterQueueLimits,
    WriterQueues, spawn_reader, spawn_stderr_drain, spawn_writer,
};

/// One explicitly allowlisted environment entry for the private bootstrap process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEnvironmentBinding {
    pub key: OsString,
    pub value: OsString,
}

/// Verified active runtime paths consumed by process launch rather than the ambient PATH.
#[derive(Clone)]
pub struct PluginRuntimeAssets {
    bun_executable: PathBuf,
    bootstrap_entry: PathBuf,
    empty_bunfig: PathBuf,
    pub runtime_version: PluginVersion,
    environment: Vec<RuntimeEnvironmentBinding>,
    lease: Option<RuntimeAssetLease>,
}

impl PluginRuntimeAssets {
    pub fn new(
        bun_executable: impl Into<PathBuf>,
        bootstrap_entry: impl Into<PathBuf>,
        empty_bunfig: impl Into<PathBuf>,
        runtime_version: PluginVersion,
    ) -> Self {
        Self {
            bun_executable: bun_executable.into(),
            bootstrap_entry: bootstrap_entry.into(),
            empty_bunfig: empty_bunfig.into(),
            runtime_version,
            environment: Vec::new(),
            lease: None,
        }
    }

    /// Converts a deployed asset lease into paths that are revalidated before every generation.
    pub async fn from_runtime_lease(lease: RuntimeAssetLease) -> Result<Self, PluginError> {
        let mut assets = lease.launch_assets().await?;
        assets.lease = Some(lease);
        Ok(assets)
    }

    /// Adds one Host-owned environment binding without inheriting the ambient process map.
    pub fn with_environment(
        mut self,
        key: impl Into<OsString>,
        value: impl Into<OsString>,
    ) -> Self {
        self.environment.push(RuntimeEnvironmentBinding {
            key: key.into(),
            value: value.into(),
        });
        self
    }

    pub fn bun_executable(&self) -> &Path {
        &self.bun_executable
    }

    pub fn bootstrap_entry(&self) -> &Path {
        &self.bootstrap_entry
    }

    pub fn empty_bunfig(&self) -> &Path {
        &self.empty_bunfig
    }

    /// Rebuilds launch paths from the active lease so a later tamper cannot reuse stale proof.
    pub async fn verified_for_spawn(&self) -> Result<Self, PluginError> {
        let Some(lease) = &self.lease else {
            return Ok(self.clone());
        };
        let mut assets = lease.launch_assets().await?;
        assets.environment = self.environment.clone();
        assets.lease = Some(lease.clone());
        Ok(assets)
    }

    /// Resolves one grant at the spawn boundary and constructs the clear+allowlist environment.
    pub async fn process_spec<Resolver>(
        &self,
        descriptor: &ValidatedLaunchDescriptor,
        resolver: &Resolver,
    ) -> Result<ProcessSpec, PluginError>
    where
        Resolver: LaunchValueResolver,
    {
        // Bun accepts an alternate bunfig only in the single-token `--config=<path>` form. Passing
        // the path as the next argv item exits successfully without executing the entry point.
        let mut bunfig_argument = OsString::from("--config=");
        bunfig_argument.push(&self.empty_bunfig);
        let mut spec = ProcessSpec::new(self.bun_executable.as_os_str())
            .arg(bunfig_argument)
            .arg("--no-env-file")
            .arg("run")
            .arg("--no-install")
            .arg(self.bootstrap_entry.as_os_str())
            .cwd(&descriptor.extension_path)
            .clear_and_allowlist_environment();
        let mut targets = std::collections::BTreeSet::new();
        for binding in &self.environment {
            let key = binding.key.to_string_lossy().to_ascii_uppercase();
            if !targets.insert(key) || contains_nul(&binding.value) {
                return Err(PluginError::PluginRuntimeUnavailable);
            }
            spec = spec.env(&binding.key, &binding.value);
        }
        if let Some(grant) = &descriptor.launch_grant {
            for binding in &grant.environment {
                if !targets.insert(binding.target.as_str().to_ascii_uppercase()) {
                    return Err(PluginError::LaunchGrantUnavailable {
                        plugin_id: descriptor.plugin_id.clone(),
                    });
                }
                let value = resolver.resolve(&binding.value).await.map_err(|_| {
                    PluginError::LaunchGrantUnavailable {
                        plugin_id: descriptor.plugin_id.clone(),
                    }
                })?;
                let value = match value {
                    ResolvedLaunchValue::Plain { value } => value,
                    ResolvedLaunchValue::Secret { value } => {
                        value.expose_for_process().to_os_string()
                    }
                };
                if contains_nul(&value) {
                    return Err(PluginError::LaunchGrantUnavailable {
                        plugin_id: descriptor.plugin_id.clone(),
                    });
                }
                spec = spec.env(binding.target.as_str(), value);
            }
        }
        Ok(spec)
    }
}

/// Rejects interior NUL before platform environment-block construction.
#[cfg(windows)]
fn contains_nul(value: &std::ffi::OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().any(|unit| unit == 0)
}

#[cfg(not(windows))]
fn contains_nul(value: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().contains(&0)
}

/// Process and pipe events observed independently so exit-first order remains representable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationProcessEvent {
    DirectExit(Result<ProcessExit, ProcessTreeError>),
    TreeEmpty(Result<(), ProcessTreeError>),
    StderrDrained(StderrDrainSummary),
}

/// Long-lived capabilities handed to exactly one generation actor after contained spawn.
pub struct GenerationTransport<Controller> {
    pub generation: u64,
    pub process_id: u32,
    pub controller: Controller,
    pub writer: WriterQueues,
    pub writer_events: mpsc::Receiver<WriterCompletion>,
    pub reader_events: mpsc::Receiver<ReaderEvent>,
    pub process_events: mpsc::Receiver<GenerationProcessEvent>,
}

/// Creates one contained generation and starts I/O watchers without interpreting plugin business.
pub trait GenerationLauncher: Clone + Send + Sync + 'static {
    type Controller: ProcessTreeController;

    fn launch(
        &self,
        generation: u64,
        spec: ProcessSpec,
    ) -> impl Future<Output = Result<GenerationTransport<Self::Controller>, GenerationLaunchError>> + Send;
}

/// Adapts any statically-dispatched process-tree spawner into the runtime generation boundary.
pub struct ProcessTreeGenerationLauncher<Spawner> {
    spawner: Arc<Spawner>,
    maximum_json_depth: usize,
    reader_event_capacity: usize,
    stderr_retained_bytes: usize,
    writer_limits: WriterQueueLimits,
}

impl<Spawner> Clone for ProcessTreeGenerationLauncher<Spawner> {
    fn clone(&self) -> Self {
        Self {
            spawner: Arc::clone(&self.spawner),
            maximum_json_depth: self.maximum_json_depth,
            reader_event_capacity: self.reader_event_capacity,
            stderr_retained_bytes: self.stderr_retained_bytes,
            writer_limits: self.writer_limits.clone(),
        }
    }
}

impl<Spawner> ProcessTreeGenerationLauncher<Spawner> {
    pub fn new(spawner: Spawner) -> Self {
        Self {
            spawner: Arc::new(spawner),
            maximum_json_depth: 64,
            reader_event_capacity: 256,
            stderr_retained_bytes: 256 * 1024,
            writer_limits: WriterQueueLimits::v1_defaults(),
        }
    }

    /// Overrides bounded transport resources while retaining one immutable launcher profile.
    pub fn with_transport_limits(
        mut self,
        maximum_json_depth: usize,
        reader_event_capacity: usize,
        stderr_retained_bytes: usize,
        writer_limits: WriterQueueLimits,
    ) -> Self {
        self.maximum_json_depth = maximum_json_depth;
        self.reader_event_capacity = reader_event_capacity;
        self.stderr_retained_bytes = stderr_retained_bytes;
        self.writer_limits = writer_limits;
        self
    }
}

impl<Spawner> GenerationLauncher for ProcessTreeGenerationLauncher<Spawner>
where
    Spawner: ProcessTreeSpawner + Send + Sync + 'static,
    Spawner::ProcessTree: Send + 'static,
    <Spawner::ProcessTree as ManagedProcessTree>::Stdin: Send,
    <Spawner::ProcessTree as ManagedProcessTree>::Stdout: Send,
    <Spawner::ProcessTree as ManagedProcessTree>::Stderr: Send,
{
    type Controller = <Spawner::ProcessTree as ManagedProcessTree>::Controller;

    async fn launch(
        &self,
        generation: u64,
        spec: ProcessSpec,
    ) -> Result<GenerationTransport<Self::Controller>, GenerationLaunchError> {
        let spawner = Arc::clone(&self.spawner);
        let tree = tokio::task::spawn_blocking(move || spawner.spawn_tree(spec))
            .await
            .map_err(|_| GenerationLaunchError::SpawnWorkerFailed)?
            .map_err(GenerationLaunchError::Spawn)?;
        let process_id = tree.direct_process_id();
        let parts = tree
            .into_parts()
            .map_err(GenerationLaunchError::ProcessTree)?;
        let (writer, writer_events) = spawn_writer(parts.stdio.stdin, self.writer_limits.clone())
            .map_err(|_| GenerationLaunchError::WriterConfiguration)?;
        let reader_events = spawn_reader(
            parts.stdio.stdout,
            self.maximum_json_depth,
            self.reader_event_capacity,
        );
        let stderr_summary = spawn_stderr_drain(parts.stdio.stderr, self.stderr_retained_bytes);
        let (process_tx, process_events) = mpsc::channel(8);

        let direct_tx = process_tx.clone();
        tokio::spawn(async move {
            let _ = direct_tx
                .send(GenerationProcessEvent::DirectExit(parts.direct_exit.await))
                .await;
        });
        let tree_tx = process_tx.clone();
        tokio::spawn(async move {
            let _ = tree_tx
                .send(GenerationProcessEvent::TreeEmpty(parts.tree_empty.await))
                .await;
        });
        tokio::spawn(async move {
            if let Ok(summary) = stderr_summary.await {
                let _ = process_tx
                    .send(GenerationProcessEvent::StderrDrained(summary))
                    .await;
            }
        });

        Ok(GenerationTransport {
            generation,
            process_id,
            controller: parts.controller,
            writer,
            writer_events,
            reader_events,
            process_events,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GenerationLaunchError {
    #[error("process-tree spawn worker stopped")]
    SpawnWorkerFailed,
    #[error("contained process spawn failed")]
    Spawn(#[source] std::io::Error),
    #[error("process-tree capability transfer failed")]
    ProcessTree(#[source] ProcessTreeError),
    #[error("writer queue configuration is invalid")]
    WriterConfiguration,
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use ora_plugin_protocol::{
        ContentDigest, ContentOwnerId, JsonSafeU64, PluginId, PluginKind, PluginVersion,
    };
    use pretty_assertions::assert_eq;

    use super::PluginRuntimeAssets;
    use crate::{
        LaunchGrantError, LaunchValueReference, LaunchValueResolver, ResolvedLaunchValue,
        ValidatedLaunchDescriptor,
    };

    struct EmptyResolver;

    impl LaunchValueResolver for EmptyResolver {
        async fn resolve(
            &self,
            _reference: &LaunchValueReference,
        ) -> Result<ResolvedLaunchValue, LaunchGrantError> {
            Err(LaunchGrantError::ReferenceUnavailable)
        }
    }

    /// Freezes the command prefix and proves the runtime never resolves Bun through PATH.
    #[tokio::test]
    async fn process_spec_uses_pinned_assets_and_empty_config() {
        let assets = PluginRuntimeAssets::new(
            PathBuf::from("runtime").join("bun.exe"),
            PathBuf::from("runtime").join("plugin-host-bootstrap.js"),
            PathBuf::from("runtime").join("empty-bunfig.toml"),
            PluginVersion::parse("1.0.0")
                .unwrap_or_else(|error| panic!("runtime version: {error}")),
        )
        .with_environment("SystemRoot", r"C:\Windows");
        let descriptor = ValidatedLaunchDescriptor {
            plugin_id: PluginId::parse("ora.runtime")
                .unwrap_or_else(|error| panic!("plugin id: {error}")),
            plugin_version: PluginVersion::parse("0.1.0")
                .unwrap_or_else(|error| panic!("plugin version: {error}")),
            kind: PluginKind::Agent,
            content_digest: ContentDigest::parse(format!("sha256:{}", "a".repeat(64)))
                .unwrap_or_else(|error| panic!("content digest: {error}")),
            content_owner: ContentOwnerId::parse(format!("sha256-{}", "b".repeat(64)))
                .unwrap_or_else(|error| panic!("content owner: {error}")),
            extension_path: PathBuf::from("plugin"),
            entry_path: PathBuf::from("plugin").join("dist").join("index.js"),
            storage_path: PathBuf::from("data"),
            declared_agents: Vec::new(),
            enablement_epoch: JsonSafeU64::new(1)
                .unwrap_or_else(|error| panic!("enablement epoch: {error}")),
            registry_revision: JsonSafeU64::new(1)
                .unwrap_or_else(|error| panic!("registry revision: {error}")),
            launch_grant: None,
        };

        let spec = assets
            .process_spec(&descriptor, &EmptyResolver)
            .await
            .unwrap_or_else(|error| panic!("process spec: {error}"));
        let mut expected_bunfig_argument = OsString::from("--config=");
        expected_bunfig_argument.push(PathBuf::from("runtime").join("empty-bunfig.toml"));
        assert_eq!(spec.program(), PathBuf::from("runtime").join("bun.exe"));
        assert_eq!(
            spec.args_iter().map(OsString::from).collect::<Vec<_>>(),
            vec![
                expected_bunfig_argument,
                OsString::from("--no-env-file"),
                OsString::from("run"),
                OsString::from("--no-install"),
                PathBuf::from("runtime")
                    .join("plugin-host-bootstrap.js")
                    .into_os_string(),
            ]
        );
    }
}
