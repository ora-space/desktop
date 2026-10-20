use super::{ProcessConfig, Shutdown};
use gitlancer::{GitIntent, GitOutput};
use ora_process_client::ProcessHost;
use ora_process_protocol::*;
use std::{
    io,
    time::{Duration, Instant},
};

/// Replays scope creation before sealing: even a lost CreateScope reply cannot bypass the close barrier.
pub(super) async fn close(
    client: &ProcessHost,
    scope: ScopeId,
    timeout: Duration,
) -> io::Result<()> {
    match client.execute(HostOperation::CreateScope { scope }).await? {
        HostReply::Scope(_) => {}
        other => {
            return Err(io::Error::other(format!(
                "cannot recover original Scope: {other:?}"
            )));
        }
    }
    let deadline = Instant::now() + timeout;
    loop {
        match client.execute(HostOperation::Close { scope }).await? {
            HostReply::Scope(view) if matches!(view.last_observed, Some(ScopeState::Closed(_))) => {
                return Ok(());
            }
            HostReply::Scope(_) if Instant::now() < deadline => {}
            other => {
                return Err(io::Error::other(format!(
                    "original Scope cleanup is unverified: {other:?}"
                )));
            }
        }
        tokio::time::sleep(Duration::from_millis(/*millis*/ 25)).await;
    }
}

/// A timeout requests cleanup; neither acceptance nor a signal is interpreted as completion.
pub(super) async fn execute(
    client: &ProcessHost,
    intent: &HostRunIntent,
    command_intent: GitIntent,
    config: &ProcessConfig,
    shutdown: &Shutdown,
) -> io::Result<GitOutput> {
    let started = Instant::now();
    observe(
        client,
        intent,
        config,
        shutdown,
        started,
        Cutoff::Command(started + config.command_budget(command_intent)),
    )
    .await
}

/// An absolute restore cutoff also bounds in-flight IPC; cleanup is performed by the caller
/// outside this timeout and must still close the original Scope after a lost Start reply.
pub(super) async fn execute_before(
    client: &ProcessHost,
    intent: &HostRunIntent,
    command_intent: GitIntent,
    config: &ProcessConfig,
    shutdown: &Shutdown,
    restore_deadline: Instant,
) -> io::Result<GitOutput> {
    let started = Instant::now();
    let deadline = restore_deadline.min(started + config.command_budget(command_intent));
    tokio::time::timeout_at(
        deadline.into(),
        observe(
            client,
            intent,
            config,
            shutdown,
            started,
            Cutoff::Restore(deadline),
        ),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Git completion deadline expired"))?
}

/// Restore clips admission and IPC as well as completion; ordinary command semantics stay local.
#[derive(Clone, Copy)]
enum Cutoff {
    Command(Instant),
    Restore(Instant),
}

impl Cutoff {
    /// A ready IPC response must not admit another restore operation after its absolute cutoff.
    fn check_exchange(self) -> io::Result<()> {
        match self {
            Self::Command(_) => Ok(()),
            Self::Restore(deadline) => check(deadline),
        }
    }
}

/// Only execution and output collection consume this cutoff, never the subsequent close barrier.
async fn observe(
    client: &ProcessHost,
    intent: &HostRunIntent,
    config: &ProcessConfig,
    shutdown: &Shutdown,
    started: Instant,
    cutoff: Cutoff,
) -> io::Result<GitOutput> {
    let (Cutoff::Command(deadline) | Cutoff::Restore(deadline)) = cutoff;
    cutoff.check_exchange()?;
    match client
        .execute(HostOperation::CreateScope {
            scope: intent.scope,
        })
        .await?
    {
        HostReply::Scope(_) => {}
        other => return Err(io::Error::other(format!("Scope rejected: {other:?}"))),
    }
    cutoff.check_exchange()?;
    match client
        .execute(HostOperation::Start {
            intent: intent.clone(),
        })
        .await?
    {
        HostReply::Run(_) => {}
        other => return Err(io::Error::other(format!("Run rejected: {other:?}"))),
    }
    let outcome = loop {
        if Instant::now() >= deadline
            || shutdown.expired(Duration::from_millis(config.shutdown_grace_ms))
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Git completion deadline expired",
            ));
        }
        match client
            .execute(HostOperation::QueryRun { run: intent.run })
            .await?
        {
            HostReply::Run(view) => {
                cutoff.check_exchange()?;
                if let Some(snapshot) = view.last_observed
                    && matches!(snapshot.cleanup, CleanupState::Complete(_))
                {
                    break match snapshot.direct {
                        DirectProcessState::Exited(ExitOutcome::Code(code)) => Some(code),
                        DirectProcessState::Exited(
                            ExitOutcome::Signal(_) | ExitOutcome::Unknown,
                        )
                        | DirectProcessState::NotStarted => None,
                        DirectProcessState::Running | DirectProcessState::Unknown => {
                            return Err(io::Error::other(
                                "cleanup contradicts direct-process state",
                            ));
                        }
                    };
                }
            }
            other => return Err(io::Error::other(format!("Run query rejected: {other:?}"))),
        }
        tokio::time::sleep(Duration::from_millis(/*millis*/ 25)).await;
    };
    if intent.spec.output == OutputPolicy::Discard {
        return Ok(GitOutput::new(
            outcome,
            String::new(),
            String::new(),
            started.elapsed().as_millis() as u64,
        ));
    }
    let mut streams = [String::new(), String::new()];
    for (slot, stream) in streams
        .iter_mut()
        .zip([OutputStream::Stdout, OutputStream::Stderr])
    {
        let mut bytes = Vec::new();
        loop {
            cutoff.check_exchange()?;
            match client
                .execute(HostOperation::Output {
                    run: intent.run,
                    stream,
                    offset: bytes.len(),
                    max_bytes: GUARDIAN_OUTPUT_CHUNK_LIMIT,
                })
                .await?
            {
                HostReply::Output { output, .. } => {
                    if output.truncated || matches!(output.state, OutputState::Failed(_)) {
                        return Err(io::Error::other("Git output is truncated or unavailable"));
                    }
                    bytes.extend(output.bytes);
                    cutoff.check_exchange()?;
                    if output.state == OutputState::Eof && bytes.len() == output.retained {
                        break;
                    }
                }
                other => {
                    return Err(io::Error::other(format!(
                        "Git output unavailable: {other:?}"
                    )));
                }
            }
            if Instant::now() >= deadline {
                return Err(io::Error::other("Git output EOF is unverified"));
            }
            tokio::time::sleep(Duration::from_millis(/*millis*/ 25)).await;
        }
        *slot = String::from_utf8_lossy(&bytes).into_owned();
    }
    let [stdout, stderr] = streams;
    Ok(GitOutput::new(
        outcome,
        stdout,
        stderr,
        started.elapsed().as_millis() as u64,
    ))
}

/// Refuses an expired admission even if the inner future would be immediately ready.
fn check(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Git completion deadline expired",
        ))
    } else {
        Ok(())
    }
}
