//! Exact translation of Cloud's self-contained plugin plans and Node's bounded results.
use crate::*;
use ora_controller_proto::v1 as proto;

/// Rebuilds a recorded command; the business operation stays in Cloud and the attempt ID goes
/// on the Node wire, exactly as for clone retries.
pub(super) fn command(
    record: &proto::ExecutionRecord,
    node: &NodeId,
) -> Result<PluginCommand, Error> {
    if record.node_id != node.as_str() {
        return Err(Error::Conflict);
    }
    from_input(
        record.input.as_ref().ok_or(Error::Conflict)?,
        node,
        OperationId::new(record.node_operation_id.clone()),
        ExecutionId::new(record.execution_id.clone()),
    )
}

/// Validates without rewriting any field from the operation snapshot.
pub(in crate::cloud) fn from_input(
    input: &proto::ExecutionInput,
    node: &NodeId,
    operation: OperationId,
    execution: ExecutionId,
) -> Result<PluginCommand, Error> {
    let command = match input.spec.as_ref() {
        Some(proto::execution_input::Spec::InstallPlugins(spec)) => {
            PluginCommand::Install(InstallPluginsMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: operation,
                execution_id: execution,
                payload: InstallPlugins {
                    spec: InstallPluginsSpec {
                        node_id: node.clone(),
                        plugins: spec
                            .plugins
                            .iter()
                            .map(|p| {
                                let release = match (&p.universal, p.targets.is_empty()) {
                                    (Some(source), true) => PluginRelease::Universal {
                                        download: download(source),
                                    },
                                    (None, false) => PluginRelease::Targets {
                                        targets: p
                                            .targets
                                            .iter()
                                            .map(|t| {
                                                Ok(PluginTargetDownload {
                                                    target: t.target.clone(),
                                                    download: download(
                                                        t.download
                                                            .as_ref()
                                                            .ok_or(Error::Conflict)?,
                                                    ),
                                                })
                                            })
                                            .collect::<Result<_, Error>>()?,
                                    },
                                    _ => return Err(Error::Conflict),
                                };
                                Ok(PluginInstall {
                                    plugin_id: PluginId::new(p.plugin_id.clone()),
                                    version: PluginVersion::new(p.version.clone()),
                                    release,
                                    // Cloud's contract carries no declaration association yet:
                                    // the catalog snapshot slice extends the proto and maps it
                                    // through here. Until then no plan binds a declaration.
                                    agent_configuration: None,
                                })
                            })
                            .collect::<Result<_, Error>>()?,
                    },
                },
            })
        }
        Some(proto::execution_input::Spec::RemovePlugins(spec)) => {
            PluginCommand::Remove(RemovePluginsMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: operation,
                execution_id: execution,
                payload: RemovePlugins {
                    spec: RemovePluginsSpec {
                        node_id: node.clone(),
                        plugins: spec
                            .plugins
                            .iter()
                            .map(|p| PluginRemoval {
                                plugin_id: PluginId::new(p.plugin_id.clone()),
                                version: PluginVersion::new(p.version.clone()),
                            })
                            .collect(),
                    },
                },
            })
        }
        _ => return Err(Error::Conflict),
    };
    command.validate()?;
    Ok(command)
}

/// Preserves the release URL and required digest verbatim.
fn download(source: &proto::PluginDownload) -> PluginDownload {
    PluginDownload {
        url: source.url.clone(),
        sha256: Sha256Digest::new(source.sha256.clone()),
    }
}

/// Identifies the plugin family without guessing from an operation kind or a result.
pub(in crate::cloud) fn is_plugin(record: &proto::ExecutionRecord) -> bool {
    matches!(
        record.input.as_ref().and_then(|i| i.spec.as_ref()),
        Some(
            proto::execution_input::Spec::InstallPlugins(_)
                | proto::execution_input::Spec::RemovePlugins(_)
        )
    )
}

/// Converts finite Node outcomes to Cloud's contract; diagnostic strings never enter evidence.
pub(super) fn result(result: &PluginExecutionResult) -> proto::ExecutionResult {
    let (node, outcome) = match result {
        PluginExecutionResult::PluginsCompleted(completed) => (
            &completed.node,
            proto::execution_result::Outcome::PluginsResult(proto::PluginsResult {
                items: completed
                    .items
                    .iter()
                    .map(|item| proto::PluginItemResult {
                        plugin_id: item.plugin_id.as_str().into(),
                        outcome: Some(match &item.outcome {
                            PluginItemOutcome::Installed { version } => {
                                proto::plugin_item_result::Outcome::Installed(
                                    proto::PluginItemInstalled {
                                        version: version.as_str().into(),
                                    },
                                )
                            }
                            PluginItemOutcome::Removed {} => {
                                proto::plugin_item_result::Outcome::Removed(
                                    proto::PluginItemRemoved {},
                                )
                            }
                            PluginItemOutcome::Failed { failure } => {
                                proto::plugin_item_result::Outcome::Failed(
                                    proto::PluginItemFailed {
                                        reason: match failure {
                                            PluginFailureCode::DownloadFailed => {
                                                proto::PluginFailureReason::DownloadFailed
                                            }
                                            PluginFailureCode::ChecksumMismatch => {
                                                proto::PluginFailureReason::ChecksumMismatch
                                            }
                                            PluginFailureCode::NoMatchingTarget => {
                                                proto::PluginFailureReason::NoMatchingTarget
                                            }
                                            PluginFailureCode::InvalidPackage => {
                                                proto::PluginFailureReason::InvalidPackage
                                            }
                                            PluginFailureCode::InstallFailed => {
                                                proto::PluginFailureReason::InstallFailed
                                            }
                                            PluginFailureCode::PluginInUse => {
                                                proto::PluginFailureReason::PluginInUse
                                            }
                                        } as i32,
                                    },
                                )
                            }
                        }),
                    })
                    .collect(),
            }),
        ),
        PluginExecutionResult::PluginsFailed(failed) => (
            &failed.node,
            proto::execution_result::Outcome::PluginsFailed(proto::PluginsFailed {
                reason: match failed.failure {
                    PluginsFailureCode::PluginRootUnavailable => {
                        proto::PluginsFailureReason::PluginRootUnavailable
                    }
                    PluginsFailureCode::Interrupted => proto::PluginsFailureReason::Interrupted,
                } as i32,
            }),
        ),
    };
    proto::ExecutionResult {
        node: Some(proto::NodeIdentity {
            node_id: node.node_id.as_str().into(),
            node_incarnation_id: node.incarnation_id.as_str().into(),
        }),
        outcome: Some(outcome),
    }
}
