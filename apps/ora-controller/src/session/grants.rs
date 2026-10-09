//! Upload-grant relay for running Revision deliveries, independent of the frame pump.
//!
//! Grants are requested when the Node reports `UploadGrantNeeded` (with the digests it froze) and
//! when this connection first sees a delivery the Node already runs, which covers a reconnect. They
//! are not requested right after sending `DeliverRevision`: the Node has not frozen its objects
//! yet, so such a grant could not bind their checksums, and the Node asks as soon as it can.
//!
//! Every piece of state here is connection-local and dropped with the connection; issued grants
//! pass straight to the transport and are never retained or logged.
use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use tokio::{sync::mpsc, task::JoinSet};

/// How long a grant request Cloud could not serve waits before it is tried again.
const RETRY_DELAY: Duration = Duration::from_millis(/*millis*/ 500);

/// A reason to ask Cloud for grants, already validated as coming from this Node.
pub(super) enum Request {
    /// The Node holds no valid grant for these objects.
    Needed {
        operation: OperationId,
        execution: ExecutionId,
        checksums: BTreeMap<ObjectKey, Sha256Digest>,
    },
    /// The Node reported an execution as accepted or running on this connection.
    Running {
        operation: OperationId,
        execution: ExecutionId,
    },
}

/// One bounded worker per connection when the Node advertises Revision delivery.
pub(super) struct Grants {
    pub(super) input: mpsc::Sender<Request>,
    pub(super) outgoing: mpsc::Receiver<ControllerToNodeMessage>,
    pub(super) tasks: JoinSet<Result<(), Error>>,
}

impl Grants {
    /// Starts the worker only for a delivery-capable Node; otherwise requests are never produced.
    pub(super) fn new<S: CoordinationStore>(
        store: S,
        node: NodeRuntimeIdentity,
        enabled: bool,
    ) -> Self {
        let (input, receive) = mpsc::channel(256);
        let (output, outgoing) = mpsc::channel(64);
        let mut tasks = JoinSet::new();
        if enabled {
            tasks.spawn(run(store, node, receive, output));
        }
        Self {
            input,
            outgoing,
            tasks,
        }
    }
}

/// Serves requests until the connection drops. Requests for one execution coalesce, so only the
/// newest digests are used; a Cloud outage keeps the request and retries it later, while a refusal
/// drops it without ever turning into a delivery failure.
async fn run<S: CoordinationStore>(
    store: S,
    node: NodeRuntimeIdentity,
    mut input: mpsc::Receiver<Request>,
    output: mpsc::Sender<ControllerToNodeMessage>,
) -> Result<(), Error> {
    let mut seen = HashSet::new();
    let mut pending: HashMap<ExecutionId, (OperationId, GrantRequest)> = HashMap::new();
    loop {
        if pending.is_empty() {
            let Some(request) = input.recv().await else {
                return Ok(());
            };
            accept(&mut seen, &mut pending, request);
        }
        while let Ok(request) = input.try_recv() {
            accept(&mut seen, &mut pending, request);
        }
        for (execution, (operation, request)) in std::mem::take(&mut pending) {
            match store
                .grant_upload(&node, &operation, &execution, request.clone())
                .await
            {
                Ok(GrantOutcome::Issued(message)) => {
                    let count = message.payload.grants.len();
                    if output
                        .send(ControllerToNodeMessage::UploadGrant(message))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                    ora_logging::ora_info!(execution_id = %execution.as_str(), grants = count, "upload grants relayed to the Node");
                }
                Ok(GrantOutcome::NotGrantable) => {
                    ora_logging::ora_info!(execution_id = %execution.as_str(), "delivery is no longer grantable; Cloud settles its run");
                }
                Ok(GrantOutcome::AwaitNode) => {}
                // Nothing was issued; the same request is served once Cloud or the lease is back.
                Err(Error::Unavailable(_) | Error::Unknown(_) | Error::StaleEligibility) => {
                    pending.entry(execution).or_insert((operation, request));
                }
                Err(error) => {
                    ora_logging::ora_warn!(execution_id = %execution.as_str(), error = %error, "upload grants could not be relayed; the Node asks again");
                }
            }
        }
        if !pending.is_empty() {
            tokio::select! {
                request = input.recv() => match request {
                    Some(request) => accept(&mut seen, &mut pending, request),
                    None => return Ok(()),
                },
                () = tokio::time::sleep(RETRY_DELAY) => {}
            }
        }
    }
}

/// Coalesces one request. A Node report always wins; a running status triggers a resumed request
/// only the first time this connection sees the execution, so polling does not refresh grants.
fn accept(
    seen: &mut HashSet<ExecutionId>,
    pending: &mut HashMap<ExecutionId, (OperationId, GrantRequest)>,
    request: Request,
) {
    match request {
        Request::Needed {
            operation,
            execution,
            checksums,
        } => {
            seen.insert(execution.clone());
            pending.insert(execution, (operation, GrantRequest::Needed(checksums)));
        }
        Request::Running {
            operation,
            execution,
        } => {
            if seen.insert(execution.clone()) {
                pending
                    .entry(execution)
                    .or_insert((operation, GrantRequest::Resumed));
            }
        }
    }
}
