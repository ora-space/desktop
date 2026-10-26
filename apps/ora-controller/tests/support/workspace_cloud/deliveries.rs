//! Revision delivery authority of the fake Cloud: queued delivery work, checksum-bound grants and
//! the receipt-only terminal takeover the real Cloud enforces for deliveries.
use super::*;
use ora_node_protocol::ExecutionId;
use prost::Message;
use std::collections::{HashMap, HashSet};

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
    /// Per-run terminal failures, separate from upload-grant refusal.
    terminal_faults: HashMap<String, Status>,
    /// Registration failures remain independent of delivery terminal takeover.
    dispatch_faults: HashMap<String, Status>,
    /// Exact terminal requests accepted or refused at the real gRPC boundary.
    attempts: Vec<proto::TakeOverNodeEventRequest>,
    /// Durable event receipts, separate from execution results.
    receipts: HashMap<String, proto::TakeOverNodeEventRequest>,
    /// Receipt-only commits deliberately omit the execution result returned by the fake.
    receipt_only: HashSet<String>,
    /// Runs whose next committed response is lost in transit.
    lose_reply: HashSet<String>,
    /// Pending-list omissions let quiesce exercise the observer's independent responsibility.
    hidden_pending: HashSet<String>,
}

/// The standard gRPC status envelope used by Cloud for its typed `ErrorDetail`.
#[derive(Clone, PartialEq, Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<prost_types::Any>,
}

/// Builds the complete transport/detail/message triple, including deliberately mismatched cases.
pub fn terminal_status(code: tonic::Code, detail: proto::ErrorCode, message: &str) -> Status {
    let envelope = RpcStatus {
        code: code as i32,
        message: message.into(),
        details: vec![prost_types::Any {
            type_url: "type.googleapis.com/ora.cloud.internal.v1.ErrorDetail".into(),
            value: proto::ErrorDetail {
                code: detail as i32,
            }
            .encode_to_vec(),
        }],
    };
    Status::with_details(code, message, envelope.encode_to_vec().into())
}

impl WorkspaceCloud {
    /// Refuses normal registration; removing the fault wakes the same still-queued work.
    pub fn dispatch_fault(&self, run: &str, fault: Option<Status>) {
        let mut state = self.lock();
        if let Some(fault) = fault {
            state
                .agents
                .deliveries
                .dispatch_faults
                .insert(run.into(), fault);
        } else {
            state.agents.deliveries.dispatch_faults.remove(run);
            if let Some(subscriber) = &state.subscriber {
                let _ = subscriber.try_send(Ok(proto::WatchResponse {
                    signal: Some(Signal::WorkAvailable(proto::WorkAvailable {
                        operation_id: Some(run.into()),
                    })),
                }));
            }
        }
    }

    /// Returns the ordinary dispatch fault before the fake registers any execution.
    pub(super) fn dispatch_refusal(&self, run: &str) -> Option<Status> {
        self.lock()
            .agents
            .deliveries
            .dispatch_faults
            .get(run)
            .cloned()
    }

    /// Models Cloud giving up delivery while retaining the original execution and live Node.
    pub fn give_up_delivery(&self, run: &str) {
        let mut state = self.lock();
        state.workspace.observed_state = "deleting".into();
        state.workspace.admission_open = false;
        state.agents.deliveries.refuse = true;
        state.agents.deliveries.terminal_faults.insert(
            run.into(),
            terminal_status(
                tonic::Code::Aborted,
                proto::ErrorCode::Conflict,
                "dispatch_conflict",
            ),
        );
    }

    /// Changes only the original terminal RPC's outcome; dispatch ownership remains unchanged.
    pub fn terminal_fault(&self, run: &str, fault: Option<Status>) {
        let mut state = self.lock();
        if let Some(fault) = fault {
            state
                .agents
                .deliveries
                .terminal_faults
                .insert(run.into(), fault);
        } else {
            state.agents.deliveries.terminal_faults.remove(run);
        }
    }

    /// Returns the exact requests so replay tests compare original identities and event bytes.
    pub fn revision_attempts(&self, run: &str) -> Vec<proto::TakeOverNodeEventRequest> {
        self.lock()
            .agents
            .deliveries
            .attempts
            .iter()
            .filter(|request| request.operation_id == run)
            .cloned()
            .collect()
    }

    /// Exposes the receipt that alone permits the Controller to acknowledge a terminal.
    pub fn revision_receipt(
        &self,
        execution: &ExecutionId,
    ) -> Option<proto::TakeOverNodeEventRequest> {
        self.lock()
            .agents
            .deliveries
            .receipts
            .get(execution.as_str())
            .cloned()
    }

    /// Keeps takeover success dependent on its exact receipt, without an execution-result hint.
    pub fn receipt_only_revision(&self, run: &str) {
        self.lock()
            .agents
            .deliveries
            .receipt_only
            .insert(run.into());
    }

    /// Loses a response after the exact receipt was committed.
    pub fn lose_revision_reply(&self, run: &str) {
        self.lock().agents.deliveries.lose_reply.insert(run.into());
    }

    /// Keeps the record addressable but omits it from pending polling during observer assertions.
    pub fn hide_delivery_pending(&self, execution: &ExecutionId) {
        self.lock()
            .agents
            .deliveries
            .hidden_pending
            .insert(execution.as_str().into());
    }

    /// Takes a snapshot before pending listing holds the state lock.
    pub(super) fn hidden_delivery_executions(&self) -> HashSet<String> {
        self.lock().agents.deliveries.hidden_pending.clone()
    }

    /// Reads registered identity and reported connection without changing either.
    pub fn delivery_node_records(&self) -> Vec<proto::NodeRecord> {
        self.lock().nodes.clone()
    }
    /// Queues an exact delivery input for `run` against the current sandbox and wakes the
    /// Controller through the real signal stream.
    pub fn queue_delivery(&self, run: &str) -> proto::ExecutionInput {
        let mut state = self.lock();
        let sandbox = state.sandboxes.last().unwrap();
        let node = state.nodes.last().unwrap().identity.as_ref().unwrap();
        let input = proto::ExecutionInput {
            spec: Some(proto::execution_input::Spec::DeliverRevision(
                proto::DeliverRevisionSpec {
                    workflow: None,
                    session_execution_id: format!("session-of-{run}"),
                    checkout_execution_id: state.clones[0].execution_id.clone(),
                    base_commit: BASE_COMMIT.into(),
                    revision_ref: format!("refs/ora/revisions/{run}"),
                    history_key: format!("runs/{run}/history.jsonl"),
                    prior_revision: None,
                    git_repository_url: REPOSITORY.into(),
                    delivery_ref: format!("refs/ora/deliveries/{run}"),
                    workspace_ref: format!("refs/heads/workspaces/{WORKSPACE}/main"),
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
            || message.checksums.keys().any(|key| *key != spec.history_key)
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
        self.lock().agents.deliveries.attempts.push(message.clone());
        while self.lock().agents.blocked.contains(&message.operation_id) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if message.epoch != EPOCH || message.event.is_empty() {
            return Err(Status::invalid_argument("invalid_receipt"));
        }
        if let Some(fault) = self
            .lock()
            .agents
            .deliveries
            .terminal_faults
            .get(&message.operation_id)
            .cloned()
        {
            return Err(fault);
        }
        if let Some(receipt) = self
            .lock()
            .agents
            .deliveries
            .receipts
            .get(&message.execution_id)
            && (receipt.sequence != message.sequence
                || receipt.event != message.event
                || receipt.result != message.result)
        {
            return Err(conflict("receipt_conflict"));
        }
        let record = if self
            .lock()
            .agents
            .deliveries
            .receipt_only
            .contains(&message.operation_id)
        {
            self.lock()
                .clones
                .iter()
                .find(|record| record.execution_id == message.execution_id)
                .cloned()
                .ok_or_else(|| Status::not_found("not_found"))?
        } else {
            self.store_result(&message.execution_id, message.result.clone())?
        };
        let lose_reply = {
            let mut state = self.lock();
            state
                .agents
                .deliveries
                .receipts
                .entry(message.execution_id.clone())
                .or_insert(message.clone());
            state
                .agents
                .deliveries
                .lose_reply
                .remove(&message.operation_id)
        };
        self.timeline.push(Event::RevisionTaken {
            run: message.operation_id,
        });
        if lose_reply {
            return Err(Status::deadline_exceeded("lost committed revision reply"));
        }
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
