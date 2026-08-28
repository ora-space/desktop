//! Host-side handler serving `ora/session/trace_*` to workbench plugin processes.
//!
//! A `TraceHost` is built per process launch: it pins the caller's plugin id and generation at
//! construction (never from request params), refuses callers whose manifest did not declare the
//! `session.trace` capability, and resolves the session through the surface binding table — a
//! page can only ever read the session its own surface was opened for.

use crate::trace_service::{TRACE_CHUNK_MAX_BYTES, TraceService};
use ora_domain::AgentRef;
use ora_plugin_manifest::HostCapability;
use ora_plugin_runtime::{BoxFuture, HostRequestError, HostRequestHandler};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;

/// Error code for a method the caller's manifest did not declare a capability for.
pub const CAPABILITY_DENIED_CODE: i64 = -32002;
/// Error code for a surface whose generation does not match the process that is serving it.
pub const SURFACE_MISMATCH_CODE: i64 = -32003;
/// Error code for a surface with no bound session, or a session the host cannot read.
pub const SESSION_NOT_BOUND_CODE: i64 = -32004;
/// Error code for a trace the read service could not produce (missing file, IO, …).
pub const TRACE_UNAVAILABLE_CODE: i64 = -32005;

pub const TRACE_STAT_METHOD: &str = "ora/session/trace_stat";
pub const TRACE_READ_METHOD: &str = "ora/session/trace_read";
pub const TRACE_LIST_METHOD: &str = "ora/session/trace_list";

/// Resolves one workbench surface instance to the session it was opened for.
///
/// The implementation is the surface-layer binding table (registered when a panel opens bound
/// to a chat); this trait keeps the host-side handler independent of the UI layer.
pub trait TraceSessionBindingLookup: Send + Sync + 'static {
    /// Returns the bound session, or `None` when this surface has none.
    fn binding(&self, instance_id: u64, generation: u64) -> Option<TraceSessionBinding>;
}

/// One surface-to-session binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceSessionBinding {
    /// The agent provider of the bound chat.
    pub agent_ref: AgentRef,
    /// The agent-side session id, resolved through the ACP session trace registry.
    pub agent_session_id: String,
}

/// Serves the trace methods for one workbench plugin process generation.
pub struct TraceHost {
    plugin_id: String,
    generation: u64,
    capabilities: HashSet<HostCapability>,
    service: Arc<TraceService>,
    bindings: Arc<dyn TraceSessionBindingLookup>,
}

impl TraceHost {
    /// Builds the handler for one launched process.
    pub fn new(
        plugin_id: String,
        generation: u64,
        capabilities: impl IntoIterator<Item = HostCapability>,
        service: Arc<TraceService>,
        bindings: Arc<dyn TraceSessionBindingLookup>,
    ) -> Self {
        Self {
            plugin_id,
            generation,
            capabilities: capabilities.into_iter().collect(),
            service,
            bindings,
        }
    }
}

impl HostRequestHandler for TraceHost {
    /// Dispatches one of the three trace methods; anything else is `method_not_found`.
    fn handle(
        &self,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value, HostRequestError>> {
        let plugin_id = self.plugin_id.clone();
        let generation = self.generation;
        let capabilities = self.capabilities.clone();
        let service = self.service.clone();
        let bindings = self.bindings.clone();
        let method = method.to_owned();
        Box::pin(async move {
            let trace_method = matches!(
                method.as_str(),
                TRACE_STAT_METHOD | TRACE_READ_METHOD | TRACE_LIST_METHOD
            );
            if !trace_method {
                return Err(HostRequestError::method_not_found(&method));
            }
            if !capabilities.contains(&HostCapability::SessionTrace) {
                return Err(HostRequestError::new(
                    CAPABILITY_DENIED_CODE,
                    "this plugin did not declare the `session.trace` host capability",
                )
                .with_data(json!({ "kind": "capability_denied" })));
            }

            let (instance_id, surface_generation) = parse_surface(&params)?;
            if surface_generation != generation {
                return Err(HostRequestError::new(
                    SURFACE_MISMATCH_CODE,
                    "the surface generation does not match the process serving it",
                )
                .with_data(json!({ "kind": "surface_mismatch" })));
            }
            tracing::info!(
                method = %method,
                %plugin_id,
                %instance_id,
                "serving a trace host request",
            );

            if method == TRACE_LIST_METHOD {
                let agent = parse_agent_filter(&params)?;
                let entries = service.list(agent.as_ref());
                return Ok(json!({
                    "entries": entries
                        .iter()
                        .map(|entry| json!({
                            "agent": entry.agent,
                            "sessionId": entry.session_id,
                            "name": entry.name,
                            "mtimeMs": entry.mtime_ms,
                            "sizeBytes": entry.size_bytes,
                        }))
                        .collect::<Vec<_>>(),
                }));
            }

            // Two read paths: the bound session (surface → binding), or an explicitly named
            // session from the browse listing (validated for membership so a page can only read
            // what the listing can show).
            let (agent, session_id) = match (
                params.get("sessionId").and_then(Value::as_str),
                params.get("agent").and_then(Value::as_str),
            ) {
                (Some(session_id), Some(agent)) => {
                    let Ok(agent_ref) = AgentRef::parse(agent) else {
                        return Err(HostRequestError::new(
                            -32602,
                            "the `agent` filter is not a valid agent id",
                        )
                        .with_data(json!({ "kind": "invalid_params" })));
                    };
                    if !service.has_session(&agent_ref, session_id) {
                        return Err(HostRequestError::new(
                            SESSION_NOT_BOUND_CODE,
                            "the named session is not in the trace listing",
                        )
                        .with_data(json!({ "kind": "session_not_bound" })));
                    }
                    (agent_ref, session_id.to_owned())
                }
                (None, None) => {
                    let Some(binding) = bindings.binding(instance_id, surface_generation) else {
                        return Err(HostRequestError::new(
                            SESSION_NOT_BOUND_CODE,
                            "this surface is not bound to a session",
                        )
                        .with_data(json!({ "kind": "session_not_bound" })));
                    };
                    (binding.agent_ref, binding.agent_session_id)
                }
                _ => {
                    return Err(HostRequestError::new(
                        -32602,
                        "name either both `agent` and `sessionId` or neither",
                    )
                    .with_data(json!({ "kind": "invalid_params" })));
                }
            };
            tracing::info!(
                method = %method,
                %plugin_id,
                session_id = %session_id,
                "serving a bound trace host request",
            );

            if method == TRACE_STAT_METHOD {
                let Some(stat) = service.stat(&agent, &session_id) else {
                    return Err(HostRequestError::new(
                        TRACE_UNAVAILABLE_CODE,
                        "the agent has no trace declaration",
                    )
                    .with_data(json!({ "kind": "trace_unavailable" })));
                };
                return Ok(json!({
                    "format": stat.format,
                    "exists": stat.exists,
                    "sizeBytes": stat.size_bytes,
                    "mtimeMs": stat.mtime_ms,
                }));
            }

            // trace_read
            let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
            let max_bytes = params
                .get("maxBytes")
                .and_then(Value::as_u64)
                .map(|value| value.min(TRACE_CHUNK_MAX_BYTES as u64) as usize)
                .unwrap_or(TRACE_CHUNK_MAX_BYTES);
            let chunk = match params.get("childSessionId").and_then(Value::as_str) {
                Some(child_session_id) => {
                    service.read_child(&agent, &session_id, child_session_id, offset, max_bytes)
                }
                None => service.read(&agent, &session_id, offset, max_bytes),
            };
            let Some(chunk) = chunk else {
                return Err(HostRequestError::new(
                    TRACE_UNAVAILABLE_CODE,
                    "the trace could not be read",
                )
                .with_data(json!({ "kind": "trace_unavailable" })));
            };
            Ok(json!({
                "text": chunk.text,
                "nextOffset": chunk.next_offset,
                "done": chunk.done,
            }))
        })
    }
}

/// Extracts and validates the `surface` envelope every trace method requires.
fn parse_surface(params: &Value) -> Result<(u64, u64), HostRequestError> {
    let Some(surface) = params.get("surface") else {
        return Err(
            HostRequestError::new(-32602, "trace methods require a `surface` envelope")
                .with_data(json!({ "kind": "invalid_params" })),
        );
    };
    let Some(instance_id) = surface.get("instanceId").and_then(Value::as_u64) else {
        return Err(HostRequestError::new(
            -32602,
            "the surface envelope requires an integer `instanceId`",
        )
        .with_data(json!({ "kind": "invalid_params" })));
    };
    let Some(generation) = surface.get("generation").and_then(Value::as_u64) else {
        return Err(HostRequestError::new(
            -32602,
            "the surface envelope requires an integer `generation`",
        )
        .with_data(json!({ "kind": "invalid_params" })));
    };
    Ok((instance_id, generation))
}

/// Extracts the optional agent filter of a list request.
fn parse_agent_filter(params: &Value) -> Result<Option<AgentRef>, HostRequestError> {
    match params.get("agent").and_then(Value::as_str) {
        None => Ok(None),
        Some(value) => AgentRef::parse(value).map(Some).map_err(|_| {
            HostRequestError::new(-32602, "the `agent` filter is not a valid agent id")
                .with_data(json!({ "kind": "invalid_params" }))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_registry::TraceRegistry;
    use ora_plugin_manifest::PluginAgentTrace;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Extracts an expected successful result without using `expect` in tests.
    fn must<T, E: std::fmt::Debug>(result: Result<T, E>, label: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("expected {label} to succeed, got {error:?}"),
        }
    }

    /// A binding table over a plain map; missing entries mean "not bound".
    struct MapLookup {
        bindings: Mutex<HashMap<(u64, u64), TraceSessionBinding>>,
    }

    impl MapLookup {
        fn insert(&self, instance_id: u64, generation: u64, binding: TraceSessionBinding) {
            self.bindings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert((instance_id, generation), binding);
        }
    }

    impl TraceSessionBindingLookup for MapLookup {
        fn binding(&self, instance_id: u64, generation: u64) -> Option<TraceSessionBinding> {
            self.bindings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&(instance_id, generation))
                .cloned()
        }
    }

    /// Builds a tempdir-backed service with one written trace file.
    fn build_service(temp: &tempfile::TempDir) -> (Arc<TraceService>, AgentRef) {
        let registry = Arc::new(TraceRegistry::new(Vec::new()));
        let agent = must(AgentRef::parse("ora-space.test"), "agent ref");
        registry.register_plugin(
            agent.clone(),
            must(
                PluginAgentTrace::file(
                    "opencode",
                    "{data_dir}/opencode/trace/{agent_session_id}.ndjson",
                ),
                "file declaration",
            ),
        );
        let service = Arc::new(TraceService::new(
            registry,
            temp.path().join("home"),
            temp.path().join("data"),
        ));
        let trace_dir = temp.path().join("data/opencode/trace");
        std::fs::create_dir_all(&trace_dir).expect("trace dir");
        std::fs::write(
            trace_dir.join("ses_1.ndjson"),
            "{\"type\":\"session.start\",\"title\":\"t\"}\n",
        )
        .expect("trace file");
        (service, agent)
    }

    /// Builds a host with one bound session (surface 7, generation 1).
    fn host_with_bound_session() -> (Arc<TraceHost>, tempfile::TempDir) {
        let temp = tempdir().expect("tempdir");
        let (service, agent) = build_service(&temp);
        let lookup = Arc::new(MapLookup {
            bindings: Mutex::new(HashMap::new()),
        });
        lookup.insert(
            7,
            1,
            TraceSessionBinding {
                agent_ref: agent,
                agent_session_id: "ses_1".to_owned(),
            },
        );
        let host = Arc::new(TraceHost::new(
            "official.dashboard".to_owned(),
            1,
            [HostCapability::SessionTrace],
            service,
            lookup,
        ));
        (host, temp)
    }

    fn surface_params(instance_id: u64, generation: u64) -> Value {
        json!({ "surface": { "instanceId": instance_id, "generation": generation } })
    }

    /// The declared capability serves stat and read for the bound surface.
    #[tokio::test]
    async fn serves_stat_and_read_for_a_bound_surface() {
        let (host, _temp) = host_with_bound_session();

        let stat = host
            .handle(TRACE_STAT_METHOD, surface_params(7, 1))
            .await
            .expect("stat succeeds");
        assert_eq!(stat["exists"], json!(true));
        assert_eq!(stat["format"], json!("opencode"));

        let read = host
            .handle(TRACE_READ_METHOD, surface_params(7, 1))
            .await
            .expect("read succeeds");
        assert_eq!(
            read["text"],
            json!("{\"type\":\"session.start\",\"title\":\"t\"}\n")
        );
        assert_eq!(read["done"], json!(true));
    }

    /// A capability-less host refuses every trace method before looking at params.
    #[tokio::test]
    async fn refuses_without_the_capability() {
        let temp = tempdir().expect("tempdir");
        let (service, _agent) = build_service(&temp);
        let bare = Arc::new(TraceHost::new(
            "official.dashboard".to_owned(),
            1,
            std::iter::empty(),
            service,
            Arc::new(MapLookup {
                bindings: Mutex::new(HashMap::new()),
            }),
        ));

        let error = bare
            .handle(TRACE_STAT_METHOD, surface_params(7, 1))
            .await
            .expect_err("capability check refuses first");
        assert_eq!(error.code(), CAPABILITY_DENIED_CODE);
        assert_eq!(error.data()["kind"], json!("capability_denied"));
    }

    /// A surface from a different generation is refused even with the capability.
    #[tokio::test]
    async fn refuses_a_surface_from_another_generation() {
        let (host, _temp) = host_with_bound_session();

        let error = host
            .handle(TRACE_STAT_METHOD, surface_params(7, 2))
            .await
            .expect_err("stale surface generation is refused");
        assert_eq!(error.code(), SURFACE_MISMATCH_CODE);
    }

    /// A surface with no binding is refused; a bound surface with an unknown session id is too.
    #[tokio::test]
    async fn refuses_unbound_surfaces() {
        let (host, _temp) = host_with_bound_session();

        let error = host
            .handle(TRACE_STAT_METHOD, surface_params(9, 1))
            .await
            .expect_err("unbound surface is refused");
        assert_eq!(error.code(), SESSION_NOT_BOUND_CODE);
    }

    /// The list method serves every declared agent without a session binding.
    #[tokio::test]
    async fn list_serves_declared_agents() {
        let (host, _temp) = host_with_bound_session();

        let list = host
            .handle(TRACE_LIST_METHOD, surface_params(7, 1))
            .await
            .expect("list succeeds");
        let entries = list["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["sessionId"], json!("ses_1"));
        assert_eq!(entries[0]["name"], json!("t"));
    }

    /// The production composition: storage and trace methods served side by side, unknown
    /// methods falling through the whole chain.
    #[tokio::test]
    async fn composite_serves_storage_and_trace_side_by_side() {
        struct StorageStub;
        impl HostRequestHandler for StorageStub {
            fn handle(
                &self,
                method: &str,
                _params: Value,
            ) -> BoxFuture<'static, Result<Value, HostRequestError>> {
                let method = method.to_owned();
                Box::pin(async move {
                    if method == "ora/storage/list" {
                        Ok(json!({ "stub": true }))
                    } else {
                        Err(HostRequestError::method_not_found(&method))
                    }
                })
            }
        }

        let (host, _temp) = host_with_bound_session();
        let composite = ora_plugin_runtime::CompositeHostRequests::new(vec![
            Arc::new(StorageStub),
            host.clone(),
        ]);

        let storage = composite
            .handle("ora/storage/list", Value::Null)
            .await
            .expect("storage is served by the first handler");
        assert_eq!(storage, json!({ "stub": true }));

        let stat = composite
            .handle(TRACE_STAT_METHOD, surface_params(7, 1))
            .await
            .expect("trace falls through to the trace host");
        assert_eq!(stat["exists"], json!(true));

        let error = composite
            .handle("nobody/ping", Value::Null)
            .await
            .expect_err("unknown method exhausts the chain");
        assert_eq!(error.code(), ora_plugin_runtime::METHOD_NOT_FOUND_CODE);
    }

    /// A session named through the browse listing is readable; an unlisted id is refused.
    #[tokio::test]
    async fn reads_named_sessions_from_the_listing_only() {
        let (host, temp) = host_with_bound_session();
        let trace_dir = temp.path().join("data/opencode/trace");
        std::fs::write(
            trace_dir.join("ses_2.ndjson"),
            "{\"type\":\"session.start\",\"title\":\"two\"}\n",
        )
        .expect("second trace");

        let read = host
            .handle(
                TRACE_READ_METHOD,
                json!({
                    "surface": { "instanceId": 7, "generation": 1 },
                    "agent": "ora-space.test",
                    "sessionId": "ses_2",
                }),
            )
            .await
            .expect("listed session reads");
        assert_eq!(
            read["text"],
            json!("{\"type\":\"session.start\",\"title\":\"two\"}\n")
        );

        let missing = host
            .handle(
                TRACE_READ_METHOD,
                json!({
                    "surface": { "instanceId": 7, "generation": 1 },
                    "agent": "ora-space.test",
                    "sessionId": "ses_missing",
                }),
            )
            .await
            .expect_err("unlisted session is refused");
        assert_eq!(missing.code(), SESSION_NOT_BOUND_CODE);

        // Half-named requests are invalid params.
        let half = host
            .handle(
                TRACE_READ_METHOD,
                json!({
                    "surface": { "instanceId": 7, "generation": 1 },
                    "sessionId": "ses_2",
                }),
            )
            .await
            .expect_err("half-named request is refused");
        assert_eq!(half.code(), -32602);
    }

    /// Unknown methods fall through so the composite can keep delegating.
    #[tokio::test]
    async fn unknown_methods_are_method_not_found() {
        let (host, _temp) = host_with_bound_session();

        let error = host
            .handle("ora/storage/list", Value::Null)
            .await
            .expect_err("storage is not a trace method");
        assert_eq!(error.code(), ora_plugin_runtime::METHOD_NOT_FOUND_CODE);
    }
}
