//! Node reports the Controller makes to Cloud on behalf of the sandbox Nodes it holds sessions
//! with (Cloud `NodeReportService`). A desktop Node never talks to Cloud; the handshake the
//! Controller completed and the session it keeps are the evidence. Every report of one sandbox runs
//! under that sandbox's report lock, so the Node record version each call presents is the one the
//! previous call returned.
use super::{CloudStore, fault, fleet::Sandbox};
use crate::*;
use ora_controller_proto::v1::{
    self as proto, node_report_service_client::NodeReportServiceClient,
};
use tonic::transport::Channel;

/// Cloud's record of the incarnation this Controller last registered for a sandbox.
pub(super) struct Record {
    id: String,
    version: i64,
    incarnation: NodeIncarnationId,
}

/// What this Controller has told Cloud about one sandbox's Node.
#[derive(Default)]
pub(super) struct Reported {
    record: Option<Record>,
    /// Whether the last status Cloud accepted said connected.
    connected: bool,
}

impl Reported {
    /// Whether Cloud has a registered, connected incarnation from this Controller: what the
    /// node step's evidence needs before the operation may advance.
    pub(super) fn ready(&self) -> bool {
        self.record.is_some() && self.connected
    }
}

impl CloudStore {
    fn nodes(&self) -> NodeReportServiceClient<Channel> {
        NodeReportServiceClient::new(self.inner.channel.clone())
    }
}

/// Brings Cloud in line with the session: a new incarnation is registered (ending the previous one
/// first), a live one refreshes its heartbeat, and a session that ended is reported disconnected
/// once. Failures are logged and retried on the next tick; they never touch the session.
pub(super) async fn sync(store: &CloudStore, sandbox: &Sandbox) {
    let current = sandbox.identity();
    let mut reported = sandbox.report.lock().await;
    let result = match (current, &reported.record) {
        (Some(node), Some(record)) if record.incarnation == node.incarnation_id => {
            status(store, &mut reported, proto::NodeConnection::Connected).await
        }
        (Some(node), _) => replace(store, sandbox, &mut reported, &node).await,
        (None, Some(_)) if reported.connected => {
            status(store, &mut reported, proto::NodeConnection::Disconnected).await
        }
        (None, _) => Ok(()),
    };
    if let Err(error) = result {
        ora_logging::ora_warn!(
            sandbox_instance_id = %sandbox.binding.sandbox_id,
            error = %error,
            "Node report to Cloud failed; retrying on the next report"
        );
    }
}

/// Registers `node` as the sandbox's current incarnation. A previous incarnation this Controller
/// registered, or one Cloud still lists as live (after a Controller restart), is ended first:
/// Cloud accepts a new incarnation only once the old one is over.
async fn replace(
    store: &CloudStore,
    sandbox: &Sandbox,
    reported: &mut Reported,
    node: &NodeRuntimeIdentity,
) -> Result<(), Error> {
    if let Some(old) = reported.record.take() {
        reported.connected = false;
        end(store, &old.id, old.version).await?;
    }
    match register(store, sandbox, reported, node).await {
        Err(Error::Conflict) => {
            for (id, version) in sandbox.stale_incarnations(&node.incarnation_id) {
                end(store, &id, version).await?;
            }
            register(store, sandbox, reported, node).await
        }
        result => result,
    }
}

/// `RegisterNode`, idempotent by (sandbox, incarnation): repeating it also reads the current
/// record back, which is how a stale version is refreshed.
async fn register(
    store: &CloudStore,
    sandbox: &Sandbox,
    reported: &mut Reported,
    node: &NodeRuntimeIdentity,
) -> Result<(), Error> {
    let epoch = store.epoch()?;
    let binding = &sandbox.binding;
    let response = fault::write(|submission_id| async move {
        let request = store.request(proto::RegisterNodeRequest {
            submission_id,
            epoch,
            sandbox_instance_id: binding.sandbox_id.clone(),
            generation: binding.generation,
            node: Some(proto::NodeIdentity {
                node_id: node.node_id.as_str().into(),
                node_incarnation_id: node.incarnation_id.as_str().into(),
            }),
            protocol_version: u32::from(CURRENT_PROTOCOL_VERSION.value()),
        });
        store.nodes().register_node(request).await
    })
    .await
    .map_err(|verdict| store.settle(verdict))?;
    let record = response.node.ok_or(Error::Conflict)?;
    reported.connected = record.connection == proto::NodeConnection::Connected as i32;
    reported.record = Some(Record {
        id: record.id,
        version: record.version,
        incarnation: node.incarnation_id.clone(),
    });
    ora_logging::ora_info!(
        sandbox_instance_id = %binding.sandbox_id,
        node_id = %node.node_id.as_str(),
        node_incarnation_id = %node.incarnation_id.as_str(),
        "Node registered with Cloud"
    );
    Ok(())
}

/// `ReportNodeStatus`; a version conflict re-reads the record once through `RegisterNode`.
async fn status(
    store: &CloudStore,
    reported: &mut Reported,
    connection: proto::NodeConnection,
) -> Result<(), Error> {
    let Some(record) = reported.record.as_mut() else {
        return Ok(());
    };
    let epoch = store.epoch()?;
    let (id, version) = (record.id.clone(), record.version);
    let response = fault::write(|submission_id| {
        let id = id.clone();
        async move {
            let request = store.request(proto::ReportNodeStatusRequest {
                submission_id,
                epoch,
                node_instance_id: id,
                version,
                connection: connection as i32,
                initialized: true,
            });
            store.nodes().report_node_status(request).await
        }
    })
    .await;
    match response {
        Ok(response) => {
            let node = response.node.ok_or(Error::Conflict)?;
            record.version = node.version;
            reported.connected = connection == proto::NodeConnection::Connected;
            Ok(())
        }
        // The record moved on (an idle report, or Cloud ended it); forget it so the next sync
        // registers again, which reads it back or reports a new incarnation.
        Err(verdict) => {
            let error = store.settle(verdict);
            if matches!(error, Error::Conflict) {
                reported.record = None;
                reported.connected = false;
            }
            Err(error)
        }
    }
}

/// `EndNode`; ending an incarnation Cloud already ended is not an error.
async fn end(store: &CloudStore, id: &str, version: i64) -> Result<(), Error> {
    let epoch = store.epoch()?;
    fault::write(|submission_id| async move {
        let request = store.request(proto::EndNodeRequest {
            submission_id,
            epoch,
            node_instance_id: id.into(),
            version,
        });
        store.nodes().end_node(request).await
    })
    .await
    .map(drop)
    .map_err(|verdict| store.settle(verdict))
}

/// Reports quiesce evidence for the sandbox's registered incarnation. Returns whether Cloud
/// accepted it; `false` means Cloud restored admission and failed the operation.
pub(super) async fn idle(
    store: &CloudStore,
    sandbox: &Sandbox,
    operation: &str,
    admission_epoch: i64,
    idle: bool,
) -> Result<bool, Error> {
    let mut reported = sandbox.report.lock().await;
    let Some(record) = reported.record.as_mut() else {
        return Err(Error::Conflict);
    };
    let epoch = store.epoch()?;
    let (id, version) = (record.id.clone(), record.version);
    let response = fault::write(|submission_id| {
        let id = id.clone();
        async move {
            let request = store.request(proto::ReportNodeIdleRequest {
                submission_id,
                epoch,
                node_instance_id: id,
                version,
                operation_id: operation.into(),
                admission_epoch,
                idle,
            });
            store.nodes().report_node_idle(request).await
        }
    })
    .await
    .map_err(|verdict| store.settle(verdict))?;
    if let Some(node) = response.node {
        record.version = node.version;
    }
    Ok(response.accepted)
}
