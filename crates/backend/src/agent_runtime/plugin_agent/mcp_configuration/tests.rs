use super::{
    ConfigureWorkspaceError, ConfigureWorkspaceRuntime, DesiredResolvedMcp, ExpectedManagedMcp,
    ExpectedReceiptCoverage, McpConfigurationDisableReason, NegotiatedMcpConfiguration,
    ResolvedMcpTransport, SnapshotRequestError, agent_capability_revision, configure_workspace,
    negotiate_mcp_configuration, parse_mcp_configuration_receipt,
    prepare_mcp_configuration_snapshot, snapshot_request_json, validate_mcp_configuration_receipt,
};
use ora_effect::{AgentTargetId, ConditionImpact, Digest, Generation};
use ora_logging::{with_recorded_trace_logging, with_trace_logging};
use ora_plugin_runtime::{
    McpConfigurationCapability, McpConfigurationProtocolVersion, McpCoordinationMode,
    McpTransportKind, McpTransportSet, PluginRegistration, PluginRuntimeError,
    parse_mcp_configuration,
};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::layer;
use url::Url;

/// Loads one shared protocol fixture used by both Rust and TypeScript tests.
fn fixture(kind: &str, name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/plugin-sdk/tests/fixtures/mcp-configuration")
        .join(kind)
        .join(name);
    serde_json::from_slice(
        &std::fs::read(&path)
            .unwrap_or_else(|error| panic!("read MCP fixture {}: {error}", path.display())),
    )
    .expect("MCP fixture is JSON")
}

fn registration_from_fixture(name: &str) -> PluginRegistration {
    let params = fixture("registration", name);
    let methods = params["methods"]
        .as_array()
        .expect("methods")
        .iter()
        .map(|value| value.as_str().expect("method").to_string())
        .collect::<HashSet<_>>();
    let emits = params["emits"]
        .as_array()
        .expect("emits")
        .iter()
        .map(|value| value.as_str().expect("emit").to_string())
        .collect::<HashSet<_>>();
    PluginRegistration {
        methods,
        emits,
        effect_surfaces: Vec::new(),
        mcp_configuration: parse_mcp_configuration(Some(&params)),
    }
}

fn http_capability() -> McpConfigurationCapability {
    McpConfigurationCapability::new(
        McpConfigurationProtocolVersion::V1,
        McpTransportSet::new([McpTransportKind::Http]).expect("http"),
        McpCoordinationMode::WaitForIdleAndRestart,
    )
}

fn tavily_http() -> DesiredResolvedMcp {
    DesiredResolvedMcp {
        canonical_identity: "official/ora-space.tavily-search".to_string(),
        managed_identity: "mcp-tavily".to_string(),
        package_version: "0.1.0".to_string(),
        source_revision_id: "rev-tavily-1".to_string(),
        transport: ResolvedMcpTransport::Http {
            url: Url::parse("https://mcp.tavily.com/mcp").expect("url"),
            headers: BTreeMap::from([(
                "Authorization".to_string(),
                "Bearer tavily-secret-key".to_string(),
            )]),
        },
    }
}

fn stdio_mcp() -> DesiredResolvedMcp {
    DesiredResolvedMcp {
        canonical_identity: "official/ora-space.stdio-mcp".to_string(),
        managed_identity: "mcp-stdio".to_string(),
        package_version: "1.0.0".to_string(),
        source_revision_id: "rev-stdio-1".to_string(),
        transport: ResolvedMcpTransport::Stdio {
            executable: PathBuf::from("/pkg/bin/mcp"),
            args: vec!["--token".to_string(), "stdio-secret".to_string()],
            env: BTreeMap::from([("API_KEY".to_string(), "env-secret".to_string())]),
            working_directory: PathBuf::from("/workspace"),
        },
    }
}

fn expected_tavily() -> ExpectedReceiptCoverage {
    ExpectedReceiptCoverage {
        generation: Generation::new(4),
        desired: vec![ExpectedManagedMcp {
            managed_identity: "mcp-tavily".to_string(),
            source_revision_id: "rev-tavily-1".to_string(),
        }],
    }
}

fn prepared_tavily_http() -> super::PreparedMcpConfiguration {
    prepare_mcp_configuration_snapshot(
        &http_capability(),
        "op-7",
        AgentTargetId::new("target-1"),
        PathBuf::from("/workspace"),
        Generation::new(4),
        vec![tavily_http()],
    )
    .expect("prepare")
}

/// Older plugins omit the capability and remain conversation-capable.
#[test]
fn omitted_capability_is_unsupported_without_failing_the_agent_contract() {
    let registration = registration_from_fixture("omitted.json");
    assert_eq!(
        negotiate_mcp_configuration(&registration),
        NegotiatedMcpConfiguration::Unsupported
    );
}

/// Unknown protocol versions disable MCP materialization only.
#[test]
fn unknown_protocol_version_disables_mcp_and_keeps_the_agent_contract() {
    let registration = registration_from_fixture("unknown-protocol-version.json");
    assert_eq!(
        negotiate_mcp_configuration(&registration),
        NegotiatedMcpConfiguration::Disabled {
            reason: McpConfigurationDisableReason::Invalid(
                ora_plugin_runtime::McpConfigurationCapabilityIssue::ProtocolVersionUnsupported {
                    actual: 2
                }
            )
        }
    );
    assert_eq!(
        negotiate_mcp_configuration(&registration).disable_error_code(),
        Some("mcp_capability_version_unsupported")
    );
}

/// Duplicate or unknown capability fields disable MCP without failing conversation.
#[test]
fn malformed_and_duplicate_capabilities_do_not_invalidate_the_agent_contract() {
    assert_eq!(
        negotiate_mcp_configuration(&registration_from_fixture("malformed.json")),
        NegotiatedMcpConfiguration::Disabled {
            reason: McpConfigurationDisableReason::Invalid(
                ora_plugin_runtime::McpConfigurationCapabilityIssue::UnknownField(
                    "nativeSchema".to_string()
                )
            )
        }
    );
    assert_eq!(
        negotiate_mcp_configuration(&registration_from_fixture("duplicate-transports.json")),
        NegotiatedMcpConfiguration::Disabled {
            reason: McpConfigurationDisableReason::Invalid(
                ora_plugin_runtime::McpConfigurationCapabilityIssue::DuplicateTransport(
                    "http".to_string()
                )
            )
        }
    );
}

/// Capability and handler must be present together; either side alone is disabled.
#[test]
fn capability_and_handler_must_be_registered_together() {
    assert_eq!(
        negotiate_mcp_configuration(&registration_from_fixture(
            "capability-without-handler.json"
        )),
        NegotiatedMcpConfiguration::Disabled {
            reason: McpConfigurationDisableReason::CapabilityWithoutHandler
        }
    );
    assert_eq!(
        negotiate_mcp_configuration(&registration_from_fixture(
            "capability-without-handler.json"
        ))
        .disable_error_code(),
        Some("mcp_capability_invalid")
    );
    assert_eq!(
        negotiate_mcp_configuration(&registration_from_fixture(
            "handler-without-capability.json"
        )),
        NegotiatedMcpConfiguration::Disabled {
            reason: McpConfigurationDisableReason::HandlerWithoutCapability
        }
    );
    assert_eq!(
        negotiate_mcp_configuration(&registration_from_fixture("valid-http-v1.json")).capability(),
        Some(&http_capability())
    );
}

/// Host excludes unsupported transports and records NonBlocking target-specific issues.
#[test]
fn unsupported_transports_are_excluded_as_non_blocking_conditions() {
    let prepared = prepare_mcp_configuration_snapshot(
        &http_capability(),
        "op-7",
        AgentTargetId::new("target-1"),
        PathBuf::from("/workspace"),
        Generation::new(4),
        vec![tavily_http(), stdio_mcp()],
    )
    .expect("prepare");
    assert_eq!(prepared.resolved_mcps, vec![tavily_http()]);
    assert_eq!(
        prepared.unsupported,
        vec![super::UnsupportedMcp {
            managed_identity: "mcp-stdio".to_string(),
            transport: McpTransportKind::Stdio,
            impact: ConditionImpact::NonBlocking,
            code: "mcp_unsupported_by_agent",
        }]
    );
}

/// Snapshot construction refuses a relative Workspace root so plugins never receive host-relative paths.
#[test]
fn snapshot_rejects_a_relative_workspace_root() {
    assert_eq!(
        prepare_mcp_configuration_snapshot(
            &http_capability(),
            "op-7",
            AgentTargetId::new("target-1"),
            PathBuf::from("relative-workspace"),
            Generation::new(4),
            vec![tavily_http()],
        ),
        Err(SnapshotRequestError::WorkspaceRootNotAbsolute)
    );
}

/// Snapshot JSON carries the allowed identity fields and never host-private paths.
#[test]
fn snapshot_request_contains_only_allowed_fields() {
    let json = snapshot_request_json(&prepared_tavily_http());
    let mut expected = fixture("requests", "full-snapshot.json");
    // Workspace root encoding is OS-specific; the remaining closed field set must match exactly.
    expected["workspaceRoot"] = json["workspaceRoot"].clone();
    assert_eq!(json, expected);
    let serialized = json.to_string();
    assert!(!serialized.contains("manifest"));
    assert!(!serialized.contains("configurationStore"));
    assert!(!serialized.contains("store.json"));
    assert!(!serialized.contains("ora.sqlite"));
    assert!(!serialized.contains("plugins/data"));
}

/// Debug formatting redacts header and environment values used by Tavily and stdio MCPs.
#[test]
fn snapshot_debug_omits_header_and_environment_values() {
    let prepared = prepare_mcp_configuration_snapshot(
        &McpConfigurationCapability::new(
            McpConfigurationProtocolVersion::V1,
            McpTransportSet::new([McpTransportKind::Http, McpTransportKind::Stdio]).expect("set"),
            McpCoordinationMode::WaitForIdleAndRestart,
        ),
        "op-7",
        AgentTargetId::new("target-1"),
        PathBuf::from("/workspace"),
        Generation::new(4),
        vec![tavily_http(), stdio_mcp()],
    )
    .expect("prepare");
    let debug = format!("{prepared:?}");
    assert!(!debug.contains("tavily-secret-key"));
    assert!(!debug.contains("Bearer "));
    assert!(!debug.contains("env-secret"));
    assert!(!debug.contains("stdio-secret"));
}

fn assert_receipt(name: &str, expected: Result<(), super::ReceiptValidationError>) {
    let parsed = parse_mcp_configuration_receipt(fixture("receipts", name));
    let result =
        parsed.and_then(|receipt| validate_mcp_configuration_receipt(&receipt, &expected_tavily()));
    assert_eq!(result.map(|_| ()), expected);
}

/// Valid receipts cover Desired identities exactly.
#[test]
fn valid_receipt_covers_the_supported_desired_set() {
    assert_receipt("valid.json", Ok(()));
}

/// Missing, duplicate, extra, mismatched, and illegal receipts are all rejected.
#[test]
fn invalid_receipts_are_rejected() {
    use super::ReceiptValidationError::*;
    assert_receipt("missing.json", Err(MissingManagedIdentity));
    assert_receipt("duplicate.json", Err(DuplicateManagedIdentity));
    assert_receipt("extra.json", Err(ExtraManagedIdentity));
    assert_receipt("generation-mismatch.json", Err(GenerationMismatch));
    assert_receipt("source-revision-mismatch.json", Err(SourceRevisionMismatch));
    assert_eq!(
        parse_mcp_configuration_receipt(fixture("receipts", "illegal-fingerprint.json")),
        Err(IllegalFingerprint)
    );
    assert_eq!(
        parse_mcp_configuration_receipt(fixture("receipts", "locator-escape.json")),
        Err(LocatorOutOfBounds)
    );
    assert_eq!(
        parse_mcp_configuration_receipt(Value::String("not-an-object".to_string())),
        Err(NotAnObject)
    );
}

/// Plugin version and capability digest are both bound into the revision.
#[test]
fn capability_revision_changes_when_version_or_digest_changes() {
    let omitted = registration_from_fixture("omitted.json").mcp_configuration;
    let http = registration_from_fixture("valid-http-v1.json").mcp_configuration;
    let first = agent_capability_revision("0.2.2", &omitted);
    let upgraded_version = agent_capability_revision("0.3.0", &omitted);
    let added_capability = agent_capability_revision("0.2.2", &http);
    assert_ne!(first, upgraded_version);
    assert_ne!(first, added_capability);
    assert_eq!(
        first,
        ora_effect::AgentCapabilityRevision::bind(
            "0.2.2",
            &Digest::sha256(&omitted.canonical_declaration_bytes())
        )
    );
}

struct ScriptedRuntime {
    result: Result<Value, PluginRuntimeError>,
}

impl ConfigureWorkspaceRuntime for ScriptedRuntime {
    async fn invoke(&self, method: &str, _params: Value) -> Result<Value, PluginRuntimeError> {
        assert_eq!(method, ora_plugin_runtime::CONFIGURE_WORKSPACE_METHOD);
        self.result.clone()
    }
}

/// Timeout errors name the method and never echo snapshot secrets.
#[test]
fn configure_timeout_does_not_include_payload_secrets() {
    let error = with_trace_logging(|| {
        futures_executor_block_on(configure_workspace(
            &ScriptedRuntime {
                result: Err(PluginRuntimeError::CallTimeout),
            },
            &prepared_tavily_http(),
            &expected_tavily(),
        ))
        .expect_err("timeout")
    });
    assert_eq!(error, ConfigureWorkspaceError::TimedOut);
    let rendered = error.to_string();
    assert!(!rendered.contains("tavily-secret-key"));
    assert!(!rendered.contains("Authorization"));
}

/// Remote plugin errors are sanitized before they become Host diagnostics.
#[test]
fn configure_remote_errors_redact_authorization_values() {
    let error = with_trace_logging(|| {
        futures_executor_block_on(configure_workspace(
            &ScriptedRuntime {
                result: Err(PluginRuntimeError::Remote {
                    code: -32603,
                    message: "Authorization: Bearer tavily-secret-key".to_string(),
                }),
            },
            &prepared_tavily_http(),
            &expected_tavily(),
        ))
        .expect_err("remote")
    });
    let rendered = error.to_string();
    assert!(!rendered.contains("tavily-secret-key"));
}

/// Configure traces record identity fields and never the JSON-RPC body.
#[test]
fn configure_trace_omits_header_values() {
    let capture = Capture::default();
    let receipt = with_recorded_trace_logging(
        layer().with_writer(capture.clone()).with_ansi(false),
        || {
            futures_executor_block_on(configure_workspace(
                &ScriptedRuntime {
                    result: Ok(fixture("receipts", "valid.json")),
                },
                &prepared_tavily_http(),
                &expected_tavily(),
            ))
        },
    )
    .expect("configure");
    assert_eq!(
        receipt,
        parse_mcp_configuration_receipt(fixture("receipts", "valid.json")).expect("valid receipt")
    );
    let logs = capture.text();
    assert!(!logs.contains("tavily-secret-key"));
    assert!(!logs.contains("Bearer "));
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("capture").clone()).expect("utf8")
    }
}

struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("capture").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = CaptureWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter(self.0.clone())
    }
}

/// Blocks on one configure future without adding a runtime to this module's production path.
fn futures_executor_block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}
