//! Version views transfer between durable attempts without deleting a live sibling's bytes.
use crate::session::{PluginCatalog, SessionLedger, Shared};
use ora_node_db::{SessionStopChange, SessionStopIdentity, SessionStopKind};
use std::{io, sync::PoisonError};

/// Callers have already proven the attempt stopped and dropped its volatile exact-version leases.
pub(super) fn release<L: SessionLedger, C, P: PluginCatalog, R>(
    shared: &Shared<L, C, P, R>,
    identity: &SessionStopIdentity,
) -> io::Result<()> {
    let retained = shared
        .retained
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let records = shared
        .ledger
        .session_stop_records(Some(&identity.execution))
        .map_err(|_| io::Error::other("stop journal unavailable"))?;
    if let Some(retained) = retained.get(&identity.execution) {
        for (index, package) in identity.packages.iter().enumerate() {
            if identity.kind == SessionStopKind::Agent && index == 0 {
                continue;
            }
            let held = records.iter().any(|record| {
                record.identity.attempt != identity.attempt
                    && !record.resources_released
                    && record.identity.packages.iter().any(|other| {
                        other.plugin_id == package.plugin_id
                            && other.plugin_version == package.plugin_version
                    })
            });
            if !held {
                retained.placement.release_mcp_package(
                    package.plugin_id.as_str(),
                    package.plugin_version.as_str(),
                )?;
            }
        }
    }
    shared
        .ledger
        .change_session_stop(&identity.attempt, SessionStopChange::ResourcesReleased)
        .map_err(|_| io::Error::other("stop journal unavailable"))
}
