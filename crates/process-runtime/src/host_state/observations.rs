use ora_process_protocol::{
    CleanupState, DirectProcessState, LaunchFact, RunId, RunSnapshot, ScopeId, ScopeState,
    decode_guardian_payload, encode_guardian_frame,
};
use rusqlite::{Connection, OptionalExtension};

use super::{HostState, ProcessStateError};

pub(super) const RUN_SCHEMA: &str = "CREATE TABLE run_observations (
    run TEXT PRIMARY KEY NOT NULL REFERENCES run_intents(run), snapshot BLOB NOT NULL
) STRICT";
pub(super) const SCOPE_SCHEMA: &str = "CREATE TABLE scope_observations (
    scope TEXT PRIMARY KEY NOT NULL REFERENCES scope_intents(scope), snapshot BLOB NOT NULL
) STRICT";

impl HostState {
    /// A sealed, authoritatively closed Scope needs no repeated coordination once every accepted
    /// Run has its own terminal cleanup fact. This does not retire identity, history or output.
    pub(crate) fn scope_settled(&self, scope: ScopeId) -> Result<bool, ProcessStateError> {
        if !self.scope_close_requested(scope)?
            || !matches!(self.observed_scope(scope)?, Some(ScopeState::Closed(_)))
        {
            return Ok(false);
        }
        let mut statement = self.connection.prepare(
            "SELECT o.snapshot FROM run_intents i LEFT JOIN run_observations o ON o.run=i.run WHERE i.scope=?1",
        )?;
        let mut rows = statement.query([scope.to_string()])?;
        while let Some(row) = rows.next()? {
            let Some(bytes) = row.get::<_, Option<Vec<u8>>>(/*idx*/ 0)? else {
                return Ok(false);
            };
            let snapshot: RunSnapshot = decode_guardian_payload(&bytes)?;
            if !matches!(snapshot.cleanup, CleanupState::Complete(_))
                || !matches!(
                    snapshot.direct,
                    DirectProcessState::Exited(_) | DirectProcessState::NotStarted
                )
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Returns historical evidence only; recovery never upgrades it into a current live handle.
    pub(crate) fn observed_run(
        &self,
        run: RunId,
    ) -> Result<Option<RunSnapshot>, ProcessStateError> {
        let bytes: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT snapshot FROM run_observations WHERE run=?1",
                [run.to_string()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?;
        bytes
            .map(|bytes| decode_guardian_payload(&bytes).map_err(Into::into))
            .transpose()
    }

    /// Exposes the last durable Scope observation, separately from requested closure.
    pub(crate) fn observed_scope(
        &self,
        scope: ScopeId,
    ) -> Result<Option<ScopeState>, ProcessStateError> {
        let bytes: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT snapshot FROM scope_observations WHERE scope=?1",
                [scope.to_string()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?;
        bytes
            .map(|bytes| decode_guardian_payload(&bytes).map_err(Into::into))
            .transpose()
    }

    /// Commits a changed fact before host queries can expose it; unchanged polling does not rewrite it.
    pub(crate) fn observe_run(&mut self, snapshot: &RunSnapshot) -> Result<(), ProcessStateError> {
        let previous = self.observed_run(snapshot.id)?;
        let confirmed_launch = previous.as_ref().is_some_and(|previous| {
            previous.launch == LaunchFact::Started
                || matches!(
                    previous.direct,
                    DirectProcessState::Running | DirectProcessState::Exited(_)
                )
        });
        // Once launch is confirmed, a later cancellation/UnknownRun cannot erase that fact,
        // including after a host restart. The prior observation remains the authority on error.
        if confirmed_launch
            && (matches!(snapshot.launch, LaunchFact::NotStarted(_))
                || snapshot.direct == DirectProcessState::NotStarted)
        {
            return Err(ProcessStateError::ContradictoryRunObservation(snapshot.id));
        }
        let mut snapshot = snapshot.clone();
        // A temporary loss of launch visibility must not reopen the non-launch branch later.
        if confirmed_launch {
            snapshot.launch = LaunchFact::Started;
        }
        if previous.as_ref() != Some(&snapshot) {
            let bytes = encode_guardian_frame(&snapshot)?;
            self.connection.execute("INSERT INTO run_observations VALUES (?1, ?2) ON CONFLICT(run) DO UPDATE SET snapshot=excluded.snapshot", rusqlite::params![snapshot.id.to_string(), &bytes[4..]])?;
        }
        self.layout.sync()?;
        Ok(())
    }

    /// Retains closure evidence even when the original guardian is later unavailable.
    pub(crate) fn observe_scope(
        &mut self,
        scope: ScopeId,
        state: ScopeState,
    ) -> Result<(), ProcessStateError> {
        if self.observed_scope(scope)? != Some(state) {
            let bytes = encode_guardian_frame(&state)?;
            self.connection.execute("INSERT INTO scope_observations VALUES (?1, ?2) ON CONFLICT(scope) DO UPDATE SET snapshot=excluded.snapshot", rusqlite::params![scope.to_string(), &bytes[4..]])?;
        }
        self.layout.sync()?;
        Ok(())
    }
}

/// Checks indexes, typed facts and references read-only before any authority advancement.
pub(super) fn inspect(connection: &Connection) -> Result<(), ProcessStateError> {
    let mut statement = connection.prepare("SELECT run, snapshot FROM run_observations")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let run: RunId = row.get::<_, String>(/*idx*/ 0)?.parse()?;
        let snapshot: RunSnapshot = decode_guardian_payload(&row.get::<_, Vec<u8>>(/*idx*/ 1)?)?;
        if run != snapshot.id {
            return Err(ProcessStateError::Rejected(
                "Run observation identity mismatch",
            ));
        }
    }
    let mut statement = connection.prepare("SELECT scope, snapshot FROM scope_observations")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let _: ScopeId = row.get::<_, String>(/*idx*/ 0)?.parse()?;
        let _: ScopeState = decode_guardian_payload(&row.get::<_, Vec<u8>>(/*idx*/ 1)?)?;
    }
    let orphaned: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM run_observations WHERE run NOT IN (SELECT run FROM run_intents)) OR EXISTS(SELECT 1 FROM scope_observations WHERE scope NOT IN (SELECT scope FROM scope_intents))", [], |row| row.get(/*idx*/ 0))?;
    if orphaned {
        return Err(ProcessStateError::Rejected("orphaned host observation"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_process_protocol::{
        CleanupEvidence, DescendantPolicy, ExitOutcome, GuardianHostDisconnect, HostRunIntent,
        LaunchFact, RunSpec,
    };
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;

    /// Scope closure alone cannot stop coordination of a Run whose own cleanup is still unknown.
    #[test]
    fn settled_scope_requires_every_accepted_run_and_retains_its_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
        let directory = tempfile::Builder::new()
            .prefix("hs")
            .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(home)?;
        let mut state = HostState::create(&directory.path().join("s"))?;
        let scope = ScopeId::new();
        state.record_scope_intent(scope)?;
        let intent = HostRunIntent {
            scope,
            run: RunId::new(),
            spec: RunSpec::new(
                "/usr/bin/true",
                directory.path(),
                DescendantPolicy::WaitForAll,
            ),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        };
        state.record_run_intent(intent.clone())?;
        state.request_scope_close(scope)?;
        state.observe_scope(
            scope,
            ScopeState::Closed(CleanupEvidence::BestEffortComplete),
        )?;
        assert!(!state.scope_settled(scope)?);
        let mut snapshot = RunSnapshot {
            id: intent.run,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Code(0)),
            cleanup: CleanupState::Pending,
        };
        state.observe_run(&snapshot)?;
        assert!(!state.scope_settled(scope)?);
        snapshot.cleanup = CleanupState::Complete(CleanupEvidence::BestEffortComplete);
        state.observe_run(&snapshot)?;
        assert!(state.scope_settled(scope)?);
        assert_eq!(state.run_intent(intent.run)?, Some(intent.clone()));
        assert_eq!(state.observed_run(intent.run)?, Some(snapshot));
        assert_eq!(state.run_intents()?, vec![intent]);
        Ok(())
    }
}

#[cfg(test)]
mod non_launch_tests {
    use super::*;
    use ora_process_protocol::{
        CleanupEvidence, DescendantPolicy, ExitOutcome, GuardianHostDisconnect, HostRunIntent,
        RunSpec,
    };
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    /// Recovery must not let an authoritative cancellation overwrite an earlier confirmed launch.
    #[test]
    fn confirmed_launch_rejects_not_started_before_and_after_host_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
        let directory = tempfile::Builder::new()
            .prefix("ns")
            .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(home)?;
        for (index, direct) in [
            DirectProcessState::Running,
            DirectProcessState::Exited(ExitOutcome::Code(0)),
        ]
        .into_iter()
        .enumerate()
        {
            let root = directory.path().join(index.to_string());
            let mut state = HostState::create(&root)?;
            let scope = ScopeId::new();
            state.record_scope_intent(scope)?;
            let intent = HostRunIntent {
                scope,
                run: RunId::new(),
                spec: RunSpec::new(
                    "/usr/bin/true",
                    directory.path(),
                    DescendantPolicy::WaitForAll,
                ),
                host_disconnect: GuardianHostDisconnect::KeepRunning,
            };
            state.record_run_intent(intent.clone())?;
            let original = RunSnapshot {
                id: intent.run,
                launch: LaunchFact::Started,
                direct,
                cleanup: CleanupState::Pending,
            };
            state.observe_run(&original)?;
            let contradiction = RunSnapshot {
                id: intent.run,
                launch: LaunchFact::NotStarted("cancelled before guardian acceptance".into()),
                direct: DirectProcessState::NotStarted,
                cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
            };
            assert!(matches!(
                state.observe_run(&contradiction),
                Err(ProcessStateError::ContradictoryRunObservation(run)) if run == intent.run
            ));
            assert_eq!(state.observed_run(intent.run)?, Some(original.clone()));
            drop(state);
            // Parallel fork/exec fixtures may briefly retain the old flock after this drop.
            // Match the other host-recovery fixtures: retry only contention, with a fixed bound.
            let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
            let mut recovered = loop {
                match HostState::recover(&root) {
                    Err(ProcessStateError::Io(error))
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
                    }
                    result => break result?,
                }
            };
            assert!(matches!(
                recovered.observe_run(&contradiction),
                Err(ProcessStateError::ContradictoryRunObservation(run)) if run == intent.run
            ));
            assert_eq!(recovered.observed_run(intent.run)?, Some(original));
            recovered.observe_run(&RunSnapshot {
                id: intent.run,
                launch: LaunchFact::Unknown("temporarily unavailable".into()),
                direct: DirectProcessState::Unknown,
                cleanup: CleanupState::Pending,
            })?;
            assert_eq!(
                recovered
                    .observed_run(intent.run)?
                    .ok_or("missing observation")?
                    .launch,
                LaunchFact::Started
            );
            assert!(recovered.observe_run(&contradiction).is_err());
        }
        Ok(())
    }
}
