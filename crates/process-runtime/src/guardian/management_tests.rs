use super::*;
use ora_process_protocol::{GuardianInstanceId, HostInstanceId, ScopeId};
use pretty_assertions::assert_eq;
use std::{
    future::Future,
    num::NonZeroU64,
    task::{Context, Poll, Waker},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Dropping the listener's owner cannot release the scope while an executing worker retains SQLite.
#[test]
fn executing_worker_retains_scope_lock_after_service_owner_drops() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("scope.lock");
    let mut owner = fixture(Connection::open_in_memory()?)?;
    owner._lock = ora_utils::fs::LinuxFileLock::try_acquire(std::fs::File::create(&path)?)?;
    let owner = std::sync::Arc::new(tokio::sync::Mutex::new(owner));
    let worker = owner.clone();
    drop(owner);
    assert!(
        matches!(ora_utils::fs::LinuxFileLock::try_acquire(std::fs::File::open(&path)?), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    drop(worker);
    let _recovered = ora_utils::fs::LinuxFileLock::try_acquire(std::fs::File::open(&path)?)?;
    Ok(())
}

/// Initializes the same schema and transactional session owner used by the guardian app.
fn fixture(connection: Connection) -> Result<Management, Box<dyn std::error::Error>> {
    let intent = ScopeCreationIntent {
        scope: ScopeId::new(),
        guardian: GuardianInstanceId::new(),
        created_by: HostBinding {
            epoch: NonZeroU64::MIN,
            instance: HostInstanceId::new(),
        },
    };
    let mut owner = Management::new(
        connection,
        intent,
        ora_utils::fs::LinuxFileLock::try_acquire(tempfile::tempfile()?)?,
    )?;
    let transaction = owner.connection.transaction()?;
    initialize(&transaction, owner.intent.created_by)?;
    super::super::runs::initialize(&transaction)?;
    transaction.commit()?;
    Ok(owner)
}

/// Extracts a successful binding so a rejection cannot accidentally satisfy subsequent checks.
fn session(
    reply: GuardianManagementReply,
) -> Result<GuardianHostSession, Box<dyn std::error::Error>> {
    match reply {
        GuardianManagementReply::Bound { session, .. } => Ok(session),
        other => Err(format!("expected bound session, got {other:?}").into()),
    }
}

/// A decoded old request queued at the actual execution mutex cannot use an ingress-time grant.
#[tokio::test]
async fn queued_request_checks_binding_after_takeover() -> TestResult {
    let mut owner = fixture(Connection::open_in_memory()?)?;
    let original = owner.intent.created_by;
    let old = session(owner.execute(
        GuardianChannel::Control,
        GuardianManagementOperation::Bind { host: original },
    ))?;
    let mutex = tokio::sync::Mutex::new(owner);
    let mut held = mutex.lock().await;
    let queued = async {
        mutex.lock().await.execute(
            GuardianChannel::Io,
            GuardianManagementOperation::Inspect { session: old },
        )
    };
    tokio::pin!(queued);
    // Poll once while held to prove this decoded operation has actually entered the mutex queue.
    assert!(matches!(
        queued
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    let replacement = HostBinding {
        epoch: NonZeroU64::new(2).ok_or("invalid epoch")?,
        instance: HostInstanceId::new(),
    };
    session(held.execute(
        GuardianChannel::Control,
        GuardianManagementOperation::Bind { host: replacement },
    ))?;
    drop(held);
    assert_eq!(
        queued.await,
        GuardianManagementReply::Rejected(Rejection::StaleSession)
    );
    Ok(())
}

/// Failed persistence cannot acknowledge takeover; retry and re-open preserve the accepted session.
#[test]
fn persistence_failure_and_reopen_never_reset_authority() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("journal.sqlite");
    std::fs::File::create(&path)?;
    let mut owner = fixture(crate::state_journal::open_writable(&path)?)?;
    let original = owner.intent.created_by;
    let old = session(owner.execute(
        GuardianChannel::Control,
        GuardianManagementOperation::Bind { host: original },
    ))?;
    let next = HostBinding {
        epoch: NonZeroU64::new(2).ok_or("invalid epoch")?,
        instance: HostInstanceId::new(),
    };
    owner.connection.execute_batch("PRAGMA query_only=ON;")?;
    assert_eq!(
        owner.execute(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host: next }
        ),
        GuardianManagementReply::Rejected(Rejection::StorageUnavailable)
    );
    assert_eq!(
        session(owner.execute(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host: original }
        ))?,
        old
    );
    owner.connection.execute_batch("PRAGMA query_only=OFF;")?;
    let accepted = session(owner.execute(
        GuardianChannel::Control,
        GuardianManagementOperation::Bind { host: next },
    ))?;
    let intent = owner.intent.clone();
    drop(owner);
    // This is journal verification only, never permission to restart a dead guardian.
    let mut owner = Management::new(
        crate::state_journal::open_writable(&path)?,
        intent,
        ora_utils::fs::LinuxFileLock::try_acquire(tempfile::tempfile()?)?,
    )?;
    assert_eq!(
        session(owner.execute(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host: next }
        ))?,
        accepted
    );
    assert_eq!(
        owner.execute(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host: original }
        ),
        GuardianManagementReply::Rejected(Rejection::StaleHost)
    );
    assert_eq!(
        owner.execute(
            GuardianChannel::Events,
            GuardianManagementOperation::Bind { host: next }
        ),
        GuardianManagementReply::Rejected(Rejection::WrongChannel)
    );
    let invalid = HostBinding {
        epoch: NonZeroU64::MAX,
        instance: HostInstanceId::new(),
    };
    assert_eq!(
        owner.execute(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host: invalid }
        ),
        GuardianManagementReply::Rejected(Rejection::InvalidEpoch)
    );
    assert_eq!(
        session(owner.execute(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host: next }
        ))?,
        accepted
    );
    owner
        .connection
        .execute_batch("BEGIN IMMEDIATE; UPDATE guardian_host_session SET host_epoch=99;")?;
    assert_eq!(
        owner.execute(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host: next }
        ),
        GuardianManagementReply::Rejected(Rejection::StorageUnavailable)
    );
    owner.connection.execute_batch("ROLLBACK;")?;
    assert_eq!(
        session(owner.execute(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host: next }
        ))?,
        accepted
    );
    Ok(())
}

/// Builds a Run-ledger request on the channel its operation is served from.
fn run_request(
    owner: &Management,
    operation: ora_process_protocol::GuardianRunOperation,
) -> ora_process_protocol::GuardianRunRequest {
    ora_process_protocol::GuardianRunRequest {
        version: ora_process_protocol::GUARDIAN_WIRE_VERSION,
        intent: owner.intent.clone(),
        channel: operation.channel(),
        session: GuardianHostSession {
            host: owner.intent.created_by,
        },
        operation,
    }
}

/// The service loop disarms its timer on this signal, so it must flip only at the terminal
/// state and must not take the query path down with it.
#[test]
fn reconcile_reports_settled_only_after_close_and_queries_still_answer() -> TestResult {
    use ora_process_protocol::{
        CleanupEvidence, GuardianRunOperation, GuardianRunReply, GuardianRunResult, ScopeState,
    };
    let mut owner = fixture(Connection::open_in_memory()?)?;
    assert!(!owner.reconcile(), "an open Scope still needs its timer");
    let session = GuardianHostSession {
        host: owner.intent.created_by,
    };
    let closed = GuardianRunResult::Scope(ScopeState::Closed(CleanupEvidence::BestEffortComplete));
    let close = run_request(&owner, GuardianRunOperation::Close);
    assert_eq!(
        owner.execute_run(close.channel, close),
        GuardianRunReply {
            intent: owner.intent.clone(),
            session: session.clone(),
            result: closed.clone(),
        }
    );
    assert!(owner.reconcile());
    // Settled is terminal: later reconciles keep reporting it and never touch the journal again.
    owner.connection.execute_batch("PRAGMA query_only=ON;")?;
    assert!(owner.reconcile());
    let query = run_request(&owner, GuardianRunOperation::Scope);
    assert_eq!(
        owner.execute_run(query.channel, query),
        GuardianRunReply {
            intent: owner.intent.clone(),
            session,
            result: closed,
        }
    );
    Ok(())
}
