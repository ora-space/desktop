use super::*;
use ora_node_db::CloneExecution;

/// Absence of any durable Run proves no dispatch; an existing uncertain Run must never be replaced.
pub(crate) enum CloneRecovery {
    Absent,
    Settled(AttemptSettlement),
}

/// How a clone attempt ended, read from the host only after its Scope was observed Closed.
///
/// The live and recovery paths share this classification so the same host observation always
/// yields the same business outcome, whichever process happens to read it.
pub(crate) enum AttemptSettlement {
    /// Git exited with its own verdict.
    Exited(i32),
    /// A signal ended Git before it reached a verdict (Node stop, command deadline or an external
    /// kill) and cleanup completed, so no write can follow and success is impossible.
    Terminated(i32),
    /// Exit or cleanup facts are missing; nothing may be concluded about the directory.
    Unverified,
}

impl<W: WriteGuard> ManagedGitRunner<W> {
    /// Exposes deployment paths for clone's stricter non-overlapping root checks.
    pub(crate) fn process_config(&self) -> &ProcessConfig {
        &self.config
    }

    /// Establishes managed responsibility while the directory-created phase still proves no dispatch.
    pub(crate) fn prepare_clone(&self, record: &CloneExecution) -> Result<(), ora_node_db::Error> {
        self.journal.manage_clone(record)
    }

    /// Runs one clone with discarded output, preserving its original intent before host dispatch.
    pub(crate) fn execute_clone(
        &self,
        record: &CloneExecution,
        command: &GitCommand,
    ) -> std::io::Result<AttemptSettlement> {
        if self.shutdown.requested() {
            return Err(std::io::Error::other("Node is stopping"));
        }
        let attempt = ProcessAttempt {
            execution: record.command.execution_id.clone(),
            host_directory: self.config.host_directory.clone(),
            expected_uid: self.config.expected_uid,
            intent: HostRunIntent {
                scope: ScopeId::new(),
                run: RunId::new(),
                spec: self.clone_spec(command, OutputPolicy::Discard),
                host_disconnect: GuardianHostDisconnect::KeepRunning,
            },
        };
        self.journal
            .record(&attempt)
            .map_err(std::io::Error::other)?;
        let client = ProcessHost::new(attempt.host_directory.clone(), attempt.expected_uid);
        // An expired stop grace or command deadline ends this wait without a verdict; the Run's
        // real ending is only knowable after its Scope closes, so settlement reads it from the host
        // instead of treating the interrupted wait as missing evidence.
        let _ = self.runtime.block_on(transport::execute(
            &client,
            &attempt.intent,
            &self.config,
            &self.shutdown,
        ));
        self.settle(&client, &attempt)
    }

    /// Recovers the existing Run only; absence of a Run is not permission to redispatch an intent.
    pub(crate) fn recover_clone(&self, record: &CloneExecution) -> std::io::Result<CloneRecovery> {
        let attempts = self
            .journal
            .attempts(&record.command.execution_id)
            .map_err(std::io::Error::other)?;
        match attempts.as_slice() {
            [] => Ok(CloneRecovery::Absent),
            [attempt] => {
                let client = ProcessHost::new(attempt.host_directory.clone(), attempt.expected_uid);
                self.settle(&client, attempt).map(CloneRecovery::Settled)
            }
            // Each execution dispatches at most one Run; more cannot be attributed safely.
            [_, _, ..] => Ok(CloneRecovery::Settled(AttemptSettlement::Unverified)),
        }
    }

    /// Closes the attempt's Scope, then classifies the host's view of its Run.
    ///
    /// Closing first is the fence: no directory fact is read while the old Git could still write.
    fn settle(
        &self,
        client: &ProcessHost,
        attempt: &ProcessAttempt,
    ) -> std::io::Result<AttemptSettlement> {
        self.runtime.block_on(transport::close(
            client,
            attempt.intent.scope,
            Duration::from_millis(self.config.cleanup_timeout_ms),
        ))?;
        let snapshot = match self
            .runtime
            .block_on(client.execute(HostOperation::QueryRun {
                run: attempt.intent.run,
            }))? {
            HostReply::Run(view) => view.last_observed,
            _ => None,
        };
        let settlement = match snapshot.map(|snapshot| (snapshot.direct, snapshot.cleanup)) {
            Some((
                DirectProcessState::Exited(ExitOutcome::Code(code)),
                CleanupState::Complete(_),
            )) => AttemptSettlement::Exited(code),
            Some((
                DirectProcessState::Exited(ExitOutcome::Signal(signal)),
                CleanupState::Complete(_),
            )) => AttemptSettlement::Terminated(signal),
            _ => AttemptSettlement::Unverified,
        };
        // The ending is persisted before cleanup is marked, so the database can later prove which
        // business result the single Run supports.
        match settlement {
            AttemptSettlement::Exited(code) => self
                .journal
                .record_outcome(attempt.intent.run, code)
                .map_err(std::io::Error::other)?,
            AttemptSettlement::Terminated(signal) => self
                .journal
                .record_termination(attempt.intent.run, signal)
                .map_err(std::io::Error::other)?,
            AttemptSettlement::Unverified => {}
        }
        self.journal
            .cleaned(attempt.intent.run)
            .map_err(std::io::Error::other)?;
        Ok(settlement)
    }

    /// Inspects local facts under the same non-secret deployment environment and cleanup policy.
    pub(crate) fn inspect_clone(&self, command: &GitCommand) -> std::io::Result<GitOutput> {
        let client = ProcessHost::new(self.config.host_directory.clone(), self.config.expected_uid);
        let intent = HostRunIntent {
            scope: ScopeId::new(),
            run: RunId::new(),
            spec: self.clone_spec(
                command,
                OutputPolicy::Capture {
                    stdout_limit: 8192,
                    stderr_limit: 0,
                },
            ),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        };
        let result = self.runtime.block_on(transport::execute(
            &client,
            &intent,
            &self.config,
            &self.shutdown,
        ));
        self.runtime.block_on(transport::close(
            &client,
            intent.scope,
            Duration::from_millis(self.config.cleanup_timeout_ms),
        ))?;
        result
    }

    /// Intentionally does not copy the Worktree deployment environment, which may contain secrets.
    fn clone_spec(&self, command: &GitCommand, output: OutputPolicy) -> RunSpec {
        let mut spec = RunSpec::new(
            self.config.git_program.as_os_str(),
            &command.cwd,
            DescendantPolicy::Cleanup {
                grace: Duration::ZERO,
            },
        );
        spec.args = command.args.iter().map(OsString::from).collect();
        spec.env = command
            .env
            .variables
            .iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        spec.env.insert("LANG".into(), "C".into());
        spec.env.insert("GIT_PAGER".into(), "cat".into());
        spec.output = output;
        spec.lifetime = self.owner;
        spec
    }
}
