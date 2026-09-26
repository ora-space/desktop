use super::*;
use ora_node_db::CloneExecution;

/// How a clone attempt ended, read from the host only after its Scope was observed Closed.
///
/// The live and recovery paths share this classification so the same host observation always
/// yields the same business outcome, whichever process happens to read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttemptSettlement {
    /// Git exited with its own verdict.
    Exited(i32),
    /// A signal ended Git before it reached a verdict (Node stop, command deadline or an external
    /// kill) and cleanup completed, so no write can follow and success is impossible.
    Terminated(i32),
    /// Exit or cleanup facts are missing; nothing may be concluded about the directory.
    Unverified,
}

/// The host side of clone execution: dispatching, settling and inspecting Runs.
///
/// It holds no Node database connection and writes nothing under the Node home, so it can run
/// away from the thread that owns SQLite. Every Run it touches was recorded before it was handed
/// over, which lets a restart settle the same Run without anything this value remembered.
pub(crate) struct CloneHost {
    config: ProcessConfig,
    shutdown: Shutdown,
    owner: RunLifetime,
    runtime: tokio::runtime::Runtime,
}

impl CloneHost {
    /// Each instance owns its runtime, so one can move to another thread while the runner keeps its own.
    pub(super) fn new(
        config: ProcessConfig,
        shutdown: Shutdown,
        owner: RunLifetime,
    ) -> std::io::Result<Self> {
        Ok(Self {
            config,
            shutdown,
            owner,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        })
    }

    /// Starts a recorded attempt and settles it once Git ends or its wait deadline expires.
    pub(crate) fn dispatch(&self, attempt: &ProcessAttempt) -> std::io::Result<AttemptSettlement> {
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
        self.settle(attempt)
    }

    /// Closes the attempt's Scope, then classifies the host's view of its Run.
    ///
    /// Closing first is the fence: no directory fact is read while the old Git could still write.
    pub(crate) fn settle(&self, attempt: &ProcessAttempt) -> std::io::Result<AttemptSettlement> {
        let client = ProcessHost::new(attempt.host_directory.clone(), attempt.expected_uid);
        self.runtime.block_on(transport::close(
            &client,
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
        Ok(
            match snapshot.map(|snapshot| (snapshot.direct, snapshot.cleanup)) {
                Some((
                    DirectProcessState::Exited(ExitOutcome::Code(code)),
                    CleanupState::Complete(_),
                )) => AttemptSettlement::Exited(code),
                Some((
                    DirectProcessState::Exited(ExitOutcome::Signal(signal)),
                    CleanupState::Complete(_),
                )) => AttemptSettlement::Terminated(signal),
                _ => AttemptSettlement::Unverified,
            },
        )
    }

    /// Inspects local facts under the same non-secret deployment environment and cleanup policy.
    pub(crate) fn inspect(&self, command: &GitCommand) -> std::io::Result<GitOutput> {
        let client = ProcessHost::new(self.config.host_directory.clone(), self.config.expected_uid);
        let intent = HostRunIntent {
            scope: ScopeId::new(),
            run: RunId::new(),
            spec: self.spec(
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
    fn spec(&self, command: &GitCommand, output: OutputPolicy) -> RunSpec {
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

impl<W: WriteGuard> ManagedGitRunner<W> {
    /// Exposes deployment paths for clone's stricter non-overlapping root checks.
    pub(crate) fn process_config(&self) -> &ProcessConfig {
        &self.config
    }

    /// The host side this runner uses when a caller drives a clone on its own thread.
    pub(crate) fn clone_host(&self) -> &CloneHost {
        &self.clone_host
    }

    /// A separate host side with the same deployment and stop signal, for another thread to own.
    pub(crate) fn detached_clone_host(&self) -> std::io::Result<CloneHost> {
        CloneHost::new(self.config.clone(), self.shutdown.clone(), self.owner)
    }

    /// Establishes managed responsibility while the directory-created phase still proves no dispatch.
    pub(crate) fn prepare_clone(&self, record: &CloneExecution) -> Result<(), ora_node_db::Error> {
        self.journal.manage_clone(record)
    }

    /// Lists the Runs already recorded for the execution; recovery settles these, never new ones.
    pub(crate) fn clone_attempts(
        &self,
        record: &CloneExecution,
    ) -> std::io::Result<Vec<ProcessAttempt>> {
        self.journal
            .attempts(&record.command.execution_id)
            .map_err(std::io::Error::other)
    }

    /// Persists the clone's original intent; only a recorded attempt may be dispatched.
    pub(crate) fn record_clone_attempt(
        &self,
        record: &CloneExecution,
        command: &GitCommand,
    ) -> std::io::Result<ProcessAttempt> {
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
                spec: self.clone_host.spec(command, OutputPolicy::Discard),
                host_disconnect: GuardianHostDisconnect::KeepRunning,
            },
        };
        self.journal
            .record(&attempt)
            .map_err(std::io::Error::other)?;
        Ok(attempt)
    }

    /// Records what the host observed after the Scope closed, then retires the attempt's cleanup.
    ///
    /// The ending is persisted before cleanup is marked, so the database can later prove which
    /// business result the single Run supports.
    pub(crate) fn persist_settlement(
        &self,
        attempt: &ProcessAttempt,
        settlement: AttemptSettlement,
    ) -> std::io::Result<()> {
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
            .map_err(std::io::Error::other)
    }
}
