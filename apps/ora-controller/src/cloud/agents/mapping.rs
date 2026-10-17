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
                model_binding_id: (!spec.model_binding_id.is_empty())
                    .then(|| ModelBindingId::new(spec.model_binding_id.clone())),
                git_identity: GitIdentity {
                    name: identity.name.clone(),
                    email: identity.email.clone(),
                },
                initial_turn: turn(spec.initial_turn.as_ref().ok_or(Error::Conflict)?)?,
                prior_revision: spec.prior_revision.as_ref().map(prior).transpose()?,
                workspace_directory: spec.workspace_directory.iter().map(listing).collect(),
            },
        },
    };
    command.validate()?;
    Ok(command)
}

/// A session restores its prior Revision from the Git Server location Cloud fixed, so a prior
/// Revision whose repository or delivery ref the Node protocol refuses cannot be restored and the
/// input is refused rather than started as a fresh session.
fn prior(input: &proto::PriorRevision) -> Result<PriorRevision, Error> {
    Ok(PriorRevision {
        revision_id: RevisionId::new(input.revision_id.clone()),
        final_commit: CommitId::new(input.final_commit.clone()),
        git_repository_url: GitRepositoryUrl::parse(&input.git_repository_url)
            .map_err(|_| Error::Conflict)?,
        delivery_ref: DeliveryRef::parse(&input.delivery_ref).map_err(|_| Error::Conflict)?,
        base_commit: CommitId::new(input.base_commit.clone()),
    })
}

/// Forwards one Workspace directory entry verbatim. Proto strings cannot be absent, so the empty
/// string Cloud sends for fields that do not apply (a non-run Workspace) becomes `None`.
fn listing(input: &proto::WorkspaceListing) -> WorkspaceListing {
    let optional = |value: &str| (!value.is_empty()).then(|| value.to_owned());
    WorkspaceListing {
        workspace_id: input.workspace_id.clone(),
        kind: input.kind.clone(),
        issue_id: optional(&input.issue_id),
        issue_title: optional(&input.issue_title),
        issue_run_id: optional(&input.issue_run_id),
        run_status: optional(&input.run_status),
    }
}

/// Whether a session input needs a Node that restores prior Revisions.
pub(in crate::cloud) fn resumes(input: Option<&proto::ExecutionInput>) -> bool {
    matches!(
        input.and_then(|v| v.spec.as_ref()),
        Some(proto::execution_input::Spec::AgentSession(spec)) if spec.prior_revision.is_some()
    )
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

    const PRIOR_FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const PRIOR_BASE: &str = "0123456789abcdef0123456789abcdef01234567";
    const REPOSITORY: &str = "https://git.ora.test/6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b/0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d.git";
    const DELIVERY_REF: &str = "refs/ora/deliveries/revision-1";

    /// A recorded session that resumes a prior Revision and lists two Workspaces.
    fn resumed() -> proto::ExecutionRecord {
        proto::ExecutionRecord {
            operation_id: "run".into(),
            node_operation_id: "run".into(),
            execution_id: "session".into(),
            node_id: "node".into(),
            input: Some(proto::ExecutionInput {
                spec: Some(proto::execution_input::Spec::AgentSession(
                    proto::AgentSessionSpec {
                        model_binding_id: String::new(),
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
                            git_repository_url: REPOSITORY.into(),
                            delivery_ref: DELIVERY_REF.into(),
                            base_commit: PRIOR_BASE.into(),
                        }),
                        workspace_directory: vec![
                            proto::WorkspaceListing {
                                workspace_id: "4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e".into(),
                                kind: "run".into(),
                                issue_id: "issue-7".into(),
                                issue_title: "Fix the parser".into(),
                                issue_run_id: "run-3".into(),
                                run_status: "succeeded".into(),
                            },
                            proto::WorkspaceListing {
                                workspace_id: "1e2d3c4b-5a69-4788-9a6b-5c4d3e2f1a0b".into(),
                                kind: "main".into(),
                                issue_id: String::new(),
                                issue_title: String::new(),
                                issue_run_id: String::new(),
                                run_status: String::new(),
                            },
                        ],
                    },
                )),
            }),
            result: None,
        }
    }

    /// Edits the recorded session input in place.
    fn edit(record: &mut proto::ExecutionRecord, edit: impl FnOnce(&mut proto::AgentSessionSpec)) {
        if let Some(proto::execution_input::Spec::AgentSession(spec)) =
            record.input.as_mut().and_then(|i| i.spec.as_mut())
        {
            edit(spec);
        }
    }

    /// The prior Revision and the Workspace directory reach the Node exactly as Cloud fixed them.
    #[test]
    fn resumed_sessions_carry_the_prior_revision_and_the_directory_verbatim() {
        let command = start(&resumed(), &NodeId::new("node")).unwrap();
        assert_eq!(
            command,
            StartAgentSessionMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new("run"),
                execution_id: ExecutionId::new("session"),
                payload: StartAgentSession {
                    spec: AgentSessionSpec {
                        node_id: NodeId::new("node"),
                        agent_plugin_id: PluginId::new("official/ora-space.echo"),
                        agent_plugin_version: PluginVersion::new("1.2.3"),
                        checkout_execution_id: ExecutionId::new("clone"),
                        model_binding_id: None,
                        git_identity: GitIdentity {
                            name: "User".into(),
                            email: "user@example.com".into(),
                        },
                        initial_turn: UserTurn {
                            turn_id: TurnId::new("turn"),
                            content: vec![ContentBlock::Text {
                                text: "Issue prompt".into(),
                            }],
                        },
                        prior_revision: Some(PriorRevision {
                            revision_id: RevisionId::new("revision-1"),
                            final_commit: CommitId::new(PRIOR_FINAL),
                            git_repository_url: GitRepositoryUrl::parse(REPOSITORY).unwrap(),
                            delivery_ref: DeliveryRef::parse(DELIVERY_REF).unwrap(),
                            base_commit: CommitId::new(PRIOR_BASE),
                        }),
                        workspace_directory: vec![
                            WorkspaceListing {
                                workspace_id: "4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e".into(),
                                kind: "run".into(),
                                issue_id: Some("issue-7".into()),
                                issue_title: Some("Fix the parser".into()),
                                issue_run_id: Some("run-3".into()),
                                run_status: Some("succeeded".into()),
                            },
                            WorkspaceListing {
                                workspace_id: "1e2d3c4b-5a69-4788-9a6b-5c4d3e2f1a0b".into(),
                                kind: "main".into(),
                                issue_id: None,
                                issue_title: None,
                                issue_run_id: None,
                                run_status: None,
                            },
                        ],
                    },
                },
            }
        );
        assert!(resumes(resumed().input.as_ref()));
        let mut fresh = resumed();
        edit(&mut fresh, |spec| spec.prior_revision = None);
        assert!(!resumes(fresh.input.as_ref()));
        assert_eq!(
            start(&fresh, &NodeId::new("node"))
                .unwrap()
                .payload
                .spec
                .prior_revision,
            None
        );
    }

    /// A prior Revision the Node could not fetch from the Git Server is refused rather than
    /// started as a fresh session.
    #[test]
    fn prior_revisions_outside_the_git_server_contract_are_refused() {
        let node = NodeId::new("node");
        let prior = |change: fn(&mut proto::PriorRevision)| {
            let mut record = resumed();
            edit(&mut record, |spec| {
                if let Some(prior) = spec.prior_revision.as_mut() {
                    change(prior);
                }
            });
            start(&record, &node)
        };
        for change in [
            (|prior: &mut proto::PriorRevision| prior.git_repository_url = String::new())
                as fn(&mut proto::PriorRevision),
            |prior| prior.git_repository_url = "http://git.ora.test/t/p.git".into(),
            |prior| prior.git_repository_url = "https://user@git.ora.test/t/p.git".into(),
            |prior| prior.delivery_ref = String::new(),
            |prior| prior.delivery_ref = "refs/heads/main".into(),
        ] {
            assert!(matches!(prior(change), Err(Error::Conflict)));
        }
        for change in [
            (|prior: &mut proto::PriorRevision| prior.final_commit = "89abcdef".into())
                as fn(&mut proto::PriorRevision),
            |prior| prior.base_commit = String::new(),
        ] {
            assert!(matches!(prior(change), Err(Error::Validation(_))));
        }
    }
}
