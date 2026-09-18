//! Ownership-aware pack uninstall: plan computation and execution.
//!
//! The plan is computed from the ownership journal plus reconciliation before anything is
//! removed: only members the pack created at the version it recorded are removable, everything
//! else is preserved with a structured reason so a pack uninstall can never touch user assets.
//! Execution reuses the ordinary single-plugin uninstall chain per member and releases journal
//! relationships one at a time so a mid-run failure leaves a retryable state.

use super::PluginApi;
use super::pack_reconcile::PackMemberReconciliation;
use crate::error::BackendError;
use ora_contracts::{PluginDataDisposition, UninstallPluginRequest, UninstallPluginResponse};
use ora_db::PackMemberOwnership;
use ora_logging::ora_info;

/// Why a pack uninstall preserves a member instead of removing it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PackPreserveReason {
    /// The member was already installed when the pack named it; the pack never created it.
    PreExisting,
    /// The pack created the member, but it has since been independently changed.
    VersionChanged,
}

/// The computed, not-yet-executed ownership-aware uninstall plan for one recorded pack.
///
/// Only removable member ids drive filesystem work; preserved and already-missing members are
/// listed so callers can suspend, resume, and present the full member set without re-deriving
/// the plan.
#[derive(Debug)]
pub(crate) struct PackUninstallPlanInternal {
    pub(crate) remove: Vec<String>,
    pub(crate) preserve: Vec<(String, PackPreserveReason)>,
    pub(crate) already_missing: Vec<String>,
}

impl PluginApi {
    /// Computes the ownership-aware uninstall plan for one recorded pack, or `None` when the id
    /// carries no ownership journal (an ordinary single-plugin uninstall then applies).
    ///
    /// Only members the pack created at the version it recorded are removable; everything else
    /// is preserved with the reason, so a pack uninstall can never touch user assets.
    pub(crate) fn pack_uninstall_plan_internal(
        &self,
        pack_id: &str,
    ) -> Result<Option<PackUninstallPlanInternal>, BackendError> {
        let Some(reconciliation) = self.reconcile_pack_installation(pack_id)? else {
            return Ok(None);
        };
        let mut plan = PackUninstallPlanInternal {
            remove: Vec::new(),
            preserve: Vec::new(),
            already_missing: Vec::new(),
        };
        for member in reconciliation.members() {
            match member {
                PackMemberReconciliation::ExpectedAndPresent {
                    member_id,
                    ownership: PackMemberOwnership::ManagedByPack,
                    ..
                } => plan.remove.push(member_id.clone()),
                PackMemberReconciliation::ExpectedAndPresent {
                    member_id,
                    ownership: PackMemberOwnership::PreExisting,
                    ..
                } => plan
                    .preserve
                    .push((member_id.clone(), PackPreserveReason::PreExisting)),
                // A version change means the user took over the member: the classification is
                // reported, the member is kept, and its ownership is never re-derived.
                PackMemberReconciliation::VersionChanged { member_id, .. } => plan
                    .preserve
                    .push((member_id.clone(), PackPreserveReason::VersionChanged)),
                PackMemberReconciliation::Missing {
                    member_id,
                    ownership: PackMemberOwnership::ManagedByPack,
                    ..
                } => plan.already_missing.push(member_id.clone()),
                PackMemberReconciliation::Missing {
                    member_id,
                    ownership: PackMemberOwnership::PreExisting,
                    ..
                } => plan
                    .preserve
                    .push((member_id.clone(), PackPreserveReason::PreExisting)),
            }
        }
        Ok(Some(plan))
    }

    /// Executes an ownership-aware pack uninstall: removable members go through the ordinary
    /// single-plugin uninstall chain, preserved members keep their packages, and the ownership
    /// journal releases one relationship at a time so a mid-run failure leaves a retryable state.
    ///
    /// The pack root record is deleted only after every journal relationship has been released;
    /// a member uninstall that fails keeps its journal row and stops the run, and a retry
    /// re-plans from the surviving journal.
    pub(crate) async fn uninstall_pack(
        &self,
        pack_id: &str,
        data_disposition: PluginDataDisposition,
    ) -> Result<UninstallPluginResponse, BackendError> {
        let plan = self.pack_uninstall_plan_internal(pack_id)?;
        let Some(plan) = plan else {
            return Ok(UninstallPluginResponse {
                plugin_id: pack_id.to_owned(),
            });
        };
        for member_id in plan.remove {
            let result = self
                .uninstall(UninstallPluginRequest {
                    hook_execution_acknowledged: false,
                    plugin_id: member_id.clone(),
                    data_disposition,
                })
                .await;
            if let Err(error) = result {
                // The member keeps its journal row: the next uninstall re-plans and continues
                // from exactly this member.
                return Err(BackendError::new(
                    error.classification(),
                    error.public_error().clone(),
                    format!("pack member {member_id} could not be uninstalled: {error}"),
                ));
            }
            self.pack_installations
                .remove_member(pack_id, &member_id)
                .map_err(|error| {
                    BackendError::internal(
                        "failed to release the removed member relationship",
                        error,
                    )
                })?;
        }
        // Preserved members and already-absent members leave the journal deliberately: the pack
        // uninstall dissolves the relationship without touching their packages.
        for (member_id, _reason) in plan.preserve {
            self.pack_installations
                .remove_member(pack_id, &member_id)
                .map_err(|error| {
                    BackendError::internal(
                        "failed to release the preserved member relationship",
                        error,
                    )
                })?;
        }
        for member_id in plan.already_missing {
            self.pack_installations
                .remove_member(pack_id, &member_id)
                .map_err(|error| {
                    BackendError::internal(
                        "failed to release the already-missing member relationship",
                        error,
                    )
                })?;
        }
        // The root record goes only when no relationship remains; a failed member keeps the
        // journal alive for the retry.
        let remaining = self
            .pack_installations
            .load(pack_id)
            .map_err(|error| BackendError::internal("failed to load the pack journal", error))?;
        if remaining.is_none_or(|record| record.members.is_empty()) {
            self.pack_installations.remove(pack_id).map_err(|error| {
                BackendError::internal("failed to remove the pack journal", error)
            })?;
        }
        ora_info!(plugin_id = %pack_id, "uninstalled marketplace pack");
        Ok(UninstallPluginResponse {
            plugin_id: pack_id.to_owned(),
        })
    }
}
