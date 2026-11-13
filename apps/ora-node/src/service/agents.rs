//! Production composition and durable admission of Agent sessions.
use super::{AgentConfig, ServiceConfig};
use crate::managed::{AsNode, DirectGit};
use crate::revision::{DeliveryGit, RevisionRestorer};
use crate::{
    AgentSessions, DirectoryPluginCatalog, ManagedNode, ProcessConfig, SessionConfig,
    SessionHost as _, SessionWorkload,
};
use ora_node_db::{CommandAdmission, SessionCommandInput, SessionJournal};
use ora_node_protocol::*;
use ora_process::ProcessIdentity;
use ora_utils::path::{TrustedPathKind, canonicalize_longest_existing_prefix, open_trusted_path};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Component;
use std::time::Duration;

/// Restores prior Revisions with delivery's Git and Node-identity Git Server access.
pub(super) type Restorer = RevisionRestorer<DirectGit, AsNode<DirectGit>>;

pub(super) type SessionHost =
    AgentSessions<SessionJournal, SessionJournal, DirectoryPluginCatalog, Option<Restorer>>;

/// Retains original restore inputs until handover, then composes restore when this Node can run
/// both sessions and delivery Git: restore runs Git in the checkouts clone created, with
/// delivery's policy, and fetches from the Git Server with the git-access identity, so without
/// clone and git-access configuration there is nothing to restore.
pub(super) fn restorer(
    node: &ManagedNode,
    agent: Option<&AgentConfig>,
    shutdown: crate::Shutdown,
    restore_policy: crate::revision::RestorePolicy,
    stops: &crate::managed::RestoreStops,
) -> Result<Option<Restorer>, crate::Error> {
    let (Some(_), Some(policy), Some(access), Some(config)) = (
        agent,
        node.delivery_git_policy()?,
        node.git_server(),
        node.clone_config(),
    ) else {
        return Ok(None);
    };
    let git = || {
        node.git
            .runner()
            .detached_git()
            .map_err(ora_node_db::Error::from)
    };
    let mut runner = git()?;
    // Every restore command is recorded as an attempt before it is dispatched, whether it
    // runs as the workload user in the checkout or as the Node for Git Server access.
    runner.track_restore(stops.clone());
    let mut node_runner = git()?;
    node_runner.track_restore(stops.clone());
    Ok(Some(
        RevisionRestorer::new(
            DeliveryGit::new(runner, policy),
            AsNode(node_runner),
            access,
            config.repository_root.clone(),
            node.git.runner().process_config().workload_uid,
            shutdown,
            restore_policy,
        )
        .with_stops(stops.clone()),
    ))
}

/// Crash handover and late queries never rerun restore or invent a second Agent identity.
pub(super) async fn reconcile(
    node: &mut ManagedNode,
    host: Option<&SessionHost>,
    stops: &crate::managed::RestoreStops,
    check: crate::managed::StopCheck,
    only: Option<&ora_node_protocol::ExecutionId>,
) -> Result<(), crate::Error> {
    let journal = &stops.journal;
    for record in node.database.recoverable_sessions()? {
        let execution = &record.command.execution_id;
        // An execution-status query proves only the execution it names, so it never pays for
        // every other pending stop bound on the worker thread.
        if only.is_some_and(|only| only != execution)
            || host.is_some_and(|host| host.is_live(execution))
        {
            continue;
        }
        let phase = journal.restore_phase(execution)?;
        if phase == Some(ora_node_db::RestorePhase::LegacyUnknown) {
            continue;
        }
        let ended = journal
            .deferred_restore_end(execution)?
            .unwrap_or(AgentSessionEnded {
                node: node.identity().clone(),
                reason: AgentSessionEndReason::Interrupted,
                detail: None,
                scheduled_result: None,
                artifact_manifest: None,
            });
        // A failed ledger read defers this record only; the next pass retries it.
        if let Err(error) = journal.defer_restore_end(execution, &ended) {
            ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore end could not be deferred");
            continue;
        }
        let confirmed = match stops.poll(execution, check).await {
            Ok(confirmed) => confirmed,
            Err(error) => {
                ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore stop evidence is unavailable");
                continue;
            }
        };
        if !confirmed {
            continue;
        }
        if phase.is_some()
            && let Err(detail) = crate::session::seal_unstarted_history(
                &node.home_directory().join("sessions"),
                execution,
            )
        {
            ora_logging::ora_warn!(execution_id = %execution, error = %detail, "unstarted history could not be sealed");
            continue;
        }
        if let Err(error) = journal.end_session(execution, ended) {
            ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore session could not be ended");
            continue;
        }
        cleanup_restore(node, journal, &record)?;
    }
    Ok(())
}

/// Startup cleanup consumes retained stop facts; unassociated or unknown directories stay intact.
fn sweep_restores(node: &ManagedNode, journal: &SessionJournal) -> Result<(), crate::Error> {
    for record in node.database.restore_sessions()? {
        cleanup_restore(node, journal, &record)?;
    }
    Ok(())
}

/// Live late-proof settlement and startup retry consume the same original safety facts.
fn cleanup_restore(
    node: &ManagedNode,
    journal: &SessionJournal,
    record: &ora_node_db::SessionExecution,
) -> Result<(), crate::Error> {
    let execution = &record.command.execution_id;
    if !journal.session_completed(execution)?
        || journal.restore_phase(execution)? == Some(ora_node_db::RestorePhase::LegacyUnknown)
        || !journal.pending_restore_attempts(execution)?.is_empty()
    {
        return Ok(());
    }
    // The transport repository may still be read by an unconfirmed original process.
    if let Some(config) = node.clone_config() {
        let transport = crate::git_server::TransportRepository::location(
            &config.repository_root,
            crate::revision::RESTORE_TRANSPORT_KIND,
            execution,
        );
        crate::git_server::remove_transport(&transport);
    }
    // Inputs of a restore started before the Git Server switch are retained until the same proof.
    let legacy = node
        .home_directory()
        .join("revision-restores")
        .join(ora_utils::hash::sha256_hex(execution.as_str().as_bytes()));
    if let Err(error) = std::fs::remove_dir_all(&legacy)
        && error.kind() != io::ErrorKind::NotFound
    {
        ora_logging::ora_warn!(execution_id = %execution, error = %error, "confirmed restore input cleanup failed");
    }
    Ok(())
}

/// Checks the Agent configuration before anything starts.
///
/// Agents run as the workload user exactly when Git workloads do: a Node that separates Git from
/// itself but ran agents as root would hand them its secrets and a checkout Git refuses as
/// dubiously owned, and a workload directory without a workload user would have no owner to
/// serve. The directory must already exist as the deployment's: owned by the Node's own identity
/// (root in a sandbox) and writable by nobody else, because the Node creates, links and removes
/// trees inside it with that authority. It must also not overlap any state or checkout root, so
/// purging it can never reach them. The `ora-git` directory is checked by [`validate_ora_git`].
pub(super) fn validate(agent: &AgentConfig, config: &ServiceConfig) -> io::Result<()> {
    if !agent.deno_path.is_absolute() || agent.ready_timeout_ms == 0 {
        return Err(io::Error::other(
            "agent needs an absolute Deno path and a positive ready timeout",
        ));
    }
    validate_workload(agent, config)?;
    validate_ora_git(agent, config)
}

fn validate_workload(agent: &AgentConfig, config: &ServiceConfig) -> io::Result<()> {
    let directory = match (&agent.workload_directory, config.process.workload_uid) {
        (None, None) => return Ok(()),
        (Some(directory), Some(_)) => directory,
        (Some(_), None) | (None, Some(_)) => {
            return Err(io::Error::other(
                "agent workload_directory is required exactly when process.workload_uid is set",
            ));
        }
    };
    if !directory.is_absolute()
        || directory.to_str().is_none()
        || directory
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(io::Error::other(
            "agent workload directory must be an absolute UTF-8 path without parent traversal",
        ));
    }
    // SAFETY: geteuid only reads the process identity.
    let owner = unsafe { libc::geteuid() };
    let metadata = open_trusted_path(directory, owner, TrustedPathKind::Directory)?.metadata()?;
    if metadata.uid() != owner || metadata.mode() & 0o022 != 0 {
        return Err(io::Error::other(
            "agent workload directory must belong to the Node identity and be writable by no one else",
        ));
    }
    if overlaps(directory, config, &[]) {
        return Err(io::Error::other(
            "agent workload directory overlaps Node state, the retired host directory or the clone root",
        ));
    }
    Ok(())
}

/// Checks the `ora-git` socket directory, which the Node creates when missing and clears of its
/// own sockets when it starts: the same absolute path rules as the workload directory, owned by
/// the Node identity with no one else allowed to list or change it (so `/run` or `/tmp` are
/// refused), under a parent only trusted owners control, and overlapping no Node, retired host, clone or
/// workload directory.
fn validate_ora_git(agent: &AgentConfig, config: &ServiceConfig) -> io::Result<()> {
    let Some(directory) = agent.ora_git_directory.as_deref() else {
        return Ok(());
    };
    let refuse = |reason: &str| {
        Err(io::Error::other(format!(
            "agent ora_git_directory {reason}"
        )))
    };
    let Some(parent) = directory.parent().filter(|_| {
        directory.is_absolute()
            && directory.to_str().is_some()
            && !directory
                .components()
                .any(|component| matches!(component, Component::ParentDir))
    }) else {
        return refuse("must be an absolute UTF-8 path without parent traversal, below the root");
    };
    // SAFETY: geteuid only reads the process identity.
    let owner = unsafe { libc::geteuid() };
    match open_trusted_path(directory, owner, TrustedPathKind::Directory) {
        Ok(opened) => {
            let metadata = opened.metadata()?;
            if metadata.uid() != owner || metadata.mode() & 0o066 != 0 {
                return refuse(
                    "must belong to the Node identity and be readable and writable by no one else",
                );
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if open_trusted_path(parent, owner, TrustedPathKind::Directory).is_err() {
                return refuse("must be created under a directory only trusted owners control");
            }
        }
        Err(_) => return refuse("must be a directory only trusted owners control"),
    }
    let workload = agent.workload_directory.as_deref();
    if overlaps(directory, config, &[workload]) {
        return refuse(
            "overlaps Node state, the retired host directory, the clone root or the workload directory",
        );
    }
    Ok(())
}

/// Whether `directory` contains, or is contained in, a directory whose contents the Node manages.
fn overlaps(
    directory: &std::path::Path,
    config: &ServiceConfig,
    others: &[Option<&std::path::Path>],
) -> bool {
    let directory = canonicalize_longest_existing_prefix(directory);
    let protected = [
        Some(config.node.home_directory.as_path()),
        config.process.host_directory.as_deref(),
        config
            .clone
            .as_ref()
            .map(|clone| clone.repository_root.as_path()),
    ];
    protected
        .into_iter()
        .chain(others.iter().copied())
        .flatten()
        .any(|path| {
            let path = canonicalize_longest_existing_prefix(path);
            path.starts_with(&directory) || directory.starts_with(&path)
        })
}

/// Derives where and as whom agents run from configuration [`validate`] already accepted.
pub(super) fn workload(
    agent: Option<&AgentConfig>,
    process: &ProcessConfig,
) -> io::Result<SessionWorkload> {
    let directory = agent.and_then(|agent| agent.workload_directory.as_deref());
    match (directory, process.workload_uid) {
        (Some(directory), Some(uid)) => Ok(SessionWorkload::Separate {
            directory: directory.to_path_buf(),
            // The group equals the user, as for the clone workload's Git.
            identity: ProcessIdentity::Linux(ora_utils::process::LinuxChildIdentity::new(
                uid, uid,
            )?),
            uid,
            gid: uid,
        }),
        (None, _) | (_, None) => Ok(SessionWorkload::Shared),
    }
}

/// Restore composition and stop authority shared by startup, the worker and status queries.
pub(super) struct Recovery<'a> {
    pub(super) restorer: Option<Restorer>,
    pub(super) shutdown: &'a crate::Shutdown,
    pub(super) stops: &'a crate::managed::RestoreStops,
}

/// Recovers every unfinished input before the control listener is published, even without Agent
/// configuration. Recovery never spawns an Agent or treats the old input as permission to resume.
pub(super) fn open(
    node: &mut ManagedNode,
    config: Option<&AgentConfig>,
    workload: SessionWorkload,
    timezone: &str,
    catalog: DirectoryPluginCatalog,
    recovery: Recovery<'_>,
) -> Result<Option<SessionHost>, crate::Error> {
    let Recovery {
        restorer,
        shutdown,
        stops,
    } = recovery;
    let journal = node.database.session_journal()?;
    let ora_git = config
        .and_then(|config| config.ora_git_directory.as_deref())
        .map(|directory| ora_git(node, directory, journal.clone(), shutdown.clone()))
        .transpose()?
        .flatten();
    let host = config
        .map(|config| {
            let timezone = timezone
                .parse()
                .map_err(|_| crate::Error::Configuration("invalid Agent timezone".into()))?;
            Ok::<_, crate::Error>(AgentSessions::new(
                SessionConfig {
                    home_directory: node.home_directory().to_path_buf(),
                    deno_path: config.deno_path.clone(),
                    timezone,
                    agent_ready_timeout: Duration::from_millis(config.ready_timeout_ms),
                    model_proxy: config.model_proxy.clone(),
                    workload: workload.clone(),
                },
                node.identity().clone(),
                journal.clone(),
                journal.clone(),
                catalog,
                restorer,
            ))
        })
        .transpose()?
        .map(|host| match ora_git {
            Some(broker) => host.with_ora_git(broker),
            None => host,
        });
    tokio::runtime::Handle::current().block_on(reconcile(
        node,
        host.as_ref(),
        stops,
        crate::managed::StopCheck::Handover,
        None,
    ))?;
    sweep_restores(node, &journal)?;
    // Unknown restore responsibility also retains its session placement across restart.
    if node.database.recoverable_sessions()?.is_empty()
        && let SessionWorkload::Separate { directory, .. } = &workload
    {
        crate::session::purge_workload_directory(directory).map_err(ora_node_db::Error::from)?;
    }
    Ok(host)
}

/// Composes the `ora-git` service when the Node can reach the Git Server: Node-identity access
/// from transport repositories, checkout Git as the workload user with delivery's policy.
fn ora_git(
    node: &ManagedNode,
    directory: &std::path::Path,
    ledger: SessionJournal,
    shutdown: crate::Shutdown,
) -> Result<Option<crate::ora_git::Availability>, crate::Error> {
    let (Some(policy), Some(access), Some(config)) = (
        node.delivery_git_policy()?,
        node.git_server(),
        node.clone_config(),
    ) else {
        return Ok(None);
    };
    let git = || {
        node.git
            .runner()
            .detached_git()
            .map_err(ora_node_db::Error::from)
    };
    let owner = node.git.runner().process_config().workload_uid;
    // SAFETY: geteuid only reads the process identity.
    let node_uid = unsafe { libc::geteuid() };
    crate::ora_git::Availability::for_workload(owner, node_uid, |peer_uid| {
        let service = crate::ora_git::OraGitService::new(crate::ora_git::OraGitParts {
            directory: directory.to_path_buf(),
            peer_uid,
            workload: DeliveryGit::new(git()?, policy),
            node: AsNode(git()?),
            access,
            repository_root: config.repository_root.clone(),
            owner: Some(peer_uid),
            ledger,
            shutdown,
        })
        .map_err(ora_node_db::Error::from)?;
        Ok(std::sync::Arc::new(std::sync::Arc::new(service)))
    })
    .map(Some)
}

/// A repeated start only reports durable state; it never relaunches an existing session.
pub(super) fn start(
    node: &mut ManagedNode,
    host: &SessionHost,
    controller: &ControllerId,
    input: &StartAgentSessionMessage,
    permit: Option<&RuntimeBinding>,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
    if input.payload.spec.model_binding_id.is_some() && !host.model_capable() {
        return Err(crate::Error::UnsupportedMessage);
    }
    // The Controller dispatches only on an advertised match; anything else is a protocol fault.
    if let Some(configuration) = &input.payload.spec.agent_configuration
        && !host
            .model_adapter()
            .is_some_and(|adapter| adapter.admits(configuration))
    {
        return Err(crate::Error::UnsupportedMessage);
    }
    node.database.check_controller_execution(
        controller,
        &input.operation_id,
        &input.execution_id,
    )?;
    let record = match permit {
        Some(permit) => {
            if permit.node_incarnation_id != node.identity().incarnation_id.as_str() {
                return Err(ora_node_db::Error::NodeMismatch.into());
            }
            node.database.accept_controlled_session(input, permit)?
        }
        None => node.database.accept_session(input)?,
    };
    // An earlier write failure may leave Accepted in this incarnation. It has never
    // started an actor and may retry its guarded transition; Running is never restarted.
    if record.state == ExecutionState::Accepted {
        if node
            .database
            .start_session(input, &node.identity().incarnation_id.clone())?
        {
            host.start(
                input.operation_id.clone(),
                input.execution_id.clone(),
                input.payload.spec.clone(),
            );
        } else {
            node.database.session_journal()?.end_session(
                &input.execution_id,
                host.recover_interrupted(&input.execution_id),
            )?;
        }
    }
    let state = node
        .database
        .execution_state(&input.operation_id, &input.execution_id)?;
    Ok(vec![NodeToControllerMessage::ExecutionStatus(
        ExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: input.operation_id.clone(),
            execution_id: input.execution_id.clone(),
            payload: ExecutionStatus {
                stop_unconfirmed: Vec::new(),
                node: node.identity().clone(),
                state,
            },
        },
    )])
}

/// Admission does not wake the actor: the transport does that after sending the accepted reply.
pub(super) fn command(
    node: &mut ManagedNode,
    controller: &ControllerId,
    input: SessionCommandInput,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
    node.database.check_controller_execution(
        controller,
        input.operation_id(),
        input.execution_id(),
    )?;
    let state = node
        .database
        .execution_state(input.operation_id(), input.execution_id())?;
    if !matches!(state, ExecutionState::Completed(_)) {
        node.database
            .authorize_session_command(input.execution_id(), &node.identity().incarnation_id)?;
    }
    let result = node.database.accept_session_command(&input)?;
    let operation_id = input.operation_id().clone();
    let execution_id = input.execution_id().clone();
    let command_id = input.command_id().clone();
    Ok(vec![match result {
        CommandAdmission::Accepted => {
            NodeToControllerMessage::SessionCommandAccepted(SessionCommandAcceptedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id,
                execution_id,
                payload: SessionCommandAccepted { command_id },
            })
        }
        CommandAdmission::SessionEnded => {
            NodeToControllerMessage::SessionCommandRejected(SessionCommandRejectedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id,
                execution_id,
                payload: SessionCommandRejected {
                    command_id,
                    reason: SessionCommandRejection::SessionEnded,
                },
            })
        }
    }])
}

#[cfg(test)]
mod tests;
