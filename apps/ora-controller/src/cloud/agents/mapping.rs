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
                agent_configuration: spec
                    .agent_configuration
                    .as_ref()
                    .map(configuration)
                    .transpose()?,
                git_identity: GitIdentity {
                    name: identity.name.clone(),
                    email: identity.email.clone(),
                },
                initial_turn: turn(spec.initial_turn.as_ref().ok_or(Error::Conflict)?)?,
                prior_revision: spec.prior_revision.as_ref().map(prior).transpose()?,
                workspace_directory: spec.workspace_directory.iter().map(listing).collect(),
                // Cloud decides how the run was admitted; an unknown value is a contract the
                // Controller does not implement, so the session is refused rather than started as a
                // manual run that would never report its business result.
                trigger: match proto::AgentSessionTrigger::try_from(spec.trigger) {
                    Ok(proto::AgentSessionTrigger::Unspecified) => None,
                    Ok(proto::AgentSessionTrigger::Scheduled) => Some(SessionTrigger::Scheduled),
                    Err(_) => return Err(Error::Conflict),
                },
            },
        },
    };
    command.validate()?;
    Ok(command)
}

/// Carries Cloud's frozen adapter input verbatim; an unknown protocol or a missing model cannot be
/// guessed, so the input is refused rather than started on another route.
fn configuration(
    input: &proto::AgentModelConfiguration,
) -> Result<FrozenAgentConfiguration, Error> {
    let model = input.model.as_ref().ok_or(Error::Conflict)?;
    Ok(FrozenAgentConfiguration {
        plugin_id: PluginId::new(input.plugin_id.clone()),
        plugin_version: PluginVersion::new(input.plugin_version.clone()),
        target: input.target.clone(),
        package_sha256: Sha256Digest::new(input.package_sha256.clone()),
        schema_version: input.schema_version,
        declaration_fingerprint: Sha256Digest::new(input.declaration_fingerprint.clone()),
        adapter_version: input.adapter_version,
        configuration_revision: input.configuration_revision,
        settings_json: input.settings_json.clone(),
        protocol: ModelProtocol::parse(&input.protocol).ok_or(Error::Conflict)?,
        model: BoundModel {
            id: model.id.clone(),
            name: model.name.clone(),
            context_window: model.context_window,
            max_tokens: model.max_tokens,
        },
    })
}

/// Whether a session input carries a frozen adapter configuration, and which.
pub(in crate::cloud) fn frozen_configuration(
    input: Option<&proto::ExecutionInput>,
) -> Option<&proto::AgentModelConfiguration> {
    match input.and_then(|v| v.spec.as_ref()) {
        Some(proto::execution_input::Spec::AgentSession(spec)) => spec.agent_configuration.as_ref(),
        Some(_) | None => None,
    }
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
                // The business facts Cloud stores travel exactly as the executor measured them;
                // Cloud verifies them and never derives them from the Thread.
                scheduled_result: ended.scheduled_result.as_ref().map(reported),
                artifact_manifest: ended.artifact_manifest.as_ref().map(manifest),
            },
        )),
    }
}

/// Maps the executor's verdict onto the contract's enumeration.
fn reported(value: &ScheduledRunResult) -> proto::ScheduledRunResult {
    proto::ScheduledRunResult {
        verdict: match value.verdict {
            ScheduledVerdict::Pass => proto::ScheduledVerdict::Pass,
            ScheduledVerdict::Warn => proto::ScheduledVerdict::Warn,
            ScheduledVerdict::Fail => proto::ScheduledVerdict::Fail,
            ScheduledVerdict::Blocked => proto::ScheduledVerdict::Blocked,
            ScheduledVerdict::Idle => proto::ScheduledVerdict::Idle,
            ScheduledVerdict::RangeUnresolved => proto::ScheduledVerdict::RangeUnresolved,
        } as i32,
        verdict_detail: value.verdict_detail.clone(),
    }
}

/// Maps the declared artifacts and the refused candidates onto the contract's messages.
fn manifest(value: &ScheduledArtifactManifest) -> proto::ScheduledArtifactManifest {
    proto::ScheduledArtifactManifest {
        omitted_rejections: value.omitted_rejections,
        walk_truncated: value.walk_truncated,
        items: value
            .items
            .iter()
            .map(|item| proto::ScheduledArtifactItem {
                index: item.index,
                display_name: item.display_name.clone(),
                kind: match item.kind {
                    ScheduledArtifactKind::Markdown => proto::ScheduledArtifactKind::Markdown,
                    ScheduledArtifactKind::Json => proto::ScheduledArtifactKind::Json,
                    ScheduledArtifactKind::Text => proto::ScheduledArtifactKind::Text,
                    ScheduledArtifactKind::Log => proto::ScheduledArtifactKind::Log,
                    ScheduledArtifactKind::Xml => proto::ScheduledArtifactKind::Xml,
                    ScheduledArtifactKind::Html => proto::ScheduledArtifactKind::Html,
                } as i32,
                size_bytes: item.size_bytes,
                sha256: item.sha256.clone(),
            })
            .collect(),
        rejections: value
            .rejections
            .iter()
            .map(|rejection| proto::ScheduledArtifactRejection {
                display_name: rejection.display_name.clone(),
                reason: match rejection.reason {
                    ScheduledArtifactRejectionReason::UnsupportedType => {
                        proto::ScheduledArtifactRejectionReason::UnsupportedType
                    }
                    ScheduledArtifactRejectionReason::PathUnsafe => {
                        proto::ScheduledArtifactRejectionReason::PathUnsafe
                    }
                    ScheduledArtifactRejectionReason::Symlink => {
                        proto::ScheduledArtifactRejectionReason::Symlink
                    }
                    ScheduledArtifactRejectionReason::HardLink => {
                        proto::ScheduledArtifactRejectionReason::HardLink
                    }
                    ScheduledArtifactRejectionReason::TooLarge => {
                        proto::ScheduledArtifactRejectionReason::TooLarge
                    }
                    ScheduledArtifactRejectionReason::TooMany => {
                        proto::ScheduledArtifactRejectionReason::TooMany
                    }
                    ScheduledArtifactRejectionReason::TotalTooLarge => {
                        proto::ScheduledArtifactRejectionReason::TotalTooLarge
                    }
                    ScheduledArtifactRejectionReason::ReadFailed => {
                        proto::ScheduledArtifactRejectionReason::ReadFailed
                    }
                    ScheduledArtifactRejectionReason::CandidateLimit => {
                        proto::ScheduledArtifactRejectionReason::CandidateLimit
                    }
                } as i32,
            })
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const PRIOR_FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";

    /// A cut walk retains its distinct reason and both completeness facts at the Cloud boundary.
    #[test]
    fn truncated_artifact_manifest_keeps_its_completeness() {
        let declaration = ScheduledArtifactManifest {
            items: Vec::new(),
            rejections: vec![ScheduledArtifactRejection {
                display_name: ".ora-scheduled-artifacts".to_owned(),
                reason: ScheduledArtifactRejectionReason::CandidateLimit,
            }],
            omitted_rejections: 448,
            walk_truncated: true,
        };
        assert_eq!(
            manifest(&declaration),
            proto::ScheduledArtifactManifest {
                items: Vec::new(),
                rejections: vec![proto::ScheduledArtifactRejection {
                    display_name: ".ora-scheduled-artifacts".to_owned(),
                    reason: proto::ScheduledArtifactRejectionReason::CandidateLimit as i32,
                }],
                omitted_rejections: 448,
                walk_truncated: true,
            }
        );
    }
    const PRIOR_BASE: &str = "0123456789abcdef0123456789abcdef01234567";
    const REPOSITORY: &str = "https://git.ora.test/6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b/0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d.git";
    const DELIVERY_REF: &str = "refs/ora/deliveries/revision-1";

    /// A recorded session that resumes a prior Revision and lists two Workspaces.
    fn resumed() -> proto::ExecutionRecord {
        proto::ExecutionRecord {
            created_at: None,
            operation_id: "run".into(),
            node_operation_id: "run".into(),
            execution_id: "session".into(),
            node_id: "node".into(),
            input: Some(proto::ExecutionInput {
                spec: Some(proto::execution_input::Spec::AgentSession(
                    proto::AgentSessionSpec {
                        agent_configuration: None,
                        // A manual IssueRun carries no scheduled trigger.
                        trigger: 0,
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
                        agent_configuration: None,
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
                        trigger: None,
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

    /// A scheduled occurrence reaches the Node as one, and an unknown admission is refused rather
    /// than run as a manual session that would never report its business result.
    #[test]
    fn scheduled_admission_crosses_only_when_cloud_says_so() {
        let mut scheduled = resumed();
        edit(&mut scheduled, |spec| {
            spec.trigger = proto::AgentSessionTrigger::Scheduled as i32
        });
        assert_eq!(
            start(&scheduled, &NodeId::new("node"))
                .unwrap()
                .payload
                .spec
                .trigger,
            Some(SessionTrigger::Scheduled)
        );

        let manual = resumed();
        assert_eq!(
            start(&manual, &NodeId::new("node"))
                .unwrap()
                .payload
                .spec
                .trigger,
            None
        );

        let mut unknown = resumed();
        edit(&mut unknown, |spec| spec.trigger = 99);
        assert!(matches!(
            start(&unknown, &NodeId::new("node")),
            Err(Error::Conflict)
        ));
    }

    /// The executor's business facts reach Cloud exactly as it reported them.
    #[test]
    fn scheduled_results_carry_the_reported_facts() {
        let ended = AgentSessionResult::AgentSessionEnded(AgentSessionEnded {
            node: NodeRuntimeIdentity {
                node_id: NodeId::new("node"),
                incarnation_id: NodeIncarnationId::new("incarnation"),
            },
            reason: AgentSessionEndReason::IdleTimeout,
            detail: None,
            scheduled_result: Some(ScheduledRunResult {
                verdict: ScheduledVerdict::RangeUnresolved,
                verdict_detail: Some("no comparison base".to_owned()),
            }),
            artifact_manifest: Some(ScheduledArtifactManifest {
                items: vec![ScheduledArtifactItem {
                    index: 0,
                    display_name: "reports/review.md".to_owned(),
                    kind: ScheduledArtifactKind::Markdown,
                    size_bytes: 5,
                    sha256: "ab".repeat(32),
                }],
                rejections: vec![ScheduledArtifactRejection {
                    display_name: "notes.pdf".to_owned(),
                    reason: ScheduledArtifactRejectionReason::UnsupportedType,
                }],
                omitted_rejections: 448,
                walk_truncated: true,
            }),
        });
        let Some(proto::execution_result::Outcome::AgentSessionEnded(mapped)) =
            result(&ended).outcome
        else {
            panic!("a session result must map to a session outcome")
        };
        assert_eq!(
            mapped.scheduled_result,
            Some(proto::ScheduledRunResult {
                verdict: proto::ScheduledVerdict::RangeUnresolved as i32,
                verdict_detail: Some("no comparison base".to_owned()),
            })
        );
        assert_eq!(
            mapped.artifact_manifest,
            Some(proto::ScheduledArtifactManifest {
                omitted_rejections: 448,
                walk_truncated: true,
                items: vec![proto::ScheduledArtifactItem {
                    index: 0,
                    display_name: "reports/review.md".to_owned(),
                    kind: proto::ScheduledArtifactKind::Markdown as i32,
                    size_bytes: 5,
                    sha256: "ab".repeat(32),
                }],
                rejections: vec![proto::ScheduledArtifactRejection {
                    display_name: "notes.pdf".to_owned(),
                    reason: proto::ScheduledArtifactRejectionReason::UnsupportedType as i32,
                }],
            })
        );
    }
}
