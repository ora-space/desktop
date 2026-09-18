use super::*;
use pretty_assertions::assert_eq;

/// Rejected limits consume no attempt; proven non-starts consume their ID and count toward capacity.
#[tokio::test]
async fn limits_and_proven_non_starts_preserve_attempt_identity() -> TestResult {
    let fixture = Fixture::new()?;
    let mut host = HostState::create(&fixture.directory.path().join("s"))?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &fixture.executable).await?;
    ready(&access).await?;
    // SAFETY: geteuid only reads the current OS identity.
    let client = GuardianRuns::new(
        access.clone(),
        unsafe { libc::geteuid() },
        bind(&access, host.binding()).await?,
    );
    let mut spec = RunSpec::new(
        fixture.directory.path().join("absent-program"),
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.output = OutputPolicy::Capture {
        stdout_limit: ora_process_protocol::GUARDIAN_CAPTURE_LIMIT + 1,
        stderr_limit: 0,
    };
    let id = RunId::new();
    assert_eq!(
        client
            .execute(Operation::Start {
                run: id,
                spec: spec.clone(),
                host_disconnect: GuardianHostDisconnect::KeepRunning
            })
            .await?,
        Reply::Rejected(Rejection::LimitExceeded)
    );
    assert_eq!(
        client.execute(Operation::Query { run: id }).await?,
        Reply::Rejected(Rejection::UnknownRun)
    );
    spec.output = OutputPolicy::Discard;
    for run in std::iter::once(id).chain((1..64).map(|_| RunId::new())) {
        let start = Operation::Start {
            run,
            spec: spec.clone(),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        };
        let reply = client.execute(start.clone()).await?;
        assert!(
            matches!(&reply, Reply::Run(snapshot) if matches!(snapshot.launch, LaunchFact::NotStarted(_)))
        );
        assert_eq!(client.execute(start).await?, reply);
    }
    assert_eq!(
        client
            .execute(Operation::Start {
                run: RunId::new(),
                spec,
                host_disconnect: GuardianHostDisconnect::KeepRunning
            })
            .await?,
        Reply::Rejected(Rejection::LimitExceeded)
    );
    Ok(())
}

/// A committed acceptance without a live runtime record remains uncertain and blocks completed closure.
#[tokio::test]
async fn durable_unknown_attempt_is_never_executed_by_replay() -> TestResult {
    let fixture = Fixture::new()?;
    let mut host = HostState::create(&fixture.directory.path().join("s"))?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &fixture.executable).await?;
    ready(&access).await?;
    // SAFETY: geteuid only reads the current OS identity.
    let client = GuardianRuns::new(
        access.clone(),
        unsafe { libc::geteuid() },
        bind(&access, host.binding()).await?,
    );
    let run = RunId::new();
    let mut spec = RunSpec::new(
        "/bin/sh",
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args = ["-c", "printf unexpected > count"].map(Into::into).to_vec();
    let unknown = RunSnapshot {
        id: run,
        launch: LaunchFact::Unknown("fixture committed acceptance".into()),
        direct: DirectProcessState::Unknown,
        cleanup: CleanupState::Pending,
    };
    let encoded_spec = encode_guardian_frame(&spec)?;
    let encoded_snapshot = encode_guardian_frame(&unknown)?;
    let journal = rusqlite::Connection::open(access.scope_dir.join("guardian.sqlite"))?;
    journal.execute(
        "INSERT INTO guardian_runs VALUES (?1, ?2, ?3, 'keep_running', 0)",
        rusqlite::params![run.to_string(), &encoded_spec[4..], &encoded_snapshot[4..]],
    )?;
    assert_eq!(
        client
            .execute(Operation::Start {
                run,
                spec,
                host_disconnect: GuardianHostDisconnect::KeepRunning
            })
            .await?,
        Reply::Run(unknown.clone())
    );
    assert_eq!(
        client.execute(Operation::Stop { run }).await?,
        Reply::Run(unknown)
    );
    assert_eq!(
        client.execute(Operation::Close).await?,
        Reply::Scope(ScopeState::Closing)
    );
    assert_eq!(
        client.execute(Operation::Scope).await?,
        Reply::Scope(ScopeState::Closing)
    );
    assert!(!fixture.directory.path().join("count").exists());
    Ok(())
}
