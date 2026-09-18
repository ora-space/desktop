use super::*;
use ora_process_client::GuardianManagement;
use ora_process_protocol::{
    GuardianHostSession, GuardianManagementOperation, GuardianManagementRejection as Rejection,
    GuardianManagementReply as Reply, GuardianManagementRequest, HostBinding, HostInstanceId,
};
use pretty_assertions::assert_eq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Extracts the complete accepted session without treating a transport success as acceptance.
fn bound(reply: Reply) -> Result<GuardianHostSession, Box<dyn std::error::Error>> {
    match reply {
        Reply::Bound { session, .. } => Ok(session),
        other => Err(format!("unexpected binding reply: {other:?}").into()),
    }
}

/// Real takeover commits before reply, invalidates old channels and survives lost caller replies.
#[tokio::test]
async fn durable_takeover_fences_old_channels_and_replays_original_session() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let mut host = HostState::create(&root)?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let original_host = host.binding();
    let access = host.start_guardian(scope, &fixture.executable).await?;
    ready(&access).await?;
    // SAFETY: geteuid reads the test's current OS identity.
    let manager = GuardianManagement::new(access.clone(), unsafe { libc::geteuid() });
    let old = bound(manager.bind(original_host).await?)?;
    assert_eq!(bound(manager.bind(original_host).await?)?, old);
    let mut wrong = access.clone();
    wrong.intent.guardian = ora_process_protocol::GuardianInstanceId::new();
    // SAFETY: geteuid only queries the current identity.
    let owner = unsafe { libc::geteuid() };
    assert!(
        GuardianManagement::new(wrong, owner)
            .bind(original_host)
            .await
            .is_err()
    );
    assert!(
        GuardianManagement::new(access.clone(), owner.wrapping_add(1))
            .bind(original_host)
            .await
            .is_err()
    );

    // A live old connection delivers its complete request only after a newer host has committed.
    let mut delayed = tokio::net::UnixStream::connect(access.scope_dir.join("io.sock")).await?;
    let frame = encode_guardian_frame(&GuardianManagementRequest {
        version: GUARDIAN_WIRE_VERSION,
        intent: access.intent.clone(),
        channel: GuardianChannel::Io,
        operation: GuardianManagementOperation::Inspect {
            session: old.clone(),
        },
    })?;
    delayed.write_all(&frame[..4]).await?;
    drop(host);
    let recovered = recover(&root).await?;
    let new = bound(manager.bind(recovered.binding()).await?)?;
    assert_ne!(old.host, new.host);
    assert_eq!(new.host, recovered.binding());
    assert_eq!(bound(manager.bind(recovered.binding()).await?)?, new);
    for channel in [
        GuardianChannel::Control,
        GuardianChannel::Events,
        GuardianChannel::Io,
    ] {
        assert_eq!(
            manager.inspect(channel, old.clone()).await?,
            Reply::Rejected(Rejection::StaleSession)
        );
        assert_eq!(
            manager.inspect(channel, new.clone()).await?,
            Reply::Current {
                intent: access.intent.clone(),
                session: new.clone(),
                channel,
            }
        );
    }
    delayed.write_all(&frame[4..]).await?;
    let length = delayed.read_u32().await? as usize;
    assert!(length <= ora_process_protocol::GUARDIAN_MAX_FRAME);
    let mut bytes = vec![0; length];
    delayed.read_exact(&mut bytes).await?;
    assert_eq!(
        ora_process_protocol::decode_guardian_payload::<Reply>(&bytes)?,
        Reply::Rejected(Rejection::StaleSession)
    );

    assert_eq!(
        manager.bind(original_host).await?,
        Reply::Rejected(Rejection::StaleHost)
    );
    assert_eq!(
        manager
            .bind(HostBinding {
                epoch: new.host.epoch,
                instance: HostInstanceId::new()
            })
            .await?,
        Reply::Rejected(Rejection::ConflictingHost)
    );
    let journal = rusqlite::Connection::open_with_flags(
        access.scope_dir.join("guardian.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let stored = journal.query_row(
        "SELECT host_epoch, host_instance FROM guardian_host_session",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(/*idx*/ 0)?,
                row.get::<_, String>(/*idx*/ 1)?,
            ))
        },
    )?;
    assert_eq!(
        stored,
        (new.host.epoch.get() as i64, new.host.instance.to_string())
    );

    // Lose the takeover response: query the durable row to synchronize, then replay that binding.
    drop(recovered);
    let recovered = recover(&root).await?;
    let mut lost = tokio::net::UnixStream::connect(access.scope_dir.join("control.sock")).await?;
    lost.write_all(&encode_guardian_frame(&GuardianManagementRequest {
        version: GUARDIAN_WIRE_VERSION,
        intent: access.intent.clone(),
        channel: GuardianChannel::Control,
        operation: GuardianManagementOperation::Bind {
            host: recovered.binding(),
        },
    })?)
    .await?;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    loop {
        let epoch: i64 =
            journal.query_row("SELECT host_epoch FROM guardian_host_session", [], |row| {
                row.get(/*idx*/ 0)
            })?;
        if epoch == recovered.binding().epoch.get() as i64 {
            break;
        }
        assert!(Instant::now() < deadline, "takeover was not committed");
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    drop(lost);
    let session = bound(manager.bind(recovered.binding()).await?)?;
    assert_eq!(bound(manager.bind(recovered.binding()).await?)?, session);
    assert_eq!(
        manager.inspect(GuardianChannel::Control, new).await?,
        Reply::Rejected(Rejection::StaleSession)
    );
    Ok(())
}
