use crate::{
    AgentInvocationHandle, AgentPluginRuntime, GenerationLauncher, LaunchGrantError,
    LaunchValueReference, LaunchValueResolver, PluginError, PluginManagerConfig,
    PluginRuntimeAssets, PluginRuntimeControl, PluginRuntimeEvent, PluginRuntimeEventSink,
    PluginRuntimeInvocation, ResolvedLaunchValue, RuntimeAdmissionProvider, StopReason,
    ValidatedLaunchDescriptor, spawn_agent_plugin_runtime,
};
use ora_plugin_protocol::{AgentRequest, PluginId};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, Weak};
use tokio::sync::Mutex;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Object-safe bridge used only to break the management↔runtime construction cycle.
trait ErasedAdmission: Send + Sync {
    fn admit<'a>(
        &'a self,
        plugin_id: &'a PluginId,
    ) -> BoxFuture<'a, Result<ValidatedLaunchDescriptor, PluginError>>;

    fn recheck_after_activate<'a>(
        &'a self,
        descriptor: &'a ValidatedLaunchDescriptor,
    ) -> BoxFuture<'a, Result<(), PluginError>>;
}

impl<Admission> ErasedAdmission for Admission
where
    Admission: RuntimeAdmissionProvider + Send + Sync,
{
    fn admit<'a>(
        &'a self,
        plugin_id: &'a PluginId,
    ) -> BoxFuture<'a, Result<ValidatedLaunchDescriptor, PluginError>> {
        Box::pin(RuntimeAdmissionProvider::admit(self, plugin_id))
    }

    fn recheck_after_activate<'a>(
        &'a self,
        descriptor: &'a ValidatedLaunchDescriptor,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(RuntimeAdmissionProvider::recheck_after_activate(
            self, descriptor,
        ))
    }
}

/// Late-bound admission port; it fails closed until the management service is fully bootstrapped.
#[derive(Clone, Default)]
struct AdmissionBinding {
    inner: Arc<OnceLock<Weak<dyn ErasedAdmission>>>,
}

impl AdmissionBinding {
    fn bind<Admission>(&self, admission: Arc<Admission>) -> Result<(), PluginError>
    where
        Admission: RuntimeAdmissionProvider + Send + Sync + 'static,
    {
        let admission: Arc<dyn ErasedAdmission> = admission;
        self.inner
            .set(Arc::downgrade(&admission))
            .map_err(|_| PluginError::Internal {
                message: "runtime admission was already bound".to_owned(),
            })
    }
}

impl RuntimeAdmissionProvider for AdmissionBinding {
    async fn admit(&self, plugin_id: &PluginId) -> Result<ValidatedLaunchDescriptor, PluginError> {
        let admission = self
            .inner
            .get()
            .and_then(Weak::upgrade)
            .ok_or(PluginError::BackendShuttingDown)?;
        admission.admit(plugin_id).await
    }

    async fn recheck_after_activate(
        &self,
        descriptor: &ValidatedLaunchDescriptor,
    ) -> Result<(), PluginError> {
        let admission = self
            .inner
            .get()
            .and_then(Weak::upgrade)
            .ok_or(PluginError::BackendShuttingDown)?;
        admission.recheck_after_activate(descriptor).await
    }
}

/// Object-safe bridge for the late-bound critical event sink.
trait ErasedEventSink: Send + Sync {
    fn record<'a>(&'a self, event: PluginRuntimeEvent) -> BoxFuture<'a, Result<(), PluginError>>;
}

impl<Events> ErasedEventSink for Events
where
    Events: PluginRuntimeEventSink + Send + Sync,
{
    fn record<'a>(&'a self, event: PluginRuntimeEvent) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(PluginRuntimeEventSink::record(self, event))
    }
}

/// Late-bound critical event port paired with the admission binding.
#[derive(Clone, Default)]
struct EventBinding {
    inner: Arc<OnceLock<Arc<dyn ErasedEventSink>>>,
}

impl EventBinding {
    fn bind<Events>(&self, events: Arc<Events>) -> Result<(), PluginError>
    where
        Events: PluginRuntimeEventSink + Send + Sync + 'static,
    {
        self.inner.set(events).map_err(|_| PluginError::Internal {
            message: "runtime event sink was already bound".to_owned(),
        })
    }
}

impl PluginRuntimeEventSink for EventBinding {
    async fn record(&self, event: PluginRuntimeEvent) -> Result<(), PluginError> {
        let events = self.inner.get().ok_or(PluginError::BackendShuttingDown)?;
        events.record(event).await
    }
}

struct HubState<Controller> {
    closed_admission: BTreeSet<PluginId>,
    runtimes: BTreeMap<PluginId, AgentPluginRuntime<Controller>>,
}

impl<Controller> Default for HubState<Controller> {
    fn default() -> Self {
        Self {
            closed_admission: BTreeSet::new(),
            runtimes: BTreeMap::new(),
        }
    }
}

struct PluginRuntimeHubInner<Launcher, Resolver>
where
    Launcher: GenerationLauncher,
{
    config: PluginManagerConfig,
    assets: PluginRuntimeAssets,
    launcher: Launcher,
    resolver: Arc<Resolver>,
    admission: AdmissionBinding,
    events: EventBinding,
    state: Mutex<HubState<Launcher::Controller>>,
}

/// Multi-plugin runtime facade that creates one lazy single-flight supervisor per plugin id.
pub struct PluginRuntimeHub<Launcher, Resolver>
where
    Launcher: GenerationLauncher,
{
    inner: Arc<PluginRuntimeHubInner<Launcher, Resolver>>,
}

impl<Launcher, Resolver> Clone for PluginRuntimeHub<Launcher, Resolver>
where
    Launcher: GenerationLauncher,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<Launcher, Resolver> PluginRuntimeHub<Launcher, Resolver>
where
    Launcher: GenerationLauncher,
    Resolver: LaunchValueResolver + Send + Sync + 'static,
{
    pub fn new(
        config: PluginManagerConfig,
        assets: PluginRuntimeAssets,
        launcher: Launcher,
        resolver: Resolver,
    ) -> Self {
        Self {
            inner: Arc::new(PluginRuntimeHubInner {
                config,
                assets,
                launcher,
                resolver: Arc::new(resolver),
                admission: AdmissionBinding::default(),
                events: EventBinding::default(),
                state: Mutex::new(HubState::default()),
            }),
        }
    }

    /// Completes the construction cycle only after management bootstrap and reconciliation pass.
    pub fn bind<Admission, Events>(
        &self,
        admission: Arc<Admission>,
        events: Arc<Events>,
    ) -> Result<(), PluginError>
    where
        Admission: RuntimeAdmissionProvider + Send + Sync + 'static,
        Events: PluginRuntimeEventSink + Send + Sync + 'static,
    {
        self.inner.admission.bind(admission)?;
        self.inner.events.bind(events)
    }

    /// Gets or creates the plugin's supervisor while holding the hub's short metadata lock.
    async fn runtime(
        &self,
        plugin_id: &PluginId,
    ) -> Result<AgentPluginRuntime<Launcher::Controller>, PluginError> {
        let mut state = self.inner.state.lock().await;
        if state.closed_admission.contains(plugin_id) {
            return Err(PluginError::BackendShuttingDown);
        }
        if let Some(runtime) = state.runtimes.get(plugin_id) {
            return Ok(runtime.clone());
        }
        let runtime = spawn_agent_plugin_runtime(
            plugin_id.clone(),
            self.inner.config.clone(),
            self.inner.assets.clone(),
            self.inner.launcher.clone(),
            Arc::new(self.inner.admission.clone()),
            Arc::new(self.inner.events.clone()),
            Arc::clone(&self.inner.resolver),
        );
        state.runtimes.insert(plugin_id.clone(), runtime.clone());
        Ok(runtime)
    }

    /// Returns a runtime clone without creating a generation or supervisor.
    async fn existing(
        &self,
        plugin_id: &PluginId,
    ) -> Option<AgentPluginRuntime<Launcher::Controller>> {
        self.inner
            .state
            .lock()
            .await
            .runtimes
            .get(plugin_id)
            .cloned()
    }
}

impl<Launcher, Resolver> PluginRuntimeInvocation for PluginRuntimeHub<Launcher, Resolver>
where
    Launcher: GenerationLauncher,
    Resolver: LaunchValueResolver + Send + Sync + 'static,
{
    async fn start(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        self.runtime(plugin_id).await?.start().await
    }

    async fn stop(&self, plugin_id: &PluginId, reason: StopReason) -> Result<(), PluginError> {
        if let Some(runtime) = self.existing(plugin_id).await {
            runtime.stop_and_reap(plugin_id, reason).await?;
        }
        Ok(())
    }

    async fn invoke(
        &self,
        plugin_id: &PluginId,
        request: AgentRequest,
    ) -> Result<AgentInvocationHandle, PluginError> {
        if self
            .inner
            .state
            .lock()
            .await
            .closed_admission
            .contains(plugin_id)
        {
            return Err(PluginError::BackendShuttingDown);
        }
        if let Err(admission_error) =
            RuntimeAdmissionProvider::admit(&self.inner.admission, plugin_id).await
        {
            self.close_admission(plugin_id).await?;
            self.stop_and_reap(plugin_id, StopReason::ManualStop)
                .await?;
            return Err(admission_error);
        }
        self.runtime(plugin_id).await?.invoke(request).await
    }

    async fn shutdown_all(&self) -> Result<(), PluginError> {
        let runtimes = {
            let mut state = self.inner.state.lock().await;
            let plugin_ids = state.runtimes.keys().cloned().collect::<Vec<_>>();
            state.closed_admission.extend(plugin_ids);
            state
                .runtimes
                .iter()
                .map(|(plugin_id, runtime)| (plugin_id.clone(), runtime.clone()))
                .collect::<Vec<_>>()
        };
        let mut first_error = None;
        for (plugin_id, runtime) in runtimes {
            if let Err(error) = runtime.close_admission(&plugin_id).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
            if let Err(error) = runtime
                .stop_and_reap(&plugin_id, StopReason::Shutdown)
                .await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        self.inner.state.lock().await.runtimes.clear();
        first_error.map_or(Ok(()), Err)
    }
}

impl<Launcher, Resolver> PluginRuntimeControl for PluginRuntimeHub<Launcher, Resolver>
where
    Launcher: GenerationLauncher,
    Resolver: LaunchValueResolver + Send + Sync + 'static,
{
    async fn open_admission(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        self.inner
            .state
            .lock()
            .await
            .closed_admission
            .remove(plugin_id);
        Ok(())
    }

    async fn close_admission(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        let runtime = {
            let mut state = self.inner.state.lock().await;
            state.closed_admission.insert(plugin_id.clone());
            state.runtimes.get(plugin_id).cloned()
        };
        if let Some(runtime) = runtime {
            runtime.close_admission(plugin_id).await?;
        }
        Ok(())
    }

    async fn stop_and_reap(
        &self,
        plugin_id: &PluginId,
        reason: StopReason,
    ) -> Result<(), PluginError> {
        if let Some(runtime) = self.existing(plugin_id).await {
            runtime.stop_and_reap(plugin_id, reason).await?;
        }
        Ok(())
    }

    async fn reset_crash_loop(&self, plugin_id: &PluginId) -> Result<(), PluginError> {
        if let Some(runtime) = self.existing(plugin_id).await {
            runtime.reset_crash_loop().await?;
        }
        Ok(())
    }
}

/// Fail-closed production default until a product credential/config resolver is injected.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableLaunchValueResolver;

impl LaunchValueResolver for UnavailableLaunchValueResolver {
    async fn resolve(
        &self,
        _reference: &LaunchValueReference,
    ) -> Result<ResolvedLaunchValue, LaunchGrantError> {
        Err(LaunchGrantError::ReferenceUnavailable)
    }
}
