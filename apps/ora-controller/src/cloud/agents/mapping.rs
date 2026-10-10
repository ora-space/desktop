//! Lossless translation of Cloud-owned session inputs into the Node protocol.
use crate::*;
use ora_controller_proto::v1 as proto;

/// Identifies only session work; delivery remains a separate execution family.
pub(in crate::cloud) fn is_agent(record: &proto::ExecutionRecord) -> bool {
    matches!(
        record.input.as_ref().and_then(|v| v.spec.as_ref()),
        Some(proto::execution_input::Spec::AgentSession(_))
    )
}

/// Reconstructs a recorded start, using Cloud's stable Node-local operation identity.
pub(in crate::cloud) fn start(
    record: &proto::ExecutionRecord,
    node: &NodeId,
) -> Result<StartAgentSessionMessage, Error> {
    if record.node_id != node.as_str() {
        return Err(Error::Conflict);
    }
    let Some(proto::execution_input::Spec::AgentSession(spec)) =
        record.input.as_ref().and_then(|v| v.spec.as_ref())
    else {
        return Err(Error::Conflict);
    };
    let identity = spec.git_identity.as_ref().ok_or(Error::Conflict)?;
    let command = StartAgentSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new(record.node_operation_id.clone()),
        execution_id: ExecutionId::new(record.execution_id.clone()),
        payload: StartAgentSession {
            spec: AgentSessionSpec {
                node_id: node.clone(),
                agent_plugin_id: PluginId::new(spec.agent_plugin_id.clone()),
                agent_plugin_version: PluginVersion::new(spec.agent_plugin_version.clone()),
                checkout_execution_id: ExecutionId::new(spec.checkout_execution_id.clone()),
                git_identity: GitIdentity {
                    name: identity.name.clone(),
                    email: identity.email.clone(),
                },
                initial_turn: turn(spec.initial_turn.as_ref().ok_or(Error::Conflict)?)?,
                prior_revision: spec.prior_revision.as_ref().map(prior).transpose()?,
            },
        },
    };
    command.validate()?;
    Ok(command)
}

/// A session restores its prior Revision from exactly the object Cloud verified, so a prior
/// Revision without one cannot be restored and the input is refused.
fn prior(input: &proto::PriorRevision) -> Result<PriorRevision, Error> {
    let bundle = input.bundle.as_ref().ok_or(Error::Conflict)?;
    Ok(PriorRevision {
        revision_id: RevisionId::new(input.revision_id.clone()),
        final_commit: CommitId::new(input.final_commit.clone()),
        bundle: StoredObject {
            key: ObjectKey::new(bundle.key.clone()),
            size: bundle.size,
            sha256: Sha256Digest::new(bundle.sha256.clone()),
        },
    })
}

/// Whether a session input needs a Node that restores prior Revisions.
pub(in crate::cloud) fn resumes(input: Option<&proto::ExecutionInput>) -> bool {
    matches!(
        input.and_then(|v| v.spec.as_ref()),
        Some(proto::execution_input::Spec::AgentSession(spec)) if spec.prior_revision.is_some()
    )
}

/// Hands Cloud's download grant to the Node unchanged: headers verbatim and the same expiry. Only
/// a read of the session's own prior bundle is accepted, so Cloud can never point the restore at
/// another object.
pub(super) fn download_grants(
    response: proto::GrantRevisionDownloadResponse,
    command: &StartAgentSessionMessage,
) -> Result<DownloadGrantMessage, Error> {
    let bundle = &command
        .payload
        .spec
        .prior_revision
        .as_ref()
        .ok_or(Error::Conflict)?
        .bundle
        .key;
    let grants = response
        .grants
        .into_iter()
        .map(|grant| {
            let object_key = ObjectKey::new(grant.object_key);
            if object_key != *bundle || grant.method != "GET" {
                return Err(Error::Conflict);
            }
            Ok(ObjectDownloadGrant {
                object_key,
                url: PresignedUrl::new(grant.url),
                method: DownloadMethod::Get,
                headers: grant.headers.into_iter().collect(),
                expires_at: super::super::mapping::expiry(grant.expires_at)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let message = DownloadGrantMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command.operation_id.clone(),
        execution_id: command.execution_id.clone(),
        payload: DownloadGrant::Granted { grants },
    };
    message.validate()?;
    Ok(message)
}

/// Rejects unknown content blocks rather than silently losing part of a prompt.
fn turn(input: &proto::UserTurn) -> Result<UserTurn, Error> {
    Ok(UserTurn {
        turn_id: TurnId::new(input.turn_id.clone()),
        content: input
            .content
            .iter()
            .map(|block| match &block.block {
                Some(proto::content_block::Block::Text(text)) => Ok(ContentBlock::Text {
                    text: text.text.clone(),
                }),
                None => Err(Error::Conflict),
            })
            .collect::<Result<_, _>>()?,
    })
}

/// Correlates a command with its registered run, execution and destination before transmission.
pub(super) fn command(
    input: &proto::ThreadCommand,
    record: &proto::ExecutionRecord,
    node: &NodeId,
) -> Result<AgentCommand, Error> {
    let start = start(record, node)?;
    if input.run_id != record.operation_id
        || input.execution_id != record.execution_id
        || input
            .target
            .as_ref()
            .is_none_or(|target| target.node_id != node.as_str())
    {
        return Err(Error::Conflict);
    }
    let command = match input.command.as_ref().ok_or(Error::Conflict)? {
        proto::thread_command::Command::SubmitUserTurn(submit) => {
            AgentCommand::Submit(SubmitUserTurnMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: start.operation_id,
                execution_id: start.execution_id,
                payload: SubmitUserTurn {
                    node_id: node.clone(),
                    command_id: CommandId::new(input.command_id.clone()),
                    turn: turn(submit.turn.as_ref().ok_or(Error::Conflict)?)?,
                },
            })
        }
        proto::thread_command::Command::EndSession(end) => AgentCommand::End(EndSessionMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: start.operation_id,
            execution_id: start.execution_id,
            payload: EndSession {
                node_id: node.clone(),
                command_id: CommandId::new(input.command_id.clone()),
                reason: match proto::EndSessionReason::try_from(end.reason)
                    .map_err(|_| Error::Conflict)?
                {
                    proto::EndSessionReason::UserEnded => EndSessionReason::UserEnded,
                    proto::EndSessionReason::IdleTimeout => EndSessionReason::IdleTimeout,
                    proto::EndSessionReason::Cancelled => EndSessionReason::Cancelled,
                    proto::EndSessionReason::Unspecified => return Err(Error::Conflict),
                },
            },
        }),
    };
    command.message().validate()?;
    Ok(command)
}

/// Preserves the producing incarnation, including when a restarted Node replays old evidence.
pub(super) fn result(value: &AgentSessionResult) -> proto::ExecutionResult {
    let AgentSessionResult::AgentSessionEnded(ended) = value;
    proto::ExecutionResult {
        node: Some(proto::NodeIdentity {
            node_id: ended.node.node_id.as_str().into(),
            node_incarnation_id: ended.node.incarnation_id.as_str().into(),
        }),
        outcome: Some(proto::execution_result::Outcome::AgentSessionEnded(
            proto::AgentSessionEnded {
                reason: match ended.reason {
                    AgentSessionEndReason::UserEnded => proto::AgentSessionEndReason::UserEnded,
                    AgentSessionEndReason::IdleTimeout => proto::AgentSessionEndReason::IdleTimeout,
                    AgentSessionEndReason::Cancelled => proto::AgentSessionEndReason::Cancelled,
                    AgentSessionEndReason::AgentFailed => proto::AgentSessionEndReason::AgentFailed,
                    AgentSessionEndReason::Interrupted => proto::AgentSessionEndReason::Interrupted,
                } as i32,
                detail: ended.detail.clone(),
            },
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;

    const PRIOR_FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const BUNDLE: &str = "runs/prior/revision.bundle";

    /// A recorded session that resumes a prior Revision held in `BUNDLE`.
    fn resumed() -> proto::ExecutionRecord {
        proto::ExecutionRecord {
            operation_id: "run".into(),
            node_operation_id: "run".into(),
            execution_id: "session".into(),
            node_id: "node".into(),
            input: Some(proto::ExecutionInput {
                spec: Some(proto::execution_input::Spec::AgentSession(
                    proto::AgentSessionSpec {
                        agent_plugin_id: "official/ora-space.echo".into(),
                        agent_plugin_version: "1.2.3".into(),
                        checkout_execution_id: "clone".into(),
                        git_identity: Some(proto::GitIdentity {
                            name: "User".into(),
                            email: "user@example.com".into(),
                        }),
                        initial_turn: Some(proto::UserTurn {
                            turn_id: "turn".into(),
                            content: vec![proto::ContentBlock {
                                block: Some(proto::content_block::Block::Text(
                                    proto::TextContent {
                                        text: "Issue prompt".into(),
                                    },
                                )),
                            }],
                        }),
                        prior_revision: Some(proto::PriorRevision {
                            revision_id: "revision-1".into(),
                            final_commit: PRIOR_FINAL.into(),
                            bundle: Some(proto::StoredObject {
                                key: BUNDLE.into(),
                                size: 42,
                                sha256: DIGEST.into(),
                            }),
                        }),
                    },
                )),
            }),
            result: None,
        }
    }

    /// Edits the recorded prior Revision in place.
    fn edit_prior(
        record: &mut proto::ExecutionRecord,
        edit: impl FnOnce(&mut proto::PriorRevision),
    ) {
        if let Some(proto::execution_input::Spec::AgentSession(spec)) =
            record.input.as_mut().and_then(|i| i.spec.as_mut())
            && let Some(prior) = spec.prior_revision.as_mut()
        {
            edit(prior);
        }
    }

    /// The prior Revision reaches the Node exactly as Cloud fixed it, and a prior Revision
    /// without its verified bundle object is refused rather than started as a fresh session.
    #[test]
    fn resumed_sessions_carry_the_prior_revision() {
        let node = NodeId::new("node");
        let command = start(&resumed(), &node).unwrap();
        assert_eq!(
            command.payload.spec.prior_revision,
            Some(PriorRevision {
                revision_id: RevisionId::new("revision-1"),
                final_commit: CommitId::new(PRIOR_FINAL),
                bundle: StoredObject {
                    key: ObjectKey::new(BUNDLE),
                    size: 42,
                    sha256: Sha256Digest::new(DIGEST),
                },
            })
        );
        assert!(resumes(resumed().input.as_ref()));
        let mut fresh = resumed();
        if let Some(proto::execution_input::Spec::AgentSession(spec)) =
            fresh.input.as_mut().and_then(|i| i.spec.as_mut())
        {
            spec.prior_revision = None;
        }
        assert!(!resumes(fresh.input.as_ref()));
        assert_eq!(
            start(&fresh, &node).unwrap().payload.spec.prior_revision,
            None
        );

        let mut without_bundle = resumed();
        edit_prior(&mut without_bundle, |prior| prior.bundle = None);
        assert!(matches!(
            start(&without_bundle, &node),
            Err(Error::Conflict)
        ));
        let mut partial = resumed();
        edit_prior(&mut partial, |prior| prior.final_commit = "89abcdef".into());
        assert!(matches!(start(&partial, &node), Err(Error::Validation(_))));
    }

    /// A download grant passes through verbatim, and only as a read of the session's own bundle.
    #[test]
    fn download_grants_pass_through_only_for_the_prior_bundle() {
        let command = start(&resumed(), &NodeId::new("node")).unwrap();
        let headers = HashMap::from([("x-amz-meta-proof".to_owned(), "signed".to_owned())]);
        let grant = |key: &str, method: &str| proto::DownloadGrant {
            object_key: key.into(),
            url: "https://store.example/read?X-Amz-Signature=s".into(),
            method: method.into(),
            headers: headers.clone(),
            expires_at: Some(prost_types::Timestamp {
                seconds: 1_800_000_000,
                nanos: 5,
            }),
        };
        assert_eq!(
            download_grants(
                proto::GrantRevisionDownloadResponse {
                    grants: vec![grant(BUNDLE, "GET")],
                },
                &command,
            )
            .unwrap(),
            DownloadGrantMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new("run"),
                execution_id: ExecutionId::new("session"),
                payload: DownloadGrant::Granted {
                    grants: vec![ObjectDownloadGrant {
                        object_key: ObjectKey::new(BUNDLE),
                        url: PresignedUrl::new("https://store.example/read?X-Amz-Signature=s"),
                        method: DownloadMethod::Get,
                        headers: headers.clone().into_iter().collect(),
                        expires_at: time::OffsetDateTime::from_unix_timestamp_nanos(
                            1_800_000_000_000_000_005
                        )
                        .unwrap(),
                    }],
                },
            }
        );
        for wrong in [
            grant("runs/other/revision.bundle", "GET"),
            grant(BUNDLE, "PUT"),
        ] {
            assert!(matches!(
                download_grants(
                    proto::GrantRevisionDownloadResponse {
                        grants: vec![wrong],
                    },
                    &command,
                ),
                Err(Error::Conflict)
            ));
        }
        assert!(matches!(
            download_grants(
                proto::GrantRevisionDownloadResponse { grants: vec![] },
                &command,
            ),
            Err(Error::Validation(_))
        ));
    }
}
