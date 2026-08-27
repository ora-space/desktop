//! Completes one launch attempt and owns the background tasks that watch the running process.
//!
//! Every transition made here is guarded by the launch attempt: a stale task that outlives a
//! newer stop or restart observes the mismatch and returns without touching state.

use crate::connection::PluginGenerationKey;
use crate::permissions::permissions_for;
use crate::ports::{
    InboundNotification, LaunchedRuntime, PluginLaunchRequest, PluginRuntime, PluginRuntimeExit,
    PluginRuntimeFailure, PluginRuntimeLauncher, PluginStatusPublisher,
};
use crate::registration::validate_registration;
use crate::state::ManagedPluginState;
use crate::{PluginLifecycleInner, PluginNotificationSink};
use ora_domain::PluginId;
use ora_plugin_manager::InstalledPlugin as DiscoveredPlugin;
use ora_plugin_runtime::PluginNotification;
use std::sync::{Arc, PoisonError};
use std::time::Duration;
use tokio::sync::{OwnedMutexGuard, mpsc};
use tokio::time::timeout;

/// How long a closed notification stream may precede process exit before it counts as failure.
///
/// Both intentional shutdown and protocol failure close the stream a moment before the process
/// exit is reported, and the exit monitor classifies those correctly. Only a stream that closes
/// while the process keeps running is a dead reader, which this grace period isolates.
const NOTIFICATION_CLOSE_GRACE: Duration = Duration::from_secs(1);

/// Completes one launch attempt without allowing stale work to overwrite a newer transition.
pub(crate) async fn complete_launch<RuntimeLauncher, StatusPublisher, NotificationSink>(
    inner: Arc<PluginLifecycleInner<RuntimeLauncher, StatusPublisher, NotificationSink>>,
    plugin_id: PluginId,
    plugin: DiscoveredPlugin,
    attempt: u64,
    _operation: OwnedMutexGuard<()>,
) where
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
    NotificationSink: PluginNotificationSink,
{
    // The data directory must exist before launch: the storage handler canonicalizes it on every
    // request, and the surface layer writes downloads there before the process is up.
    let data_dir = match inner.data_directories.ensure(&plugin_id) {
        Ok(data_dir) => data_dir,
        Err(error) => {
            transition_to_failed(
                inner,
                plugin_id,
                attempt,
                PluginRuntimeFailure::new(format!(
                    "failed to create plugin data directory: {error}"
                )),
            );
            return;
        }
    };
    // Activation refuses process-less kinds before reaching here; the check stays so a future
    // caller cannot turn "no entrypoint" into a panic or a launch of nothing.
    let Some(entrypoint) = plugin.contributes.entrypoint() else {
        transition_to_failed(
            inner,
            plugin_id,
            attempt,
            PluginRuntimeFailure::new("plugin kind has no process to launch"),
        );
        return;
    };
    let launch = inner
        .launcher
        .launch(PluginLaunchRequest {
            plugin_id: plugin_id.clone(),
            deno_path: inner.config.deno_path.clone(),
            entrypoint: plugin.package_root.join(entrypoint.to_path_buf()),
            package_root: plugin.package_root.clone(),
            permissions: permissions_for(&plugin.contributes),
            data_dir,
        })
        .await;

    let LaunchedRuntime {
        runtime,
        notifications,
    } = match launch {
        Ok(launched) => launched,
        Err(failure) => {
            transition_to_failed(inner, plugin_id, attempt, failure);
            return;
        }
    };
    if let Err(failure) = validate_registration(&plugin.contributes, &runtime.registration()) {
        let _ = runtime.stop().await;
        transition_to_failed(inner, plugin_id, attempt, failure);
        return;
    }

    let transitioned = {
        let mut state = inner.state.write().unwrap_or_else(PoisonError::into_inner);
        let owns_attempt = matches!(
            state.managed(&plugin_id),
            Some(ManagedPluginState::Starting {
                attempt: current,
            }) if *current == attempt
        );
        if owns_attempt {
            state.set_managed(
                &plugin_id,
                ManagedPluginState::Running {
                    attempt,
                    runtime: runtime.clone(),
                },
            );
        }
        owns_attempt
    };
    if !transitioned {
        let _ = runtime.stop().await;
        return;
    }

    inner.publisher.publish_status_changed(&plugin_id);
    tokio::spawn(pump_notifications(
        Arc::clone(&inner),
        plugin_id.clone(),
        attempt,
        runtime.clone(),
        notifications,
    ));
    tokio::spawn(async move {
        match runtime.wait_for_exit().await {
            PluginRuntimeExit::Stopped => transition_to_stopped(inner, plugin_id, attempt),
            PluginRuntimeExit::Failed(failure) => {
                transition_to_failed(inner, plugin_id, attempt, failure);
            }
        }
    });
}

/// Forwards plugin notifications to the sink and reports a reader that died under a live process.
async fn pump_notifications<RuntimeLauncher, StatusPublisher, NotificationSink>(
    inner: Arc<PluginLifecycleInner<RuntimeLauncher, StatusPublisher, NotificationSink>>,
    plugin_id: PluginId,
    attempt: u64,
    runtime: RuntimeLauncher::Runtime,
    mut notifications: mpsc::UnboundedReceiver<PluginNotification>,
) where
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
    NotificationSink: PluginNotificationSink,
{
    while let Some(notification) = notifications.recv().await {
        inner.sink.on_notification(InboundNotification {
            plugin_id: plugin_id.clone(),
            generation: PluginGenerationKey(attempt),
            method: notification.method,
            params: notification.params,
        });
    }
    // The exit monitor owns classification whenever the process actually exits; only a stream
    // that closes while the process stays alive is this task's failure to report.
    if timeout(NOTIFICATION_CLOSE_GRACE, runtime.wait_for_exit())
        .await
        .is_err()
    {
        transition_to_failed(
            inner,
            plugin_id,
            attempt,
            PluginRuntimeFailure::new("plugin notification channel closed"),
        );
    }
}

/// Records an intentional runtime exit only when its attempt still owns the running state.
pub(crate) fn transition_to_stopped<RuntimeLauncher, StatusPublisher, NotificationSink>(
    inner: Arc<PluginLifecycleInner<RuntimeLauncher, StatusPublisher, NotificationSink>>,
    plugin_id: PluginId,
    attempt: u64,
) where
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
{
    let transitioned = {
        let mut state = inner.state.write().unwrap_or_else(PoisonError::into_inner);
        let owns_attempt = matches!(
            state.managed(&plugin_id),
            Some(ManagedPluginState::Running {
                attempt: current,
                ..
            }) if *current == attempt
        );
        if owns_attempt {
            state.set_managed(&plugin_id, ManagedPluginState::Stopped);
        }
        owns_attempt
    };
    if transitioned {
        inner.publisher.publish_status_changed(&plugin_id);
    }
}

/// Records a launch or runtime failure only when its attempt still owns the running state.
pub(crate) fn transition_to_failed<RuntimeLauncher, StatusPublisher, NotificationSink>(
    inner: Arc<PluginLifecycleInner<RuntimeLauncher, StatusPublisher, NotificationSink>>,
    plugin_id: PluginId,
    attempt: u64,
    failure: PluginRuntimeFailure,
) where
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
{
    let transitioned = {
        let mut state = inner.state.write().unwrap_or_else(PoisonError::into_inner);
        let owns_attempt = matches!(
            state.managed(&plugin_id),
            Some(ManagedPluginState::Starting {
                attempt: current,
            }) | Some(ManagedPluginState::Running {
                attempt: current,
                ..
            }) if *current == attempt
        );
        if owns_attempt {
            state.set_managed(
                &plugin_id,
                ManagedPluginState::Failed {
                    reason: failure.reason().to_string(),
                },
            );
        }
        owns_attempt
    };
    if transitioned {
        inner.publisher.publish_status_changed(&plugin_id);
    }
}
