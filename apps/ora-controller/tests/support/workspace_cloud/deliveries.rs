//! Revision delivery authority of the fake Cloud: queued delivery work, checksum-bound grants and
//! the receipt-only terminal takeover the real Cloud enforces for deliveries.
use super::*;

/// The fake object store the grants point at; tests assert it never reaches a log.
pub const STORE_HOST: &str = "store.example.invalid";
/// Signature material embedded in every grant URL and header.
pub const SIGNATURE: &str = "fake-signature-secret";

#[derive(Default)]
pub(super) struct DeliveryState {
    /// Every grant response Cloud issued, with its request, in order.
    issued: Vec<(
        proto::GrantRevisionUploadRequest,
        proto::GrantRevisionUploadResponse,
    )>,
    /// Refuses every grant as Cloud does once the run no longer delivers.
    refuse: bool,
    /// Every queried revision result the Controller tried to record; Cloud refuses them all.
    queried: usize,
}

impl WorkspaceCloud {
    /// Queues an exact delivery input for `run` against the current sandbox and wakes the
    /// Controller through the real signal stream.
    pub fn queue_delivery(&self, run: &str) -> proto::ExecutionInput {
        let mut state = self.lock();
        let sandbox = state.sandboxes.last().unwrap();
        let node = state.nodes.last().unwrap().identity.as_ref().unwrap();
        let input = proto::ExecutionInput {
            spec: Some(proto::execution_input::Spec::DeliverRevision(
                proto::DeliverRevisionSpec {
                    session_execution_id: format!("session-of-{run}"),
                    checkout_execution_id: state.clones[0].execution_id.clone(),
                    base_commit: "0123456789abcdef0123456789abcdef01234567".into(),
                    revision_ref: format!("refs/ora/revisions/{run}"),
                    bundle_key: format!("runs/{run}/revision.bundle"),
                    history_key: format!("runs/{run}/history.jsonl"),
                },
            )),
        };
        let item = proto::WorkItem {
            operation_id: run.into(),
            target: Some(proto::WorkTarget {
                workspace_id: WORKSPACE.into(),
                sandbox_instance_id: sandbox.id.clone(),
                node_id: node.node_id.clone(),
            }),
            input: Some(input.clone()),
        };
        state.agents.work.push(item);
        if let Some(subscriber) = &state.subscriber {
            let _ = subscriber.try_send(Ok(proto::WatchResponse {
                signal: Some(Signal::WorkAvailable(proto::WorkAvailable {
                    operation_id: Some(run.into()),
                })),
            }));
        }
        input
    }

    /// Registered delivery records, for assertions on input and result.
    pub fn delivery_records(&self) -> Vec<proto::ExecutionRecord> {
        self.lock()
            .clones
            .iter()
            .filter(|r| {
                matches!(
                    r.input.as_ref().and_then(|i| i.spec.as_ref()),
                    Some(proto::execution_input::Spec::DeliverRevision(_))
                )
            })
            .cloned()
            .collect()
    }

    /// Makes Cloud refuse grants with CONFLICT, as after the delivery settled or the run stopped.
    pub fn refuse_grants(&self, refuse: bool) {
        self.lock().agents.deliveries.refuse = refuse;
    }

    /// Every grant Cloud issued so far, with the request that asked for it.
    pub fn issued_grants(
        &self,
    ) -> Vec<(
        proto::GrantRevisionUploadRequest,
        proto::GrantRevisionUploadResponse,
    )> {
        self.lock().agents.deliveries.issued.clone()
    }

    /// How often the Controller tried to settle a delivery from a status query.
    pub fn queried_revisions(&self) -> usize {
        self.lock().agents.deliveries.queried
    }

    /// Signs one grant per requested checksum, only for a registered delivery without a result.
    pub(super) fn grant(
        &self,
        message: proto::GrantRevisionUploadRequest,
    ) -> Result<Response<proto::GrantRevisionUploadResponse>, Status> {
        let mut state = self.lock();
        if message.epoch != EPOCH {
            return Err(Status::failed_precondition("stale_controller"));
        }
        let record = state
            .clones
            .iter()
            .find(|r| r.execution_id == message.execution_id)
            .cloned()
            .ok_or_else(|| Status::not_found("not_found"))?;
        let Some(proto::execution_input::Spec::DeliverRevision(spec)) =
            record.input.as_ref().and_then(|i| i.spec.as_ref())
        else {
            return Err(Status::not_found("not_found"));
        };
        if state.agents.deliveries.refuse || record.result.is_some() {
            drop(state);
            self.timeline.push(Event::GrantRefused {
                execution: message.execution_id,
            });
            return Err(conflict("dispatch_conflict"));
        }
        if message.checksums.is_empty()
            || message
                .checksums
                .keys()
                .any(|key| *key != spec.bundle_key && *key != spec.history_key)
        {
            return Err(Status::invalid_argument("invalid_upload_checksum"));
        }
        let serial = state.agents.deliveries.issued.len();
        let mut checksums: Vec<_> = message
            .checksums
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        checksums.sort();
        let response = proto::GrantRevisionUploadResponse {
            grants: checksums
                .iter()
                .map(|(key, digest)| proto::UploadGrant {
                    object_key: key.clone(),
                    url: format!("https://{STORE_HOST}/{key}?X-Amz-Signature={SIGNATURE}-{serial}"),
                    method: "PUT".into(),
                    headers: [
                        ("If-None-Match".to_owned(), "*".to_owned()),
                        ("x-amz-checksum-sha256".to_owned(), digest.clone()),
                        (
                            "x-amz-meta-proof".to_owned(),
                            format!("{SIGNATURE}-{serial}"),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    expires_at: Some(prost_types::Timestamp {
                        seconds: 1_900_000_000 + serial as i64,
                        nanos: 0,
                    }),
                })
                .collect(),
        };
        state
            .agents
            .deliveries
            .issued
            .push((message.clone(), response.clone()));
        drop(state);
        self.timeline.push(Event::GrantIssued {
            execution: message.execution_id,
            checksums,
        });
        Ok(Response::new(response))
    }

    /// Takes over a delivery terminal only together with its receipt, waiting while the run is
    /// blocked so tests can hold the commit back.
    pub(super) async fn take_over_revision(
        &self,
        message: proto::TakeOverNodeEventRequest,
    ) -> Result<Response<proto::TakeOverNodeEventResponse>, Status> {
        self.timeline.push(Event::RevisionTakeoverAttempt {
            run: message.operation_id.clone(),
        });
        while self.lock().agents.blocked.contains(&message.operation_id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if message.epoch != EPOCH || message.event.is_empty() {
            return Err(Status::invalid_argument("invalid_receipt"));
        }
        let record = self.store_result(&message.execution_id, message.result)?;
        self.timeline.push(Event::RevisionTaken {
            run: message.operation_id,
        });
        Ok(Response::new(proto::TakeOverNodeEventResponse {
            record: Some(record),
        }))
    }

    /// The real Cloud requires a receipt for every delivery result, so a queried one is refused.
    pub(super) fn refuse_queried_revision(
        &self,
        message: &proto::RecordQueriedResultRequest,
    ) -> Result<(), Status> {
        if matches!(
            message.result.as_ref().and_then(|r| r.outcome.as_ref()),
            Some(
                proto::execution_result::Outcome::RevisionDelivered(_)
                    | proto::execution_result::Outcome::RevisionUnchanged(_)
                    | proto::execution_result::Outcome::RevisionFailed(_)
            )
        ) {
            self.lock().agents.deliveries.queried += 1;
            return Err(Status::invalid_argument("invalid_receipt"));
        }
        Ok(())
    }
}
