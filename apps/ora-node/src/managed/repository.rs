use super::*;
use ora_node_db::CloneExecution;

/// Absence of any durable Run proves no dispatch; an existing uncertain Run must never be replaced.
pub(crate) enum CloneRecovery {
    Absent,
    Exited(i32),
    Unknown,
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
    ) -> std::io::Result<Option<i32>> {
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
        let result = self.runtime.block_on(transport::execute(
            &client,
            &attempt.intent,
            &self.config,
            &self.shutdown,
        ));
        if let Ok(output) = &result
            && let Some(code) = output.code
        {
            self.journal
                .record_outcome(attempt.intent.run, code)
                .map_err(std::io::Error::other)?;
        }
        self.runtime.block_on(transport::close(
            &client,
            attempt.intent.scope,
            Duration::from_millis(self.config.cleanup_timeout_ms),
        ))?;
        self.journal
            .cleaned(attempt.intent.run)
            .map_err(std::io::Error::other)?;
        result.map(|output| output.code)
    }

    /// Recovers the existing Run only; absence of a Run is not permission to redispatch an intent.
    pub(crate) fn recover_clone(&self, record: &CloneExecution) -> std::io::Result<CloneRecovery> {
        let attempts = self
            .journal
            .attempts(&record.command.execution_id)
            .map_err(std::io::Error::other)?;
        let [attempt] = attempts.as_slice() else {
            return Ok(if attempts.is_empty() {
                CloneRecovery::Absent
            } else {
                CloneRecovery::Unknown
            });
        };
        let client = ProcessHost::new(attempt.host_directory.clone(), attempt.expected_uid);
        self.runtime.block_on(transport::close(
            &client,
            attempt.intent.scope,
            Duration::from_millis(self.config.cleanup_timeout_ms),
        ))?;
        let code = match self
            .runtime
            .block_on(client.execute(HostOperation::QueryRun {
                run: attempt.intent.run,
            }))? {
            HostReply::Run(view) => {
                view.last_observed
                    .and_then(|snapshot| match (snapshot.direct, snapshot.cleanup) {
                        (
                            DirectProcessState::Exited(ExitOutcome::Code(code)),
                            CleanupState::Complete(_),
                        ) => Some(code),
                        _ => None,
                    })
            }
            _ => None,
        };
        if let Some(code) = code {
            self.journal
                .record_outcome(attempt.intent.run, code)
                .map_err(std::io::Error::other)?;
        }
        self.journal
            .cleaned(attempt.intent.run)
            .map_err(std::io::Error::other)?;
        Ok(code.map_or(CloneRecovery::Unknown, CloneRecovery::Exited))
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
