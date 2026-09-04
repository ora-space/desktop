//! Host-issued invocation contexts binding one consumer generation to one Ora session.

use ora_domain::PluginId;
use ora_plugin_runtime::PluginTraceProvider;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use uuid::Uuid;

/// Everything the trusted host resolves before a contextual plugin invocation.
#[derive(Debug, Clone)]
pub struct TraceContextGrant {
    /// The session the user opened the dashboard from. This identity never leaves the host.
    pub current_ora_session_id: String,
    /// Every trace source the host elected to make available to this dashboard invocation.
    pub sessions: Vec<TraceSessionGrant>,
}

/// One host-resolved session in a trace catalog. None of the provider identity or paths are
/// serialized to the plugin page; `label` is the only user-facing session identity.
#[derive(Debug, Clone)]
pub struct TraceSessionGrant {
    pub ora_session_id: String,
    pub provider_plugin_id: PluginId,
    pub provider_generation: PluginGenerationKey,
    pub provider_session_id: String,
    pub workspace_root: PathBuf,
    pub providers: Vec<PluginTraceProvider>,
    pub label: String,
    pub updated_at_ms: i64,
}

use crate::connection::PluginGenerationKey;

#[derive(Debug, Clone)]
pub(crate) struct AuthorizedTrace {
    pub trace_id: String,
    pub provider_id: String,
    pub format: String,
    pub path: PathBuf,
    pub containment_root: PathBuf,
    pub label: String,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredContext {
    pub consumer_plugin_id: PluginId,
    pub consumer_generation: PluginGenerationKey,
    pub trace: Option<TraceContextGrant>,
    pub traces: HashMap<String, AuthorizedTrace>,
    /// Keeps opaque handles stable across polling `list` calls for one live invocation.
    pub trace_ids: HashMap<String, String>,
}

/// Process-local registry of opaque, generation-bound context grants.
#[derive(Debug, Clone, Default)]
pub struct PluginInvocationContexts {
    inner: Arc<Mutex<HashMap<String, StoredContext>>>,
}

impl PluginInvocationContexts {
    /// Issues a new unguessable context id. Only trusted host code receives the grant fields.
    pub fn issue(
        &self,
        consumer_plugin_id: PluginId,
        consumer_generation: PluginGenerationKey,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id.clone(),
                StoredContext {
                    consumer_plugin_id,
                    consumer_generation,
                    trace: None,
                    traces: HashMap::new(),
                    trace_ids: HashMap::new(),
                },
            );
        id
    }

    /// Adds trace authority to an already scoped invocation context.
    pub fn grant_trace(&self, context_id: &str, grant: TraceContextGrant) -> bool {
        let mut contexts = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(context) = contexts.get_mut(context_id) else {
            return false;
        };
        // A workbench window may be retargeted from one Ora session to another. Revoke every
        // handle discovered under the previous grant before installing the replacement so an
        // already-open page cannot keep reading the previous session by opaque trace id.
        context.traces.clear();
        context.trace_ids.clear();
        context.trace = Some(grant);
        true
    }

    /// Revokes one page/session context when its host-owned invocation target closes.
    pub fn revoke(&self, context_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(context_id);
    }

    /// Revokes every context owned by a stopped consumer generation.
    pub fn revoke_generation(&self, plugin_id: &PluginId, generation: PluginGenerationKey) {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, context| {
                context.consumer_plugin_id != *plugin_id
                    || context.consumer_generation != generation
            });
    }

    pub(crate) fn with_context<Result>(
        &self,
        context_id: &str,
        consumer_plugin_id: &PluginId,
        consumer_generation: PluginGenerationKey,
        operation: impl FnOnce(&mut StoredContext) -> Result,
    ) -> Option<Result> {
        let mut contexts = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let context = contexts.get_mut(context_id)?;
        if context.consumer_plugin_id != *consumer_plugin_id
            || context.consumer_generation != consumer_generation
        {
            return None;
        }
        Some(operation(context))
    }
}
