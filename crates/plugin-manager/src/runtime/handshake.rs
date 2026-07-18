use std::time::Duration;

use ora_plugin_protocol::{
    ActivateParams, ActivateResult, ActivationReason, DeclaredAgent, HostRequestId,
    HostResolvedAbsolutePath, InitializeParams, InitializePaths, InitializePlugin,
    InitializeResult, JsonRpcEnvelope, JsonRpcResponse, METHOD_ACTIVATE, METHOD_INITIALIZE,
    PLUGIN_API_VERSION_V1, PluginKind, WIRE_VERSION_V1, encode_json_rpc_request,
};
use ora_process::ProcessTreeController;

use crate::{
    ActivationFailure, GenerationProcessEvent, GenerationTransport, HandshakeFailure, PluginError,
    PluginManagerConfig, PluginRuntimeAssets, ReaderEvent, RuntimeAdmissionProvider,
    SessionControlKind, ValidatedLaunchDescriptor, WriterCommandOwner, WriterCompletion,
    WriterLane,
};

/// Successful initialize/activate proof retained by the running generation actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeProof {
    pub session_id: String,
    pub next_request_sequence: u64,
    pub declared_agents: Vec<DeclaredAgent>,
}

/// Performs both lifecycle round trips and the post-activate admission barrier.
pub async fn perform_handshake<Controller, Admission>(
    transport: &mut GenerationTransport<Controller>,
    admission: &Admission,
    descriptor: &ValidatedLaunchDescriptor,
    config: &PluginManagerConfig,
    assets: &PluginRuntimeAssets,
    reason: ActivationReason,
) -> Result<HandshakeProof, PluginError>
where
    Controller: ProcessTreeController,
    Admission: RuntimeAdmissionProvider,
{
    if descriptor.kind != PluginKind::Agent {
        return Err(PluginError::HandshakeFailed {
            plugin_id: descriptor.plugin_id.clone(),
            reason: HandshakeFailure::IdentityMismatch,
        });
    }
    let session_id = uuid::Uuid::new_v4().to_string();
    let declared_agents = descriptor
        .declared_agents
        .iter()
        .cloned()
        .map(|id| DeclaredAgent {
            id,
            contract_version: ora_plugin_protocol::AGENT_CONTRACT_VERSION_V1,
        })
        .collect::<Vec<_>>();
    let initialize = InitializeParams {
        wire_version: WIRE_VERSION_V1,
        host_version: config.host_version.clone(),
        runtime_version: assets.runtime_version.clone(),
        session_id: session_id.clone(),
        plugin: InitializePlugin {
            id: descriptor.plugin_id.clone(),
            version: descriptor.plugin_version.clone(),
            kind: descriptor.kind,
            plugin_api: PLUGIN_API_VERSION_V1,
            content_owner: descriptor.content_owner.clone(),
        },
        paths: InitializePaths {
            extension_path: protocol_path(&descriptor.extension_path).map_err(|_| {
                PluginError::HandshakeFailed {
                    plugin_id: descriptor.plugin_id.clone(),
                    reason: HandshakeFailure::IdentityMismatch,
                }
            })?,
            entry_path: protocol_path(&descriptor.entry_path).map_err(|_| {
                PluginError::HandshakeFailed {
                    plugin_id: descriptor.plugin_id.clone(),
                    reason: HandshakeFailure::IdentityMismatch,
                }
            })?,
            storage_path: protocol_path(&descriptor.storage_path).map_err(|_| {
                PluginError::HandshakeFailed {
                    plugin_id: descriptor.plugin_id.clone(),
                    reason: HandshakeFailure::IdentityMismatch,
                }
            })?,
        },
        declared_agents: declared_agents.clone(),
        limits: config.limits.runtime.clone(),
    };
    initialize
        .validate()
        .map_err(|_| PluginError::HandshakeFailed {
            plugin_id: descriptor.plugin_id.clone(),
            reason: HandshakeFailure::IdentityMismatch,
        })?;

    let initialize_id = HostRequestId::from_sequence(1).map_err(|_| PluginError::Internal {
        message: "static initialize request id is invalid".to_owned(),
    })?;
    let initialize_response = lifecycle_round_trip(
        transport,
        &initialize_id,
        METHOD_INITIALIZE,
        &initialize,
        SessionControlKind::Initialize,
        config.deadlines.initialize,
    )
    .await
    .map_err(|failure| PluginError::HandshakeFailed {
        plugin_id: descriptor.plugin_id.clone(),
        reason: failure.as_handshake_failure(),
    })?;
    let initialize_result: InitializeResult = response_result(initialize_response)
        .and_then(|value| {
            serde_json::from_value(value).map_err(|_| LifecycleRoundTripFailure::InvalidResult)
        })
        .map_err(|failure| PluginError::HandshakeFailed {
            plugin_id: descriptor.plugin_id.clone(),
            reason: failure.as_handshake_failure(),
        })?;
    if initialize_result.wire_version != WIRE_VERSION_V1
        || initialize_result.runtime_version != assets.runtime_version
        || initialize_result.session_id != session_id
        || initialize_result.plugin.id != descriptor.plugin_id
        || initialize_result.plugin.version != descriptor.plugin_version
    {
        return Err(PluginError::HandshakeFailed {
            plugin_id: descriptor.plugin_id.clone(),
            reason: HandshakeFailure::IdentityMismatch,
        });
    }

    let activate_id = HostRequestId::from_sequence(2).map_err(|_| PluginError::Internal {
        message: "static activate request id is invalid".to_owned(),
    })?;
    let activate_response = lifecycle_round_trip(
        transport,
        &activate_id,
        METHOD_ACTIVATE,
        &ActivateParams { reason },
        SessionControlKind::Activate,
        config.deadlines.activate,
    )
    .await
    .map_err(|failure| PluginError::ActivationFailed {
        plugin_id: descriptor.plugin_id.clone(),
        reason: failure.as_activation_failure(),
    })?;
    let activate_result: ActivateResult = response_result(activate_response)
        .and_then(|value| {
            serde_json::from_value(value).map_err(|_| LifecycleRoundTripFailure::InvalidResult)
        })
        .map_err(|failure| PluginError::ActivationFailed {
            plugin_id: descriptor.plugin_id.clone(),
            reason: failure.as_activation_failure(),
        })?;
    activate_result
        .validate_declared_providers(&declared_agents)
        .map_err(|_| PluginError::ActivationFailed {
            plugin_id: descriptor.plugin_id.clone(),
            reason: ActivationFailure::ProviderMismatch,
        })?;
    admission
        .recheck_after_activate(descriptor)
        .await
        .map_err(|_| PluginError::ActivationFailed {
            plugin_id: descriptor.plugin_id.clone(),
            reason: ActivationFailure::AdmissionChanged,
        })?;

    Ok(HandshakeProof {
        session_id,
        next_request_sequence: 3,
        declared_agents,
    })
}

/// Sends one lifecycle request and preserves response-before-writer-ack causal ordering.
async fn lifecycle_round_trip<Controller, Params>(
    transport: &mut GenerationTransport<Controller>,
    id: &HostRequestId,
    method: &str,
    params: &Params,
    control: SessionControlKind,
    deadline: Duration,
) -> Result<JsonRpcResponse, LifecycleRoundTripFailure>
where
    Controller: ProcessTreeController,
    Params: serde::Serialize,
{
    let payload = encode_json_rpc_request(id, method, params)
        .map_err(|_| LifecycleRoundTripFailure::LocalEncoding)?;
    let owner = WriterCommandOwner::SessionControl(control);
    transport
        .writer
        .enqueue(
            transport.generation,
            owner.clone(),
            &payload,
            WriterLane::SessionControl,
            deadline,
        )
        .await
        .map_err(|_| LifecycleRoundTripFailure::WriterFailure)?;

    let timeout = tokio::time::sleep(deadline);
    tokio::pin!(timeout);
    let mut frame_written = false;
    let mut deferred_response = None;
    loop {
        tokio::select! {
            _ = &mut timeout => return Err(LifecycleRoundTripFailure::DeadlineExceeded),
            writer = transport.writer_events.recv() => {
                match writer {
                    Some(WriterCompletion::FrameWritten { generation, owner: actual })
                        if generation == transport.generation && actual == owner => {
                            frame_written = true;
                            if let Some(response) = deferred_response.take() {
                                return Ok(response);
                            }
                        }
                    Some(WriterCompletion::WriteFailed { generation, owner: actual, .. })
                        if generation == transport.generation && actual == owner => {
                            return Err(LifecycleRoundTripFailure::WriterFailure);
                        }
                    Some(_) => return Err(LifecycleRoundTripFailure::Protocol),
                    None => return Err(LifecycleRoundTripFailure::WriterFailure),
                }
            }
            reader = transport.reader_events.recv() => {
                match reader {
                    Some(ReaderEvent::Envelope(JsonRpcEnvelope::Response(response)))
                        if response_id(&response) == id => {
                            if deferred_response.is_some() {
                                return Err(LifecycleRoundTripFailure::Protocol);
                            }
                            if frame_written {
                                return Ok(response);
                            }
                            deferred_response = Some(response);
                        }
                    Some(ReaderEvent::Envelope(_)) => return Err(LifecycleRoundTripFailure::Protocol),
                    Some(ReaderEvent::BoundaryEof) | None => {
                        return Err(LifecycleRoundTripFailure::ProcessExited);
                    }
                    Some(ReaderEvent::Failure(_)) => return Err(LifecycleRoundTripFailure::Protocol),
                }
            }
            process = transport.process_events.recv() => {
                match process {
                    Some(GenerationProcessEvent::StderrDrained(_)) => {}
                    Some(GenerationProcessEvent::DirectExit(_))
                    | Some(GenerationProcessEvent::TreeEmpty(_))
                    | None => return Err(LifecycleRoundTripFailure::ProcessExited),
                }
            }
        }
    }
}

fn response_id(response: &JsonRpcResponse) -> &HostRequestId {
    match response {
        JsonRpcResponse::Success { id, .. } | JsonRpcResponse::Error { id, .. } => id,
    }
}

fn response_result(
    response: JsonRpcResponse,
) -> Result<serde_json::Value, LifecycleRoundTripFailure> {
    match response {
        JsonRpcResponse::Success { result, .. } => Ok(result),
        JsonRpcResponse::Error { .. } => Err(LifecycleRoundTripFailure::RemoteError),
    }
}

fn protocol_path(path: &std::path::Path) -> Result<HostResolvedAbsolutePath, ()> {
    path.to_str()
        .ok_or(())
        .and_then(|value| HostResolvedAbsolutePath::parse(value).map_err(|_| ()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleRoundTripFailure {
    DeadlineExceeded,
    LocalEncoding,
    WriterFailure,
    Protocol,
    ProcessExited,
    RemoteError,
    InvalidResult,
}

impl LifecycleRoundTripFailure {
    const fn as_handshake_failure(self) -> HandshakeFailure {
        match self {
            Self::DeadlineExceeded => HandshakeFailure::DeadlineExceeded,
            Self::ProcessExited => HandshakeFailure::ProcessExited,
            Self::LocalEncoding
            | Self::WriterFailure
            | Self::Protocol
            | Self::RemoteError
            | Self::InvalidResult => HandshakeFailure::FirstFrameMismatch,
        }
    }

    const fn as_activation_failure(self) -> ActivationFailure {
        match self {
            Self::DeadlineExceeded => ActivationFailure::DeadlineExceeded,
            Self::ProcessExited => ActivationFailure::ProcessExited,
            Self::RemoteError => ActivationFailure::RemoteError,
            Self::LocalEncoding | Self::WriterFailure | Self::Protocol | Self::InvalidResult => {
                ActivationFailure::InvalidResult
            }
        }
    }
}
