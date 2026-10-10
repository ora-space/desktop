//! Production composition and durable admission of Agent sessions.
use super::{AgentConfig, ServiceConfig};
use crate::managed::CloneHost;
use crate::revision::{
    DeliveryGit, DownloadGrants, HttpDownloader, RESTORE_ROOT, RestorePolicy, RevisionRestorer,
    purge_restores,
};
use crate::{
    AgentSessions, DirectoryPluginCatalog, ManagedNode, ProcessConfig, SessionConfig,
    SessionHost as _, SessionWorkload,
};
use ora_node_db::{CommandAdmission, SessionCommandInput, SessionJournal};
use ora_node_protocol::*;
use ora_process::ProcessIdentity;
use ora_utils::http::{FetchOptions, ProxyConfig, ReqwestFetcher};
use ora_utils::path::{TrustedPathKind, canonicalize_longest_existing_prefix, open_trusted_path};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Component;
use std::time::Duration;

/// Restores prior Revisions with delivery's Git and the presigned downloader.
pub(super) type Restorer = RevisionRestorer<CloneHost, HttpDownloader>;

pub(super) type SessionHost =
    AgentSessions<SessionJournal, SessionJournal, DirectoryPluginCatalog, Option<Restorer>>;

/// Bounds one bundle request; the restore deadline bounds the whole download anyway.
const FETCH_OPTIONS: FetchOptions = FetchOptions {
    connect_timeout: Duration::from_secs(/*secs*/ 30),
    total_timeout: Duration::from_secs(/*secs*/ 5 * 60),
};

/// Clears what restores of a previous process left, then composes restore when this Node can run
/// both sessions and delivery Git: restore runs Git in the checkouts clone created, with
/// delivery's policy, so without clone configuration there is nothing to restore into.
pub(super) fn restorer(
    node: &ManagedNode,
    agent: Option<&AgentConfig>,
    downloads: DownloadGrants,
) -> Result<Option<Restorer>, crate::Error> {
    let root = node.home_directory().join(RESTORE_ROOT);
    purge_restores(&root).map_err(ora_node_db::Error::from)?;
    let (Some(_), Some(policy)) = (agent, node.delivery_git_policy()?) else {
        return Ok(None);
    };
    let runner = node
        .git
        .runner()
        .detached_clone_host()
        .map_err(ora_node_db::Error::from)?;
    Ok(Some(RevisionRestorer::new(
        DeliveryGit::new(runner, policy),
        HttpDownloader::new(ReqwestFetcher::new(ProxyConfig::default()), FETCH_OPTIONS),
        downloads,
        node.node_id().clone(),
        root,
        node.git.runner().process_config().workload_uid,
        RestorePolicy::DEFAULT,
    )))
}

/// Checks the Agent configuration before anything starts.
///
/// Agents run as the workload user exactly when Git workloads do: a Node that separates Git from
/// itself but ran agents as root would hand them its secrets and a checkout Git refuses as
/// dubiously owned, and a workload directory without a workload user would have no owner to
/// serve. The directory must already exist as the deployment's: owned by the Node's own identity
/// (root in a sandbox) and writable by nobody else, because the Node creates, links and removes
/// trees inside it with that authority. It must also not overlap any state or checkout root, so
/// purging it can never reach them.
pub(super) fn validate(agent: &AgentConfig, config: &ServiceConfig) -> io::Result<()> {
    if !agent.deno_path.is_absolute() || agent.ready_timeout_ms == 0 {
        return Err(io::Error::other(
            "agent needs an absolute Deno path and a positive ready timeout",
        ));
    }
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
    let directory = canonicalize_longest_existing_prefix(directory);
    let protected = [
        Some(config.node.home_directory.as_path()),
        Some(config.process.host_directory.as_path()),
        config
            .clone
            .as_ref()
            .map(|clone| clone.repository_root.as_path()),
    ];
    for path in protected.into_iter().flatten() {
        let path = canonicalize_longest_existing_prefix(path);
        if path.starts_with(&directory) || directory.starts_with(&path) {
            return Err(io::Error::other(
                "agent workload directory overlaps Node state, host state or the clone root",
            ));
        }
    }
    Ok(())
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
            // The group equals the user, as for the clone workload's `setpriv --regid`.
            identity: ProcessIdentity::Linux(ora_utils::process::LinuxChildIdentity::new(
                uid, uid,
            )?),
            uid,
            gid: uid,
        }),
        (None, _) | (_, None) => Ok(SessionWorkload::Shared),
    }
}

/// Recovers every unfinished input before the control listener is published, even without Agent
/// configuration. Recovery never spawns an Agent or treats the old input as permission to resume.
pub(super) fn open(
    node: &mut ManagedNode,
    config: Option<&AgentConfig>,
    workload: SessionWorkload,
    timezone: &str,
    catalog: DirectoryPluginCatalog,
    restorer: Option<Restorer>,
) -> Result<Option<SessionHost>, crate::Error> {
    let journal = node.database.session_journal()?;
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
                    workload: workload.clone(),
                },
                node.identity().clone(),
                journal.clone(),
                journal.clone(),
                catalog,
                restorer,
            ))
        })
        .transpose()?;
    for record in node.database.recoverable_sessions()? {
        let ended = match &host {
            Some(host) => host.recover_interrupted(&record.command.execution_id),
            None => AgentSessionEnded {
                node: node.identity().clone(),
                reason: AgentSessionEndReason::Interrupted,
                detail: None,
            },
        };
        journal.end_session(&record.command.execution_id, ended)?;
    }
    // Every session the previous process ran has ended above, so its directories are unused.
    if let SessionWorkload::Separate { directory, .. } = &workload {
        crate::session::purge_workload_directory(directory).map_err(ora_node_db::Error::from)?;
    }
    Ok(host)
}

/// A repeated start only reports durable state; it never relaunches an existing session.
pub(super) fn start(
    node: &mut ManagedNode,
    host: &SessionHost,
    controller: &ControllerId,
    input: &StartAgentSessionMessage,
    permit: Option<&RuntimeBinding>,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
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
