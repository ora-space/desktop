//! Pack install orchestration and ownership-aware uninstall: static preflight, sequential
//! member installs, and ownership-aware member removal.
//!
//! A `kind = "pack"` listing is an orchestration entry, not a package: this module expands one
//! pack installation into member installations that reuse the ordinary single-plugin chain
//! (resolve → release selection → `Installer` download/verify/stage/commit → finalization) with
//! no new on-disk path. Every static problem is found before the first member downloads a byte
//! (extension-pack decision D5/D6): a preflight failure is a typed error with zero side effects,
//! and a member failure mid-run stops the run and is reported inside an `Ok` pack outcome rather
//! than hiding partial success. Uninstall mirrors the same discipline: a plan is computed from
//! the ownership journal and reconciliation first, then executed through the ordinary
//! single-plugin uninstall chain.

use super::PluginApi;
use super::pack_reconcile::PackMemberReconciliation;
use super::pack_uninstall::PackPreserveReason;
use crate::error::{BackendError, ErrorClassification};
use ora_application::Clock;
use ora_contracts::{
    EmptyErrorParams, InstallOutcome, InstallPluginRequest, InstallPluginResponse,
    PackInstallFailure, PackInstallationStatus, PackMemberParams, PackMemberReconciliationState,
    PackMemberStatus, PackUninstallPlan as PackUninstallPlanDto, PackUninstallPreservation,
    PackUninstallPreservationReason, PluginDataDisposition, PublicError, UninstallPluginRequest,
};
use ora_db::{PackInstallationMemberRecord, PackInstallationRecord, PackMemberOwnership};
use ora_domain::{PluginId, PluginNamespace};
use ora_logging::{ora_info, ora_warn};
use ora_plugin_manager::{
    HostTarget, Installer, PluginContribution, PluginManager, ResolvedReleaseSource, select_release,
};
use ora_plugin_manifest::{PackAgentRef, PluginKind, PluginManifest};
use ora_plugin_registry::RegistryIndex;
#[cfg(test)]
use ora_utils::http::{DownloadSource, LocalFileDownloader};
use ora_utils::http::{HttpDownload, Progress, ProgressCallback, S3Config};
use std::collections::BTreeSet;
use std::sync::Arc;

/// One applicable pack member with everything its install needs, resolved during preflight.
#[derive(Debug)]
pub(super) struct PackMemberRelease {
    pub(super) plugin_id: PluginId,
    pub(super) manifest: PluginManifest,
    pub(super) release: ResolvedReleaseSource,
}

/// The preflight result: members to install in declaration order plus members already installed.
///
/// It also carries the source the preflight resolved against, because both the member installs and
/// the ownership journal commit need that source after resolution.
#[derive(Debug)]
pub(super) struct PackPreflight {
    applicable: Vec<PackMemberRelease>,
    already_installed: Vec<String>,
    source: ora_plugin_registry::RegistrySource,
}

// The accessors exist for in-crate tests; production code in this module reads the fields
// directly because it shares the struct's module.
#[cfg(test)]
impl PackPreflight {
    /// Returns the members the install loop will install, in declaration order.
    pub(super) fn applicable(&self) -> &[PackMemberRelease] {
        &self.applicable
    }

    /// Returns the members mutably so a caller can retarget their download sources.
    pub(super) fn applicable_mut(&mut self) -> &mut Vec<PackMemberRelease> {
        &mut self.applicable
    }

    /// Returns the canonical ids of applicable members that are already installed.
    pub(super) fn already_installed(&self) -> &[String] {
        &self.already_installed
    }
}

/// What one pack install run actually did, consumed by the durable ownership journal (D3-A)
/// and by the transactional rollback when the run fails (D3-D).
///
/// On a failed run the ledger's `installed` list carries exactly the members that remain
/// installed after the rollback attempt (the residual evidence), `rollback_failed` carries
/// their canonical ids, and `skipped` is emptied so a failed attempt never mints new
/// `PreExisting` relationships.
#[derive(Debug, Default)]
pub(super) struct PackRunLedger {
    /// Members this run created: `(canonical id, version that landed)`. After a failed run
    /// with a partial rollback these are the residual members.
    installed: Vec<(String, String)>,
    /// Applicable members that were already installed and therefore skipped. Emptied on failed
    /// runs: the journal must not gain new `PreExisting` relationships from an attempt that
    /// did not complete.
    skipped: Vec<String>,
    /// Canonical ids of created members whose rollback failed (D3-D).
    rollback_failed: Vec<String>,
}

// The accessors exist for in-crate tests; production code in this module reads the fields
// directly because it shares the struct's module.
#[cfg(test)]
impl PackRunLedger {
    /// Returns the canonical ids of created members whose rollback failed.
    pub(super) fn rollback_failed(&self) -> &[String] {
        &self.rollback_failed
    }

    /// Returns the members this run created that remain installed.
    pub(super) fn installed(&self) -> &[(String, String)] {
        &self.installed
    }
}

/// Maps one reconciliation classification onto the frontend member status DTO.
fn member_status(
    record: &PackInstallationRecord,
    member: &PackMemberReconciliation,
) -> PackMemberStatus {
    let journal_member = record
        .members
        .iter()
        .find(|journal| journal.member_id == member_member_id(member));
    let ownership = member_ownership(match journal_member {
        Some(journal) => journal.ownership,
        None => ora_db::PackMemberOwnership::ManagedByPack,
    });
    match member {
        PackMemberReconciliation::ExpectedAndPresent { .. } => PackMemberStatus {
            member_id: member_member_id(member).to_owned(),
            version_at_install: journal_member
                .map(|journal| journal.version_at_install.clone())
                .unwrap_or_default(),
            ownership,
            state: PackMemberReconciliationState::ExpectedAndPresent,
        },
        PackMemberReconciliation::VersionChanged {
            current_version, ..
        } => PackMemberStatus {
            member_id: member_member_id(member).to_owned(),
            version_at_install: journal_member
                .map(|journal| journal.version_at_install.clone())
                .unwrap_or_default(),
            ownership,
            state: PackMemberReconciliationState::VersionChanged {
                current_version: current_version.clone(),
            },
        },
        PackMemberReconciliation::Missing { .. } => PackMemberStatus {
            member_id: member_member_id(member).to_owned(),
            version_at_install: journal_member
                .map(|journal| journal.version_at_install.clone())
                .unwrap_or_default(),
            ownership,
            state: PackMemberReconciliationState::Missing,
        },
    }
}

/// Maps the journal ownership onto the contract enum.
fn member_ownership(ownership: ora_db::PackMemberOwnership) -> ora_contracts::PackMemberOwnership {
    match ownership {
        ora_db::PackMemberOwnership::ManagedByPack => {
            ora_contracts::PackMemberOwnership::ManagedByPack
        }
        ora_db::PackMemberOwnership::PreExisting => ora_contracts::PackMemberOwnership::PreExisting,
    }
}

/// Reads the member id off any reconciliation classification.
fn member_member_id(member: &PackMemberReconciliation) -> &str {
    match member {
        PackMemberReconciliation::ExpectedAndPresent { member_id, .. }
        | PackMemberReconciliation::VersionChanged { member_id, .. }
        | PackMemberReconciliation::Missing { member_id, .. } => member_id,
    }
}

impl PluginApi {
    /// Installs one `kind = "pack"` listing: preflight the membership, then install every
    /// applicable member in declaration order through the ordinary single-plugin chain.
    ///
    /// The response always carries the pack's own id; the outcome distinguishes installed
    /// members, skipped members, and the first failure.
    pub(super) async fn install_pack(
        &self,
        request: InstallPluginRequest,
        manifest: PluginManifest,
        namespace: PluginNamespace,
        use_proxy: bool,
        s3_config: Option<S3Config>,
        progress: Option<ProgressCallback>,
    ) -> Result<InstallPluginResponse, BackendError> {
        let preflight = self.preflight_pack(
            &manifest,
            &namespace,
            &self.owning_registry_source(&namespace).await?,
        )?;
        let installer = self.marketplace_installer(use_proxy, s3_config).await?;
        self.finish_pack_install(
            request, manifest, namespace, preflight, &installer, progress,
        )
        .await
    }

    /// Runs pack installation through the production orchestration with local transfer sources.
    ///
    /// This exists only for offline qualification tests: registry resolution, preflight, digest
    /// verification, member finalization, rollback, and ownership recording are unchanged.
    #[cfg(test)]
    pub(super) async fn install_pack_from_local_releases(
        &self,
        request: InstallPluginRequest,
        manifest: PluginManifest,
        namespace: PluginNamespace,
        progress: Option<ProgressCallback>,
    ) -> Result<InstallPluginResponse, BackendError> {
        let mut preflight = self.preflight_pack(
            &manifest,
            &namespace,
            &self.owning_registry_source(&namespace).await?,
        )?;
        for member in preflight.applicable_mut() {
            let Some(artifact) = self.local_marketplace_release(&member.plugin_id) else {
                continue;
            };
            let digest = *member.release.sha256();
            member.release = match member.release.target().cloned() {
                Some(target) => {
                    ResolvedReleaseSource::targeted(DownloadSource::Local(artifact), digest, target)
                }
                None => ResolvedReleaseSource::universal(DownloadSource::Local(artifact), digest),
            };
        }
        self.finish_pack_install(
            request,
            manifest,
            namespace,
            preflight,
            &Installer::new(LocalFileDownloader),
            progress,
        )
        .await
    }

    /// Completes an already-resolved pack install and commits only its resulting ownership facts.
    async fn finish_pack_install<D>(
        &self,
        request: InstallPluginRequest,
        manifest: PluginManifest,
        namespace: PluginNamespace,
        preflight: PackPreflight,
        installer: &Installer<D>,
        progress: Option<ProgressCallback>,
    ) -> Result<InstallPluginResponse, BackendError>
    where
        D: HttpDownload,
    {
        // The member run consumes the preflight, so the source attribution for the ownership
        // journal is captured from it first.
        let source_url = preflight.source.canonical_url().to_owned();
        let (outcome, ledger) = self
            .install_members(&namespace, preflight, installer, progress)
            .await?;
        let pack_id = self.pack_member_id(&namespace, manifest.name().as_str())?;
        match &outcome {
            // A run whose rollback completed restores the journal to its pre-run facts:
            // record_pack_run never ran for this attempt, prior relationships are untouched,
            // and recording now would only mint relations the filesystem no longer backs.
            InstallOutcome::PackInstalled {
                failed: Some(failure),
                ..
            } if failure.rollback_failures.is_empty() => {
                ora_info!(
                    plugin_id = %request.plugin_id,
                    failure_member = %failure.plugin_id,
                    "pack install failed and fully rolled back; ownership journal left as before the run"
                );
            }
            // A complete run records ownership; a run whose rollback failed records its
            // residual members as the durable recovery evidence (D3-D).
            _ => {
                self.record_pack_run(&pack_id, &source_url, &ledger)?;
            }
        }
        ora_info!(plugin_id = %request.plugin_id, outcome = ?outcome, "installed marketplace pack");
        Ok(InstallPluginResponse {
            plugin_id: request.plugin_id,
            outcome,
        })
    }

    /// Runs every static pack check before any member downloads a byte.
    ///
    /// The checks follow the decision's order so the first error is the most actionable one:
    /// duplicate members, self-reference, per-member existence (resolved inside the pack's own
    /// source checkout), nesting, agent filtering, host compatibility, and the already-installed
    /// split. Failure is a typed public error; success carries the resolved per-member state the
    /// install loop consumes without re-reading anything.
    pub(super) fn preflight_pack(
        &self,
        pack: &PluginManifest,
        namespace: &PluginNamespace,
        source: &ora_plugin_registry::RegistrySource,
    ) -> Result<PackPreflight, BackendError> {
        let pack_name = pack.name().as_str();
        let members = pack.pack().ok_or_else(|| {
            BackendError::internal(
                "pack manifest carries no membership",
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    pack.name().as_str().to_string(),
                ),
            )
        })?;

        let mut seen = BTreeSet::new();
        let mut declared = Vec::new();
        for member in members.members() {
            let member_id = self.pack_member_id(namespace, member.identifier().as_str())?;
            if !seen.insert(member.identifier().as_str().to_owned()) {
                return Err(self.pack_member_error(PublicError::PackMemberDuplicate, &member_id));
            }
            if member.identifier().as_str() == pack_name {
                return Err(self.pack_member_error(PublicError::PackSelfReference, &member_id));
            }
            // Members resolve inside the pack's own source checkout (decision D2): the member
            // identifier is a bare name, so only the pack's repository can give it an identity.
            let member_manifest =
                RegistryIndex::resolve_manifest(source, &member_id).map_err(|error| {
                    BackendError::internal("failed to resolve pack member manifest", error)
                })?;
            let member_manifest = member_manifest.ok_or_else(|| {
                self.pack_member_error(PublicError::PackMemberNotFound, &member_id)
            })?;
            // V1 forbids nested packs: a member that is itself a pack would turn installation
            // into an unbounded traversal that the decision explicitly leaves to a later ADR.
            if matches!(member_manifest.kind(), PluginKind::Pack) {
                return Err(self.pack_member_error(PublicError::PackMemberNested, &member_id));
            }
            declared.push((member_id, member.agents().to_vec(), member_manifest));
        }

        // Agent filtering is a one-shot listing-time decision (decision D4): a member with agent
        // references belongs to this installation only when some installed agent plugin matches
        // one reference. An unmatched reference is not an error; it only thins the set.
        let installed = PluginManager::discover(&self.home_directory);
        let installed_agents = installed
            .installed_plugins()
            .iter()
            .filter(|plugin| matches!(plugin.contributes, PluginContribution::Agent(_)))
            .map(|plugin| plugin.id.clone())
            .collect::<Vec<_>>();
        let selected = declared
            .into_iter()
            .filter(|(_member_id, agents, _manifest)| {
                agents.is_empty()
                    || agents.iter().any(|reference| {
                        installed_agents.iter().any(|agent_id| match reference {
                            PackAgentRef::Bare(name) => agent_id.name() == name.as_str(),
                            PackAgentRef::Canonical(canonical) => {
                                agent_id.canonical() == canonical.canonical()
                            }
                        })
                    })
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err(BackendError::new(
                ErrorClassification::Unprocessable,
                PublicError::PackNoApplicableMembers(PackMemberParams {
                    plugin_id: format!("{namespace}/{}", pack.name().as_str()),
                }),
                "pack has no member that applies to this installation",
            ));
        }

        // Host compatibility is checked here rather than mid-run, so an incompatible member
        // fails the whole pack before the first member lands (decision D6).
        let host_target = ora_plugin_registry::current_host_target();
        let host = HostTarget::from_option(host_target.as_ref());
        let mut already_installed = Vec::new();
        let mut applicable = Vec::new();
        for (member_id, _agents, member_manifest) in selected {
            let release = select_release(&member_manifest, host).map_err(|error| {
                self.map_install_error("failed to select pack member release", error)
            })?;
            if installed
                .installed_plugins()
                .iter()
                .any(|plugin| plugin.id.canonical() == member_id.canonical())
            {
                already_installed.push(member_id.canonical());
            } else {
                applicable.push(PackMemberRelease {
                    plugin_id: member_id,
                    manifest: member_manifest,
                    release,
                });
            }
        }
        Ok(PackPreflight {
            applicable,
            already_installed,
            source: source.clone(),
        })
    }

    /// Installs the applicable members in declaration order, skipping members that were installed
    /// between preflight and their turn, and reporting the first member failure inside the
    /// returned outcome instead of rolling back members that already landed.
    pub(super) async fn install_members<D>(
        &self,
        namespace: &PluginNamespace,
        preflight: PackPreflight,
        installer: &Installer<D>,
        progress: Option<ProgressCallback>,
    ) -> Result<(InstallOutcome, PackRunLedger), BackendError>
    where
        D: HttpDownload,
    {
        let member_total = preflight.applicable.len();
        // The pack forwards one aggregated progress stream under its own id: each member's inner
        // byte progress is scaled into its share of the member count.
        let member_scale: u64 = 1_000_000;
        let mut members = Vec::new();
        let mut skipped = preflight.already_installed;
        // The ledger records what this run actually did, for the durable ownership journal.
        let mut ledger = PackRunLedger::default();
        for (index, member) in preflight.applicable.into_iter().enumerate() {
            // Re-checked at install time: a concurrent install between preflight and this member's
            // turn makes it a skip rather than a duplicate directory (invariant 5).
            if self.member_is_installed(&member.plugin_id) {
                skipped.push(member.plugin_id.canonical());
                continue;
            }
            let member_version = member.manifest.version().to_string();
            let member_progress: Option<ProgressCallback> = progress.as_ref().map(|progress| {
                let progress = Arc::clone(progress);
                Arc::new(move |inner: Progress| {
                    let fraction = inner
                        .total
                        .map(|total| inner.bytes as f64 / total as f64)
                        .unwrap_or(0.0);
                    progress(Progress {
                        bytes: ((index as f64 + fraction) * member_scale as f64) as u64,
                        total: Some(member_total as u64 * member_scale),
                    });
                }) as ProgressCallback
            });
            let install_result = match member_progress {
                Some(member_progress) => {
                    installer
                        .install_with_progress(
                            &member.manifest,
                            namespace,
                            member.release,
                            &self.home_directory,
                            member_progress,
                        )
                        .await
                }
                None => {
                    installer
                        .install(
                            &member.manifest,
                            namespace,
                            member.release,
                            &self.home_directory,
                        )
                        .await
                }
            };
            if let Err(error) = install_result {
                let mapped = self.map_install_error("failed to install pack member", error);
                ora_info!(
                    plugin_id = %member.plugin_id.canonical(),
                    error = %mapped,
                    "pack member installation failed; remaining members are not attempted"
                );
                // Transactional rollback (D3-D): undo the members this run created, in reverse
                // creation order, before reporting the failure. The rollback reuses the
                // ordinary single-plugin uninstall chain and never touches skipped members.
                let created = std::mem::take(&mut ledger.installed);
                let (residual, rollback_failures) = self.rollback_created_members(created).await;
                ledger.rollback_failed = residual
                    .iter()
                    .map(|(member_id, _version)| member_id.clone())
                    .collect();
                // A failed run must not mint new PreExisting relationships, so the journal
                // ledger's skip list is emptied; the outcome still reports the real skips.
                ledger.skipped = Vec::new();
                // The residual members are the durable recovery evidence: the journal keeps
                // them as pack-managed (D3-D).
                ledger.installed = residual.clone();
                let members = residual
                    .iter()
                    .map(|(member_id, _version)| member_id.clone())
                    .collect::<Vec<_>>();
                return Ok((
                    InstallOutcome::PackInstalled {
                        members,
                        skipped,
                        failed: Some(PackInstallFailure {
                            plugin_id: member.plugin_id.canonical(),
                            error_code: mapped.public_error().code().to_owned(),
                            rollback_failures,
                        }),
                    },
                    ledger,
                ));
            }
            // Finalize exactly like a single-plugin install so Skills, the installed snapshot,
            // and MCP desired state see the member immediately; a finalization failure is that
            // member's failure and stops the run on the same terms as a download failure.
            // Finalization only lands the package: a member's Hook lifecycle commands wait for the
            // user to authorize that Hook's own initialization.
            if let Err(error) = self
                .finalize_new_install(&member.plugin_id.canonical())
                .await
            {
                let created = std::mem::take(&mut ledger.installed);
                let (residual, rollback_failures) = self.rollback_created_members(created).await;
                ledger.rollback_failed = residual
                    .iter()
                    .map(|(member_id, _version)| member_id.clone())
                    .collect();
                // A failed run must not mint new PreExisting relationships, so the journal
                // ledger's skip list is emptied; the outcome still reports the real skips.
                ledger.skipped = Vec::new();
                // The residual members are the durable recovery evidence: the journal
                // keeps them as pack-managed (D3-D).
                ledger.installed = residual.clone();
                let members = residual
                    .iter()
                    .map(|(member_id, _version)| member_id.clone())
                    .collect::<Vec<_>>();
                return Ok((
                    InstallOutcome::PackInstalled {
                        members,
                        skipped,
                        failed: Some(PackInstallFailure {
                            plugin_id: member.plugin_id.canonical(),
                            error_code: error.public_error().code().to_owned(),
                            rollback_failures,
                        }),
                    },
                    ledger,
                ));
            }
            // This run created the member, so the durable relationship records it as
            // pack-managed (D3-A).
            ledger
                .installed
                .push((member.plugin_id.canonical(), member_version));
            members.push(member.plugin_id.canonical());
        }
        ledger.skipped = skipped.clone();
        Ok((
            InstallOutcome::PackInstalled {
                members,
                skipped,
                failed: None,
            },
            ledger,
        ))
    }

    /// Rolls back the members this run created, in reverse creation order, through the ordinary
    /// single-plugin uninstall chain (stop → supervisor → uninstall → data cleanup).
    ///
    /// The rollback stops at the first member whose uninstall fails: that member and any
    /// earlier-created members remain installed and are returned as the residual evidence.
    /// Pre-existing members are never passed here, so a rollback can never touch user assets.
    async fn rollback_created_members(
        &self,
        created: Vec<(String, String)>,
    ) -> (
        Vec<(String, String)>,
        Vec<ora_contracts::PackRollbackFailure>,
    ) {
        // `pending` keeps creation order, so `pop` yields the most recently created member
        // first: the rollback runs strictly in reverse creation order.
        let mut pending = created;
        let mut residual = Vec::new();
        let mut failures = Vec::new();
        while let Some((member_id, version)) = pending.pop() {
            let result = self
                .uninstall(UninstallPluginRequest {
                    hook_execution_acknowledged: false,
                    plugin_id: member_id.clone(),
                    data_disposition: PluginDataDisposition::Delete,
                })
                .await;
            if let Err(error) = result {
                ora_warn!(
                    plugin_id = %member_id,
                    error = %error,
                    "pack member rollback failed; earlier members stay installed as residual"
                );
                failures.push(ora_contracts::PackRollbackFailure {
                    plugin_id: member_id.clone(),
                    error_code: error.public_error().code().to_owned(),
                });
                // The failed member and every member the rollback did not reach remain
                // installed: they are the residual state the journal must record.
                residual.push((member_id, version));
                residual.append(&mut pending);
                break;
            }
        }
        (residual, failures)
    }

    /// Assembles the canonical member id a bare pack member identifier resolves to inside the
    /// pack's own namespace.
    fn pack_member_id(
        &self,
        namespace: &PluginNamespace,
        identifier: &str,
    ) -> Result<PluginId, BackendError> {
        PluginId::new(namespace.clone(), identifier).map_err(|error| {
            BackendError::internal(
                "pack member identifier is not representable as a plugin id",
                error,
            )
        })
    }

    /// Builds the typed preflight failure for one member.
    fn pack_member_error(
        &self,
        public_error: impl FnOnce(PackMemberParams) -> PublicError,
        member_id: &PluginId,
    ) -> BackendError {
        BackendError::new(
            ErrorClassification::Unprocessable,
            public_error(PackMemberParams {
                plugin_id: member_id.canonical(),
            }),
            format!("pack member {} failed preflight", member_id.canonical()),
        )
    }

    /// Projects every recorded pack installation with its reconciled member states for the
    /// frontend's installed-packs presentation (D4).
    pub(super) fn list_pack_installations(
        &self,
    ) -> Result<Vec<PackInstallationStatus>, BackendError> {
        let records = self
            .pack_installations
            .list()
            .map_err(|error| BackendError::internal("failed to list pack installations", error))?;
        records
            .iter()
            .map(|record| {
                let reconciliation = self.reconcile_pack_installation(&record.pack_id)?;
                let members = reconciliation
                    .map(|reconciled| {
                        reconciled
                            .members()
                            .iter()
                            .map(|member| member_status(record, member))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|| {
                        // The journal exists but the recorded member is gone from the
                        // installed tree: reconcile honestly as missing (D3-B).
                        record
                            .members
                            .iter()
                            .map(|member| PackMemberStatus {
                                member_id: member.member_id.clone(),
                                version_at_install: member.version_at_install.clone(),
                                ownership: member_ownership(member.ownership),
                                state: PackMemberReconciliationState::Missing,
                            })
                            .collect::<Vec<_>>()
                    });
                Ok(PackInstallationStatus {
                    pack_id: record.pack_id.clone(),
                    source_url: record.source_url.clone(),
                    members,
                })
            })
            .collect()
    }

    /// Projects the ownership-aware uninstall plan for one pack id, or `None` when the id has
    /// no ownership journal (an ordinary single-plugin uninstall then applies).
    pub(super) fn pack_uninstall_plan(
        &self,
        pack_id: &str,
    ) -> Result<Option<PackUninstallPlanDto>, BackendError> {
        let Some(plan) = self.pack_uninstall_plan_internal(pack_id)? else {
            return Ok(None);
        };
        Ok(Some(PackUninstallPlanDto {
            remove: plan.remove,
            preserve: plan
                .preserve
                .into_iter()
                .map(|(member_id, reason)| PackUninstallPreservation {
                    member_id,
                    reason: match reason {
                        PackPreserveReason::PreExisting => {
                            PackUninstallPreservationReason::PreExisting
                        }
                        PackPreserveReason::VersionChanged => {
                            PackUninstallPreservationReason::VersionChanged
                        }
                    },
                })
                .collect(),
            already_missing: plan.already_missing,
        }))
    }

    /// Returns whether `member_id` is currently installed, by rescanning the package tree.
    ///
    /// Discovery is the authority for installation state; the cached lifecycle snapshot can lag a
    /// concurrent operation, and a stale skip reads as an install error rather than a silent
    /// downgrade.
    fn member_is_installed(&self, member_id: &PluginId) -> bool {
        PluginManager::discover(&self.home_directory)
            .installed_plugins()
            .iter()
            .any(|plugin| plugin.id.canonical() == member_id.canonical())
    }

    /// Persists what one pack install run did into the durable ownership journal.
    ///
    /// Members the run created are recorded `ManagedByPack` at the version that landed. Skipped
    /// members keep the relationship they already have; one the pack never installed is recorded
    /// `PreExisting` at its current version. A member that failed mid-run is left absent so a
    /// later pack install that lands it extends the record.
    pub(super) fn record_pack_run(
        &self,
        pack_id: &PluginId,
        source_url: &str,
        ledger: &PackRunLedger,
    ) -> Result<(), BackendError> {
        let canonical = pack_id.canonical();
        let now = self.clock.now_timestamp_millis();
        self.pack_installations
            .upsert_pack(&canonical, source_url, now)
            .map_err(|error| {
                BackendError::internal("failed to record the pack installation", error)
            })?;
        for (member_id, version) in &ledger.installed {
            self.pack_installations
                .upsert_member(
                    &canonical,
                    &PackInstallationMemberRecord {
                        member_id: member_id.clone(),
                        version_at_install: version.clone(),
                        ownership: PackMemberOwnership::ManagedByPack,
                    },
                    now,
                )
                .map_err(|error| {
                    BackendError::internal("failed to record the pack member ownership", error)
                })?;
        }
        for member_id in &ledger.skipped {
            // A member the pack named but never touched keeps the relationship it already has;
            // only a first-time skip becomes a `PreExisting` relationship.
            if self
                .pack_installations
                .load_member(&canonical, member_id)
                .map_err(|error| {
                    BackendError::internal("failed to load the pack member relationship", error)
                })?
                .is_some()
            {
                continue;
            }
            let version = PluginManager::discover(&self.home_directory)
                .installed_plugins()
                .iter()
                .find(|plugin| plugin.id.canonical() == *member_id)
                .map(|plugin| plugin.version.to_string())
                .ok_or_else(|| {
                    BackendError::internal(
                        "skipped pack member disappeared before its ownership was recorded",
                        std::io::Error::new(std::io::ErrorKind::NotFound, member_id.clone()),
                    )
                })?;
            self.pack_installations
                .upsert_member(
                    &canonical,
                    &PackInstallationMemberRecord {
                        member_id: member_id.clone(),
                        version_at_install: version,
                        ownership: PackMemberOwnership::PreExisting,
                    },
                    now,
                )
                .map_err(|error| {
                    BackendError::internal("failed to record the pack member relationship", error)
                })?;
        }
        Ok(())
    }

    /// Loads one recorded pack installation with its member relationships.
    ///
    /// This is the read side of the ownership journal that restart reconciliation (D3-B) and
    /// pack uninstall (D3-C) build on.
    pub(super) fn pack_installation(
        &self,
        pack_id: &str,
    ) -> Result<Option<PackInstallationRecord>, BackendError> {
        self.pack_installations.load(pack_id).map_err(|error| {
            BackendError::internal("failed to load pack installation record", error)
        })
    }

    /// Resolves the one marketplace source whose namespace owns `namespace`.
    ///
    /// Members always resolve inside the pack's own source (decision D2), so the orchestrator
    /// needs that source's checkout and proxy policy rather than the whole source list.
    async fn owning_registry_source(
        &self,
        namespace: &PluginNamespace,
    ) -> Result<ora_plugin_registry::RegistrySource, BackendError> {
        let proxy_settings = self.settings.network_proxy_settings().await?;
        let registry_sources = self.prepared_registry_sources(proxy_settings)?;
        registry_sources
            .into_iter()
            .find(|(source, _use_proxy, _s3_config)| {
                source.namespace().as_str() == namespace.as_str()
            })
            .map(|(source, _use_proxy, _s3_config)| source)
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::PluginNotFound(EmptyErrorParams {}),
                    format!("no marketplace source owns the namespace {namespace}"),
                )
            })
    }
}
