//! Git Server access as the Node identity, from private transport repositories (Node delivery
//! push ADR D1, Node clone ADR D2, D6, D7).
//!
//! Every command that presents the git-access certificate runs with `--git-dir` pointing at a
//! bare transport repository only the Node configures. The checkout is never the repository of
//! such a command: its configuration, hooks and refs are the Agent's, and Git would honor
//! `credential.helper`, `core.askPass`, `http.<url>.*` or `url.<base>.insteadOf` found there. A
//! transport repository may borrow a checkout's object store through `objects/info/alternates`
//! as plain data, with `core.alternateRefsCommand` set so the checkout's refs are never read.
//!
//! The certificate, key, CA and execution header reach Git only as `-c` arguments of the one
//! command; Git's diagnostics stay out of logs and results, which carry bounded codes only.

mod outcome;
mod transport;

pub(crate) use outcome::{GitServerFailure, PushReport, classify, push_report};
pub(crate) use transport::{TransportRepository, purge_transports, remove_transport};

use crate::GitAccessConfig;
use gitlancer::{GitCommand, GitEnv, GitExecError, GitIntent, GitOutput, GitRunner};
use ora_node_protocol::{CommitId, ExecutionId, GitRepositoryUrl};
use std::time::{Duration, Instant};

/// Captured output stays far below the guardian capture limit; Git prints refs and short lines.
const OUTPUT_LIMIT: usize = 256 * 1024;

/// How often and how patiently a retryable Git Server failure is retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetryPolicy {
    /// Pauses before the second and third attempt; three attempts in all.
    pub(crate) delays: [Duration; 2],
}

impl RetryPolicy {
    /// Three attempts, 5 and 30 seconds apart (Node delivery push ADR D4).
    pub(crate) const DEFAULT: Self = Self {
        delays: [Duration::from_secs(5), Duration::from_secs(30)],
    };
}

/// How long Git Server commands may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeLimit {
    /// Each command gets the runner's network budget, `network_timeout_ms` (clone, `ora-git`).
    Network,
    /// Each attempt gets this long, whatever the network budget (delivery pushes, Node delivery
    /// push ADR D4).
    PerAttempt(Duration),
    /// Every attempt and every pause between them end by this instant; what is left of it when
    /// an attempt starts is that attempt's limit (restore, Node delivery push ADR D5).
    Until(Instant),
}

/// The old value a push declares for its destination ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Lease {
    /// No declaration: create-only refs rely on the Git Server refusing an existing ref.
    Unchecked,
    /// The ref must not exist yet.
    Absent,
    /// The ref must point at this commit.
    At(CommitId),
}

/// One ref update a push asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefUpdate<'a> {
    pub(crate) commit: &'a CommitId,
    pub(crate) destination: &'a str,
    pub(crate) lease: Lease,
}

/// Runs Node-identity Git against the Git Server with the deployment's git-access identity.
#[derive(Clone, Debug)]
pub(crate) struct GitServerAccess {
    material: GitAccessConfig,
    /// `PATH` for Git's helpers; the deployment's clone search path.
    search_path: String,
    retry: RetryPolicy,
    limit: TimeLimit,
}

impl GitServerAccess {
    /// Access whose commands run within the runner's network budget.
    pub(crate) fn new(material: GitAccessConfig, search_path: String, retry: RetryPolicy) -> Self {
        Self {
            material,
            search_path,
            retry,
            limit: TimeLimit::Network,
        }
    }

    /// The same access with its commands bounded by `limit` instead.
    pub(crate) fn limited(&self, limit: TimeLimit) -> Self {
        Self {
            limit,
            ..self.clone()
        }
    }

    /// Fetches `refspecs` from `url` into the transport repository, retrying what is retryable.
    pub(crate) fn fetch<R: GitRunner>(
        &self,
        runner: &R,
        transport: &TransportRepository,
        url: &GitRepositoryUrl,
        execution: &ExecutionId,
        refspecs: &[String],
        stopping: &dyn Fn() -> bool,
    ) -> Result<(), GitServerFailure> {
        let mut args = vec![
            "-c".to_owned(),
            "fetch.writeFetchHead=false".to_owned(),
            "fetch".to_owned(),
            "--no-tags".to_owned(),
            "--no-write-fetch-head".to_owned(),
            "--no-recurse-submodules".to_owned(),
            "--".to_owned(),
            url.as_str().to_owned(),
        ];
        args.extend(refspecs.iter().cloned());
        self.retrying(stopping, || {
            self.run(runner, transport, execution, &args)
                .map(drop)
                .map_err(|error| classify(&error))
        })
    }

    /// Lists the Git Server refs matching `patterns` as `(ref, commit)` pairs.
    pub(crate) fn remote_refs<R: GitRunner>(
        &self,
        runner: &R,
        transport: &TransportRepository,
        url: &GitRepositoryUrl,
        execution: &ExecutionId,
        patterns: &[String],
        stopping: &dyn Fn() -> bool,
    ) -> Result<Vec<(String, CommitId)>, GitServerFailure> {
        let mut args = vec![
            "ls-remote".to_owned(),
            "--refs".to_owned(),
            "--".to_owned(),
            url.as_str().to_owned(),
        ];
        args.extend(patterns.iter().cloned());
        let output = self.retrying(stopping, || {
            self.run(runner, transport, execution, &args)
                .map_err(|error| classify(&error))
        })?;
        Ok(output
            .stdout
            .lines()
            .filter_map(|line| {
                let (commit, name) = line.split_once('\t')?;
                Some((name.to_owned(), CommitId::new(commit)))
            })
            .collect())
    }

    /// Pushes one commit to one ref with `--porcelain`; the report tells an update from an
    /// up-to-date ref, a stale lease and a declined update.
    pub(crate) fn push<R: GitRunner>(
        &self,
        runner: &R,
        transport: &TransportRepository,
        url: &GitRepositoryUrl,
        execution: &ExecutionId,
        update: &RefUpdate<'_>,
        stopping: &dyn Fn() -> bool,
    ) -> Result<PushReport, GitServerFailure> {
        let mut args = vec![
            "push".to_owned(),
            "--porcelain".to_owned(),
            "--no-verify".to_owned(),
            "--no-recurse-submodules".to_owned(),
        ];
        match &update.lease {
            Lease::Unchecked => {}
            Lease::Absent => args.push(format!("--force-with-lease={}:", update.destination)),
            Lease::At(commit) => args.push(format!(
                "--force-with-lease={}:{}",
                update.destination,
                commit.as_str()
            )),
        }
        args.extend([
            "--".to_owned(),
            url.as_str().to_owned(),
            format!("{}:{}", update.commit.as_str(), update.destination),
        ]);
        self.retrying(stopping, || {
            match self.run(runner, transport, execution, &args) {
                Ok(output) => {
                    Ok(push_report(&output.stdout, update.destination)
                        .unwrap_or(PushReport::Updated))
                }
                // A refused ref is reported on stdout with a non-zero exit; anything else is a
                // transport failure classified from stderr.
                Err(error) => match &error {
                    GitExecError::NonZeroExit { stdout, .. } => {
                        push_report(stdout, update.destination).ok_or_else(|| classify(&error))
                    }
                    _ => Err(classify(&error)),
                },
            }
        })
    }

    /// Runs `attempt` up to three times while it fails with a retryable failure.
    fn retrying<T>(
        &self,
        stopping: &dyn Fn() -> bool,
        mut attempt: impl FnMut() -> Result<T, GitServerFailure>,
    ) -> Result<T, GitServerFailure> {
        let mut delays = self.retry.delays.iter();
        loop {
            if stopping() {
                return Err(GitServerFailure::Stopped);
            }
            match attempt() {
                Err(GitServerFailure::Unavailable) => {
                    let Some(delay) = delays.next() else {
                        return Err(GitServerFailure::Unavailable);
                    };
                    // An attempt that could only start after the deadline is not worth waiting for.
                    if let TimeLimit::Until(deadline) = self.limit
                        && Instant::now() + *delay >= deadline
                    {
                        return Err(GitServerFailure::Unavailable);
                    }
                    ora_logging::ora_info!(
                        delay_ms = delay.as_millis() as u64,
                        "Git Server unavailable; retrying"
                    );
                    if !pause(*delay, stopping) {
                        return Err(GitServerFailure::Stopped);
                    }
                }
                other => return other,
            }
        }
    }

    /// Runs one Node-identity command in the transport repository with the certificate.
    fn run<R: GitRunner>(
        &self,
        runner: &R,
        transport: &TransportRepository,
        execution: &ExecutionId,
        args: &[String],
    ) -> Result<GitOutput, GitExecError> {
        let (Some(cert), Some(key), Some(ca), Some(git_dir)) = (
            self.material.client_cert.to_str(),
            self.material.client_key.to_str(),
            self.material.ca_cert.to_str(),
            transport.path().to_str(),
        ) else {
            return Err(GitExecError::OutputReadFailed {
                stream: "Git Server access",
                source: std::io::Error::other("non-UTF-8 access material path"),
            });
        };
        let mut full = vec![format!("--git-dir={git_dir}")];
        for config in [
            format!("safe.directory={git_dir}"),
            format!("http.sslCert={cert}"),
            format!("http.sslKey={key}"),
            format!("http.sslCAInfo={ca}"),
            "http.sslVerify=true".to_owned(),
            format!("http.extraHeader=Ora-Execution-Id: {}", execution.as_str()),
            "credential.helper=".to_owned(),
            "core.askPass=".to_owned(),
            "http.followRedirects=false".to_owned(),
            "http.lowSpeedLimit=1024".to_owned(),
            "http.lowSpeedTime=60".to_owned(),
            "protocol.allow=never".to_owned(),
            "protocol.https.allow=always".to_owned(),
            "core.hooksPath=/dev/null".to_owned(),
            "core.alternateRefsCommand=true".to_owned(),
            "gc.auto=0".to_owned(),
            "maintenance.auto=false".to_owned(),
            "submodule.recurse=false".to_owned(),
        ] {
            full.extend(["-c".to_owned(), config]);
        }
        full.extend(args.iter().cloned());
        let env = GitEnv::default()
            .with_variable("PATH", &self.search_path)
            .with_variable("HOME", git_dir)
            .with_variable("GIT_CONFIG_NOSYSTEM", "1")
            .with_variable("GIT_CONFIG_GLOBAL", "/dev/null")
            .with_variable("GIT_TERMINAL_PROMPT", "0")
            .with_variable("GIT_ASKPASS", "")
            .with_variable("SSH_ASKPASS", "")
            .with_variable("GIT_LFS_SKIP_SMUDGE", "1");
        let command = GitCommand::new(
            transport.path().to_path_buf(),
            full,
            env,
            GitIntent::Network,
        );
        let command = match self.limit {
            TimeLimit::Network => command,
            TimeLimit::PerAttempt(limit) => command.with_time_limit(limit),
            TimeLimit::Until(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(GitExecError::OutputReadFailed {
                        stream: "Git Server access",
                        source: std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "Git Server deadline passed",
                        ),
                    });
                }
                command.with_time_limit(left)
            }
        };
        // Runners echo the arguments in their errors; these carry the access material's paths
        // and the execution header, so a failure keeps only Git's own output.
        let output = match runner.run_bounded(&command, OUTPUT_LIMIT, OUTPUT_LIMIT) {
            Ok(output) => output,
            Err(GitExecError::NonZeroExit {
                code,
                stdout,
                stderr,
                ..
            }) => GitOutput::new(code, stdout, stderr, /*duration_ms*/ 0),
            Err(GitExecError::SpawnFailed { source, .. }) => {
                return Err(GitExecError::OutputReadFailed {
                    stream: "Git Server access",
                    source,
                });
            }
            Err(error) => return Err(error),
        };
        if output.code == Some(0) {
            return Ok(output);
        }
        Err(GitExecError::NonZeroExit {
            code: output.code,
            args: Vec::new(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Sleeps for `delay` in short steps; returns `false` when the Node started stopping meanwhile.
fn pause(delay: Duration, stopping: &dyn Fn() -> bool) -> bool {
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < delay {
        if stopping() {
            return false;
        }
        let next = step.min(delay - waited);
        std::thread::sleep(next);
        waited += next;
    }
    !stopping()
}

#[cfg(test)]
mod tests;
