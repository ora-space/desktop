//! Download-grant relay for sessions that restore a prior Revision, independent of the frame pump.
//!
//! The Node asks with `DownloadGrantNeeded` while its restore waits, and again on every new
//! connection; the Controller therefore asks Cloud only when the Node does and keeps nothing across
//! connections. A Cloud refusal is answered to the Node as refused, which ends its restore; an
//! outage keeps the request and retries it, so it can never fail a restore by itself. Issued
//! grants pass straight to the transport and are never retained or logged.
use super::*;
use std::collections::HashMap;
use tokio::{sync::mpsc, task::JoinSet};

/// How long a request Cloud could not serve waits before it is tried again.
const RETRY_DELAY: Duration = Duration::from_millis(/*millis*/ 500);

/// A download grant request already validated as coming from this Node.
pub(super) struct Request {
    pub(super) operation: OperationId,
    pub(super) execution: ExecutionId,
}

/// One bounded worker per connection when the Node advertises Revision restore.
pub(super) struct Downloads {
    pub(super) input: mpsc::Sender<Request>,
    pub(super) outgoing: mpsc::Receiver<ControllerToNodeMessage>,
    pub(super) tasks: JoinSet<Result<(), Error>>,
}

impl Downloads {
    /// Starts the worker only for a restore-capable Node; otherwise requests are never produced.
    pub(super) fn new<S: CoordinationStore>(
        store: S,
        node: NodeRuntimeIdentity,
        enabled: bool,
    ) -> Self {
        let (input, receive) = mpsc::channel(64);
        let (output, outgoing) = mpsc::channel(16);
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

/// Serves requests until the connection drops. Repeated requests for one execution coalesce, so a
/// Node re-asking while Cloud is slow costs one call, not one per request.
async fn run<S: CoordinationStore>(
    store: S,
    node: NodeRuntimeIdentity,
    mut input: mpsc::Receiver<Request>,
    output: mpsc::Sender<ControllerToNodeMessage>,
) -> Result<(), Error> {
    let mut pending: HashMap<ExecutionId, OperationId> = HashMap::new();
    loop {
        if pending.is_empty() {
            let Some(request) = input.recv().await else {
                return Ok(());
            };
            pending.insert(request.execution, request.operation);
        }
        while let Ok(request) = input.try_recv() {
            pending.insert(request.execution, request.operation);
        }
        for (execution, operation) in std::mem::take(&mut pending) {
            match store.grant_download(&node, &operation, &execution).await {
                Ok(answer) => {
                    let granted = matches!(answer.payload, DownloadGrant::Granted { .. });
                    if output
                        .send(ControllerToNodeMessage::DownloadGrant(answer))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                    if granted {
                        ora_logging::ora_info!(execution_id = %execution.as_str(), "download grant relayed to the Node");
                    } else {
                        ora_logging::ora_info!(execution_id = %execution.as_str(), "prior Revision is not downloadable; the Node fails its restore");
                    }
                }
                // Nothing was decided; the same request is served once Cloud or the lease is back.
                Err(Error::Unavailable(_) | Error::Unknown(_) | Error::StaleEligibility) => {
                    pending.entry(execution).or_insert(operation);
                }
                Err(error) => {
                    ora_logging::ora_warn!(execution_id = %execution.as_str(), error = %error, "download grant could not be relayed; the Node asks again");
                }
            }
        }
        if !pending.is_empty() {
            tokio::select! {
                request = input.recv() => match request {
                    Some(request) => { pending.insert(request.execution, request.operation); }
                    None => return Ok(()),
                },
                () = tokio::time::sleep(RETRY_DELAY) => {}
            }
        }
    }
}
