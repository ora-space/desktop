//! Rebuilding the sandbox targets from Cloud's list of live sandboxes when a lease is first held.
//! Without it a restarted Controller would reconnect to a sandbox only when that Workspace's next
//! operation is claimed, and Cloud would see the Node as unavailable until then.
use super::super::{
    CloudStore, fault,
    fleet::{Binding, Fleet},
};
use crate::*;
use ora_controller_proto::v1 as proto;
use std::collections::HashSet;

/// Starts a target for every sandbox Cloud lists and stops every target it does not: Cloud's record
/// is the only source of targets, so one not listed belongs to a sandbox that is being terminated,
/// was terminated, or was replaced by a newer generation while this process did not hold the lease.
pub(super) async fn rebuild(store: &CloudStore, fleet: &Fleet, epoch: i64) -> Result<(), Error> {
    let call = async {
        let request = store.request(proto::ListLiveSandboxesRequest { epoch });
        store.operations().list_live_sandboxes(request).await
    };
    let listed = fault::read(call)
        .await
        .map_err(|verdict| store.settle(verdict))?;
    let mut live = HashSet::new();
    for sandbox in listed.sandboxes {
        let proto::LiveSandbox {
            sandbox: Some(record),
            node_id,
            nodes,
        } = sandbox
        else {
            continue;
        };
        // Cloud always sets both; a listing without them names no endpoint to connect to.
        let Some(external_id) = record.substrate_sandbox_id else {
            continue;
        };
        if node_id.is_empty() {
            continue;
        }
        live.insert(record.id.clone());
        fleet.ensure(
            Binding {
                sandbox_id: record.id,
                generation: record.generation,
                node_id: NodeId::new(node_id),
                external_id,
            },
            nodes,
        );
    }
    for sandbox in fleet.sandboxes() {
        if !live.contains(&sandbox) {
            fleet.remove(&sandbox).await;
        }
    }
    ora_logging::ora_info!(
        sandboxes = live.len(),
        "sandbox Node targets rebuilt from Cloud"
    );
    Ok(())
}
